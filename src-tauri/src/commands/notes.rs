//! Thin command adapters: translate + delegate. No business logic, no SQL, no ranking.

use tauri::State;

use raki_domain::{DomainError, Note, NoteId};

use crate::dto::{CreateNoteInput, ExportNotesForEvalResult, NoteDto, UpdateNoteInput};
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

use raki_domain::{
    body_to_text, EmbeddingProvider, KeywordIndex, NoteRepository, Reranker, VectorIndex,
};

use std::collections::HashMap;

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

/// Recall-union depth fed to the reranker — the exact pool `bench` reranked on SciFact.
const POOL: usize = 100;
/// Number of results returned for display.
const K: usize = 20;
/// Per-candidate text cap before reranking (review #3).
const MAX_RERANK_DOC_BYTES: usize = 4096;
/// Hard bound on a single rerank call before falling back to hybrid (review #1).
const RERANK_TIMEOUT: Duration = Duration::from_secs(5);

/// Production search: hybrid recall union → (size-capped) candidates → optional rerank →
/// top-`K` notes. A missing reranker, a rerank error, or a rerank timeout all fall back to the
/// hybrid top-`K` (which is bit-for-bit today's behavior), so search never breaks (D4).
async fn search_reranked(
    notes: &dyn NoteRepository,
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    reranker: Option<&dyn Reranker>,
    query: &str,
) -> Result<Vec<Note>, DomainError> {
    // 1. Recall union (unchanged retrieval fn).
    let pool =
        raki_retrieval::hybrid_candidates(keyword, vectors, embedder, None, query, POOL).await?;

    // 2. Hydrate pool ids → Notes in pool order; skip any deleted mid-flight.
    let mut hydrated: Vec<Note> = Vec::with_capacity(pool.len());
    for nid in &pool {
        if let Some(note) = notes.get(nid).await? {
            hydrated.push(note);
        }
    }

    // 3. Build (id, size-capped text) candidate pairs in the same order — the representation
    //    run_benchmark reranked.
    let candidates: Vec<(String, String)> = hydrated
        .iter()
        .map(|n| {
            let text = format!("{}\n\n{}", n.title, body_to_text(&n.body));
            (n.id.to_string(), cap_text(&text, MAX_RERANK_DOC_BYTES))
        })
        .collect();

    // 4. Decide final id order: rerank if present & it succeeds in time, else hybrid top-K.
    let hybrid_top_k = || -> Vec<String> {
        candidates
            .iter()
            .take(K)
            .map(|(id, _)| id.clone())
            .collect()
    };
    let ranked_ids: Vec<String> = match reranker {
        Some(r) => rerank_top_k(r, query, &candidates, K, RERANK_TIMEOUT)
            .await
            .unwrap_or_else(hybrid_top_k),
        None => hybrid_top_k(),
    };

    // 5. Map ranked ids → Notes, consuming the already-hydrated set (no second fetch).
    let mut by_id: HashMap<String, Note> = hydrated
        .into_iter()
        .map(|n| (n.id.to_string(), n))
        .collect();
    Ok(ranked_ids
        .iter()
        .filter_map(|id| by_id.remove(id))
        .collect())
}

/// Boundary validation shared by create + update (review M1). Returns the trimmed title
/// and normalized ProseMirror-JSON body so callers share a single source of truth for
/// sanitization.
fn validate(title: &str, body: &str) -> Result<(String, String), AppError> {
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
    // Body must be valid ProseMirror doc JSON.
    let normalized = raki_domain::normalize_body(body).map_err(|e| AppError {
        kind: "validation_error".into(),
        message: format!("invalid note body: {e}"),
    })?;
    Ok((t.to_string(), normalized))
}

