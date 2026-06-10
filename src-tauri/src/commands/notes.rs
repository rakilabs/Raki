//! Thin command adapters: translate + delegate. No business logic, no SQL, no ranking.

use tauri::State;

use raki_domain::{text_to_body, Note, NoteId};

use crate::dto::{CreateNoteInput, NoteDto, UpdateNoteInput};
use crate::error::AppError;
use crate::state::AppState;

const MAX_TITLE_CHARS: usize = 512;
const MAX_BODY_BYTES: usize = 256 * 1024;

/// Truncate `s` to at most `max_bytes`, backing off to the nearest char boundary so a
/// multi-byte UTF-8 character is never split. Bounds per-search rerank memory; the
/// cross-encoder only consumes ~512 tokens, so nothing it would read is lost.
fn cap_text(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

use std::time::Duration;

use raki_domain::Reranker;

/// Rerank `candidates` to top-`k`, bounded by `timeout`. Returns `Some(ids)` on success, or
/// `None` (the caller falls back to hybrid order) on timeout or any rerank error. The forward
/// pass already runs in `spawn_blocking` inside `FastEmbedReranker`, so this never stalls the
/// runtime; the timeout only bounds a degenerate hung inference. `timeout` is a parameter so
/// tests can exercise the timeout arm at 1 ms instead of waiting `RERANK_TIMEOUT`.
async fn rerank_top_k(
    reranker: &dyn Reranker,
    query: &str,
    candidates: &[(String, String)],
    k: usize,
    timeout: Duration,
) -> Option<Vec<String>> {
    match tokio::time::timeout(
        timeout,
        raki_retrieval::rerank(reranker, query, candidates, k),
    )
    .await
    {
        Ok(Ok(ids)) => Some(ids),
        Ok(Err(e)) => {
            eprintln!("rerank failed ({e}); falling back to hybrid order");
            None
        }
        Err(_elapsed) => {
            eprintln!("rerank timed out after {timeout:?}; falling back to hybrid order");
            None
        }
    }
}

/// Boundary validation shared by create + update (review M1). Returns the trimmed title
/// so callers share a single source of truth for sanitization.
fn validate(title: &str, body: &str) -> Result<String, AppError> {
    let t = title.trim();
    if t.is_empty() {
        return Err(AppError {
            kind: "validation_error".into(),
            message: "title must not be empty".into(),
        });
    }
    if t.chars().count() > MAX_TITLE_CHARS {
        return Err(AppError {
            kind: "validation_error".into(),
            message: "title too long".into(),
        });
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(AppError {
            kind: "validation_error".into(),
            message: "body too long".into(),
        });
    }
    Ok(t.to_string())
}

#[tauri::command]
pub async fn create_note(
    state: State<'_, AppState>,
    input: CreateNoteInput,
) -> Result<NoteDto, AppError> {
    let title = validate(&input.title, &input.body)?;
    let note = Note::new(title, text_to_body(&input.body), state.clock.now_ms());
    state.notes.upsert(&note).await?;
    state.index.trigger(); // embed the new note in the background (single-flight)
    Ok(NoteDto::from(note))
}

#[tauri::command]
pub async fn list_notes(state: State<'_, AppState>) -> Result<Vec<NoteDto>, AppError> {
    let notes = state.notes.list().await?;
    Ok(notes.into_iter().map(NoteDto::from).collect())
}

#[tauri::command]
pub async fn get_note(state: State<'_, AppState>, id: String) -> Result<Option<NoteDto>, AppError> {
    let note_id = NoteId::parse(&id)?;
    Ok(state.notes.get(&note_id).await?.map(NoteDto::from))
}

/// Hybrid search: fuse FTS5 keyword + sqlite-vec vector rankings (RRF), then hydrate
/// the ranked ids to DTOs. (Hydration is one `get` per hit; fine at k = 20.)
#[tauri::command]
pub async fn search_notes(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<NoteDto>, AppError> {
    let ids = raki_retrieval::hybrid_search(
        state.keyword.as_ref(),
        state.vectors.as_ref(),
        state.embedder.as_ref(),
        &query,
        20,
    )
    .await?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let note_id = NoteId::parse(&id)?;
        if let Some(note) = state.notes.get(&note_id).await? {
            out.push(NoteDto::from(note));
        }
    }
    Ok(out)
}

#[tauri::command]
pub async fn update_note(
    state: State<'_, AppState>,
    input: UpdateNoteInput,
) -> Result<NoteDto, AppError> {
    let title = validate(&input.title, &input.body)?;
    let nid = NoteId::parse(&input.id)?;
    let existing = state.notes.get(&nid).await?.ok_or_else(|| AppError {
        kind: "not_found".into(),
        message: "note not found".into(),
    })?;
    let edited = existing.edit(title, text_to_body(&input.body), state.clock.now_ms());
    // Atomic guarded update: false ⇒ the row was deleted between read and write — do not resurrect.
    if !state.notes.update(&edited).await? {
        return Err(AppError {
            kind: "not_found".into(),
            message: "note not found".into(),
        });
    }
    state.index.trigger();
    Ok(NoteDto::from(edited))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_text_passes_short_strings_through() {
        assert_eq!(cap_text("hello", 4096), "hello");
    }

    #[test]
    fn cap_text_truncates_long_ascii_to_limit() {
        let s = "a".repeat(5000);
        let out = cap_text(&s, 4096);
        assert_eq!(out.len(), 4096);
    }

    #[test]
    fn cap_text_never_splits_a_utf8_char() {
        // '€' is 3 bytes; capping at 4 bytes must back off to the 3-byte boundary.
        let s = "€€"; // 6 bytes
        let out = cap_text(s, 4);
        assert_eq!(out, "€");
        assert!(out.len() <= 4);
    }

    use async_trait::async_trait;
    use raki_ai::FakeReranker;
    use raki_domain::{DomainError, Locality, RerankScore, Reranker};
    use std::time::Duration;

    struct ErrReranker;
    #[async_trait]
    impl Reranker for ErrReranker {
        fn locality(&self) -> Locality { Locality::Local }
        fn model_id(&self) -> String { "err".into() }
        async fn rerank(&self, _q: &str, _d: &[String]) -> Result<Vec<RerankScore>, DomainError> {
            Err(DomainError::Provider("boom".into()))
        }
    }

    struct HangReranker;
    #[async_trait]
    impl Reranker for HangReranker {
        fn locality(&self) -> Locality { Locality::Local }
        fn model_id(&self) -> String { "hang".into() }
        async fn rerank(&self, _q: &str, _d: &[String]) -> Result<Vec<RerankScore>, DomainError> {
            tokio::time::sleep(Duration::from_secs(60)).await;
            Ok(Vec::new())
        }
    }

    fn candidates() -> Vec<(String, String)> {
        vec![
            ("a".to_string(), "red apple fruit".to_string()),
            ("b".to_string(), "blue ocean water".to_string()),
        ]
    }

    #[tokio::test]
    async fn rerank_top_k_returns_some_on_success() {
        let out = rerank_top_k(&FakeReranker, "apple", &candidates(), 10, Duration::from_secs(5)).await;
        let ids = out.expect("FakeReranker succeeds → Some");
        assert_eq!(ids.first().map(String::as_str), Some("a"), "apple doc ranked first");
    }

    #[tokio::test]
    async fn rerank_top_k_returns_none_on_error() {
        let out = rerank_top_k(&ErrReranker, "apple", &candidates(), 10, Duration::from_secs(5)).await;
        assert!(out.is_none(), "rerank error → None (caller uses hybrid order)");
    }

    #[tokio::test]
    async fn rerank_top_k_returns_none_on_timeout() {
        // 1 ms budget against a 60 s reranker → timeout fallback, fast.
        let out = rerank_top_k(&HangReranker, "apple", &candidates(), 10, Duration::from_millis(1)).await;
        assert!(out.is_none(), "rerank timeout → None (caller uses hybrid order)");
    }
}