#[tauri::command]
pub async fn create_note(
    state: State<'_, AppState>,
    input: CreateNoteInput,
) -> Result<NoteDto, AppError> {
    let (title, body) = validate(&input.title, &input.body)?;
    let note = Note::new(title, body, state.clock.now_ms());
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

/// Hybrid recall → optional local rerank → DTOs. Reranking is best-effort (ADR-0008): if the
/// reranker is absent, errors, or times out, results fall back to the hybrid top-K.
#[tauri::command]
pub async fn search_notes(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<NoteDto>, AppError> {
    let notes = search_reranked(
        state.notes.as_ref(),
        state.keyword.as_ref(),
        state.vectors.as_ref(),
        state.embedder.as_ref(),
        state.reranker.as_deref(),
        &query,
    )
    .await?;
    Ok(notes.into_iter().map(NoteDto::from).collect())
}

#[tauri::command]
pub async fn update_note(
    state: State<'_, AppState>,
    input: UpdateNoteInput,
) -> Result<NoteDto, AppError> {
    let (title, body) = validate(&input.title, &input.body)?;
    let nid = NoteId::parse(&input.id)?;
    let existing = state.notes.get(&nid).await?.ok_or_else(|| AppError {
        kind: "not_found".into(),
        message: "note not found".into(),
    })?;
    let edited = existing.edit(title, body, state.clock.now_ms());
    // Atomic guarded update: false ⇒ the row was deleted between read and write — do not resurrect.
    if !state.notes.update(&edited).await? {
        return Err(AppError {
            kind: "not_found".into(),
            message: "note not found".into(),
        });
    }
    state.signal_store.touch(&nid, state.clock.now_ms()).await?;
    state.index.trigger();
    Ok(NoteDto::from(edited))
}

#[tauri::command]
pub async fn delete_note(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    let nid = NoteId::parse(&id)?;
    state.notes.soft_delete(&nid, state.clock.now_ms()).await?;
    Ok(())
}

#[tauri::command]
pub async fn restore_note(state: State<'_, AppState>, id: String) -> Result<NoteDto, AppError> {
    let nid = NoteId::parse(&id)?;
    let mut note = state.notes.get_any(&nid).await?.ok_or_else(|| AppError {
        kind: "not_found".into(),
        message: "note not found".into(),
    })?;
    if note.deleted_at.is_none() {
        return Err(AppError {
            kind: "bad_request".into(),
            message: "note is not deleted".into(),
        });
    }
    note.deleted_at = None;
    note.updated_at = state.clock.now_ms();
    note.version += 1;
    state.notes.upsert(&note).await?;
    state.index.trigger();
    Ok(NoteDto::from(note))
}

#[tauri::command]
pub async fn list_trashed_notes(state: State<'_, AppState>) -> Result<Vec<NoteDto>, AppError> {
    let notes = state.notes.list_trashed().await?;
    Ok(notes.into_iter().map(NoteDto::from).collect())
}

/// Turn a note title into a filesystem-safe slug. Collapses non-alphanumerics to dashes,
/// trims leading/trailing dashes, and lowercases. Empty titles fall back to `untitled`.
fn title_to_slug(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut prev_dash = true; // so leading non-alpha becomes nothing
    for c in title.chars() {
        if c.is_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out
    }
}

/// Export all live notes to `eval-data/real/notes/*.md` so they can be used by the local
/// real-data eval harness (`cargo run -p raki-eval --bin real-eval`). Files are written as
/// Markdown with YAML frontmatter; the eval loader strips frontmatter and uses the H1 as title.
#[tauri::command]
pub async fn export_notes_for_eval(
    state: State<'_, AppState>,
) -> Result<ExportNotesForEvalResult, AppError> {
    // Same directory the `real-eval` binary reads from: project-root/eval-data/real.
    let eval_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("eval-data")
        .join("real");
    let notes_dir = eval_dir.join("notes");
    std::fs::create_dir_all(&notes_dir).map_err(|e| AppError {
        kind: "io_error".into(),
        message: format!("cannot create eval notes dir: {e}"),
    })?;

    let notes = state.notes.list().await?;
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut exported = 0;

    for note in notes {
        let base = title_to_slug(&note.title);
        let count = seen.entry(base.clone()).or_insert(0);
        *count += 1;
        let slug = if *count == 1 {
            base
        } else {
            format!("{base}-{}", count)
        };

        let path = notes_dir.join(format!("{slug}.md"));
        let body_text = raki_domain::body_to_text(&note.body);
        let content = format!(
            "---\ntitle: {}\nid: {}\n---\n\n# {}\n\n{}\n",
            note.title, note.id, note.title, body_text
        );
        std::fs::write(&path, content).map_err(|e| AppError {
            kind: "io_error".into(),
            message: format!("failed to write {path:?}: {e}"),
        })?;
        exported += 1;
    }

    Ok(ExportNotesForEvalResult { exported })
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::text_to_body;

    #[test]
    fn cap_text_passes_short_strings_through() {
        assert_eq!(cap_text("hello", 4096), "hello");
    }

    #[test]
    fn title_to_slug_sanitizes_and_lowercases() {
        assert_eq!(title_to_slug("Espresso Dialing!"), "espresso-dialing");
        assert_eq!(title_to_slug("  Postgres -- Pooling  "), "postgres-pooling");
        assert_eq!(title_to_slug("---"), "untitled");
    }

    #[test]
    fn title_to_slug_collapses_multiple_dashes() {
        assert_eq!(title_to_slug("a--b__c"), "a-b-c");
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
        fn locality(&self) -> Locality {
            Locality::Local
        }
        fn model_id(&self) -> String {
            "err".into()
        }
        async fn rerank(&self, _q: &str, _d: &[String]) -> Result<Vec<RerankScore>, DomainError> {
            Err(DomainError::Provider("boom".into()))
        }
    }

    struct HangReranker;
    #[async_trait]
    impl Reranker for HangReranker {
        fn locality(&self) -> Locality {
            Locality::Local
        }
        fn model_id(&self) -> String {
            "hang".into()
        }
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
        let out = rerank_top_k(
            &FakeReranker,
            "apple",
            &candidates(),
            10,
            Duration::from_secs(5),
        )
        .await;
        let ids = out.expect("FakeReranker succeeds → Some");
        assert_eq!(
            ids.first().map(String::as_str),
            Some("a"),
            "apple doc ranked first"
        );
    }

    #[tokio::test]
    async fn rerank_top_k_returns_none_on_error() {
        let out = rerank_top_k(
            &ErrReranker,
            "apple",
            &candidates(),
            10,
            Duration::from_secs(5),
        )
        .await;
        assert!(
            out.is_none(),
            "rerank error → None (caller uses hybrid order)"
        );
    }

    #[tokio::test]
    async fn rerank_top_k_returns_none_on_timeout() {
        // 1 ms budget against a 60 s reranker → timeout fallback, fast.
        let out = rerank_top_k(
            &HangReranker,
            "apple",
            &candidates(),
            10,
            Duration::from_millis(1),
        )
        .await;
        assert!(
            out.is_none(),
            "rerank timeout → None (caller uses hybrid order)"
        );
    }

    use raki_ai::FakeEmbeddingProvider;
    use raki_domain::{EmbeddingProvider, Note, NoteRepository, VectorIndex};
    use raki_storage::{Database, SqliteKeywordIndex, SqliteNoteRepository, SqliteVectorIndex};

    /// Build an in-memory index over the given (title, plain-body) notes (relational + FTS5 +
    /// vectors), mirroring the run_benchmark construction. Returns the four index handles.
    async fn index_with(
        notes: &[(&str, &str)],
    ) -> (
        SqliteNoteRepository,
        SqliteKeywordIndex,
        SqliteVectorIndex,
        FakeEmbeddingProvider,
    ) {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let keyword = SqliteKeywordIndex::new(db.clone());
        let vectors = SqliteVectorIndex::new(db.clone());
        let embedder = FakeEmbeddingProvider::new(384);
        for (title, body) in notes {
            let note = Note::new((*title).to_string(), text_to_body(body), 1000);
            let id = note.id.to_string();
            repo.upsert(&note).await.unwrap();
            let text = format!("{title}\n\n{body}");
            let emb = embedder.embed(std::slice::from_ref(&text)).await.unwrap();
            vectors.upsert(&id, &emb[0]).await.unwrap();
        }
        (repo, keyword, vectors, embedder)
    }

    #[tokio::test]
    async fn search_reranked_none_returns_hybrid_hits() {
        let (repo, keyword, vectors, embedder) = index_with(&[
            ("Apples", "granny smith apples"),
            ("Oceans", "deep blue water"),
        ])
        .await;
        let out = search_reranked(
            &repo,
            &keyword,
            &vectors,
            &embedder as &dyn EmbeddingProvider,
            None,
            "apples",
        )
        .await
        .unwrap();
        assert!(!out.is_empty(), "hybrid recall returns the apples note");
        assert!(out.iter().any(|n| n.title == "Apples"));
        assert!(out.len() <= K);
    }

    #[tokio::test]
    async fn search_reranked_some_reaches_rerank_and_maps_back_to_notes() {
        let (repo, keyword, vectors, embedder) = index_with(&[
            ("Apples", "granny smith apples"),
            ("Oceans", "deep blue water"),
        ])
        .await;
        let out = search_reranked(
            &repo,
            &keyword,
            &vectors,
            &embedder as &dyn EmbeddingProvider,
            Some(&FakeReranker),
            "apples",
        )
        .await
        .unwrap();
        assert!(
            out.iter().any(|n| n.title == "Apples"),
            "rerank path returns valid, mapped notes"
        );
    }

    #[tokio::test]
    async fn search_reranked_handles_oversized_body_without_panicking() {
        let big = "word ".repeat(2000); // ~10 KB plain text → capped to MAX_RERANK_DOC_BYTES
        let (repo, keyword, vectors, embedder) = index_with(&[("Big", &big)]).await;
        let out = search_reranked(
            &repo,
            &keyword,
            &vectors,
            &embedder as &dyn EmbeddingProvider,
            Some(&FakeReranker),
            "word",
        )
        .await
        .unwrap();
        assert!(
            out.iter().any(|n| n.title == "Big"),
            "oversized note returned, no panic"
        );
    }

    /// Manual latency probe (not a gate): times the real production `search_reranked` over a
    /// realistic vault of short notes at `POOL`, to choose an interactive pool size. The SciFact
    /// gate measured ~24 s/query on 512-token abstracts; real notes are short, so this measures
    /// the number that actually matters. Run: `cargo test -p raki --release rerank_latency_probe
    /// -- --ignored --nocapture`.
    #[tokio::test]
    #[ignore = "loads the real reranker model; manual latency probe, prints timing"]
    async fn rerank_latency_probe() {
        use raki_ai::FastEmbedReranker;
        use std::time::Instant;

        let owned: Vec<(String, String)> = (0..150)
            .map(|i| {
                (
                    format!("Note {i}"),
                    format!("a short personal note about topic {i}: groceries, meeting at 3pm, and a link"),
                )
            })
            .collect();
        let refs: Vec<(&str, &str)> = owned
            .iter()
            .map(|(t, b)| (t.as_str(), b.as_str()))
            .collect();
        let (repo, keyword, vectors, embedder) = index_with(&refs).await;
        let reranker = FastEmbedReranker::try_new().expect("reranker init");

        let t = Instant::now();
        let out = search_reranked(
            &repo,
            &keyword,
            &vectors,
            &embedder as &dyn EmbeddingProvider,
            Some(&reranker),
            "meeting groceries topic 42",
        )
        .await
        .unwrap();
        let elapsed = t.elapsed();
        eprintln!(
            "[latency] search_reranked over {} notes, POOL={POOL}, returned {} in {elapsed:?}",
            owned.len(),
            out.len()
        );
        assert!(!out.is_empty());
    }

    #[test]
    fn validate_accepts_valid_prosemirror_json() {
        let body = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"hi"}]}]}"#;
        let (title, out) = validate("T", body).unwrap();
        assert_eq!(title, "T");
        assert!(out.contains("blockId"));
    }

    #[test]
    fn validate_rejects_invalid_json() {
        let err = validate("T", "not json").unwrap_err();
        assert_eq!(err.kind, "validation_error");
    }

    #[test]
    fn validate_rejects_non_doc_json() {
        let err = validate("T", r#"{"type":"not-doc"}"#).unwrap_err();
        assert_eq!(err.kind, "validation_error");
    }
}
