# R1 — Attach the Cross-Encoder Reranker to Production Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the existing local cross-encoder (`FastEmbedReranker`, jina-reranker-v1-turbo-en) into production `search_notes` as a best-effort enhancement layered on the unchanged hybrid floor — reranking a 100-candidate pool to the top-20, with hard fallbacks (timeout, error, panic, missing model) to today's hybrid order.

**Architecture:** Extract the search logic into plain, dependency-injected helpers in `src/commands/notes.rs` (`cap_text`, `rerank_top_k`, `search_reranked`) so it is unit/integration-testable without constructing a full `AppState`; the `search_notes` Tauri command becomes a thin adapter. `AppState` gains an `Option<Arc<dyn Reranker>>` constructed at startup with the embedder's degrade-don't-crash pattern. One defensive one-line hardening lands in the `raki-retrieval` `rerank` wrapper.

**Tech Stack:** Rust, Tauri v2 command layer (`raki` app crate), `raki-retrieval` (`hybrid_candidates`, `rerank`), `raki-ai` (`FastEmbedReranker`, `FakeReranker`, `FakeEmbeddingProvider`), `raki-storage` (in-memory SQLite/FTS5/sqlite-vec for tests), `raki-domain` (`Reranker`, `body_to_text`, `Note`), `tokio::time::timeout`.

**Spec:** `docs/superpowers/specs/2026-06-08-r1-reranker-attach-design.md` (D1–D9). Governing ADRs: ADR-0006, ADR-0007. New: ADR-0008.

---

## Verified facts (read before starting)

- **`search_notes` today** (`src/commands/notes.rs:66-86`): calls `raki_retrieval::hybrid_search(state.keyword.as_ref(), state.vectors.as_ref(), state.embedder.as_ref(), &query, 20)`, then for each id `NoteId::parse(&id)?` + `state.notes.get(&note_id).await?` → `NoteDto::from(note)`. Returns `Result<Vec<NoteDto>, AppError>`. `AppError: From<DomainError>` exists (the `?` conversions rely on it).
- **`AppState`** (`src/state.rs:12-30`): fields `notes, keyword, vectors, embedder, clock, index, gate, settings, provider, model, k, budget_tokens`. Imports `raki_domain::{Clock, EgressSettings, EmbeddingProvider, KeywordIndex, NoteRepository, VectorIndex}`. **No `reranker` field.** Constructed only in `src/lib.rs` via `app.manage(AppState { notes, keyword, vectors, embedder, clock, index, gate, settings, provider, model, k: 10, budget_tokens: 2000, ... })` (~line 113).
- **Embedder startup pattern** (`src/lib.rs:76-82`): `let embedder: Arc<dyn EmbeddingProvider> = match FastEmbedProvider::try_new() { Ok(p) => Arc::new(p), Err(e) => { eprintln!("fastembed unavailable ({e}); using fake embeddings this session"); Arc::new(FakeEmbeddingProvider::new(384)) } };`. `lib.rs` imports `use raki_ai::{FakeEmbeddingProvider, FastEmbedProvider, GatedLlmProvider, MessagesProvider};`.
- **`FastEmbedReranker`** (`crates/raki-ai/src/rerank.rs`): `try_new() -> Result<Self, DomainError>`; model jina-reranker-v1-turbo-en; `rerank(query, documents: &[String])` runs the forward pass inside `tokio::task::spawn_blocking` (line 50) and maps a panicked closure's `JoinError` → `DomainError::Provider` (line 57). Exported from `raki_ai` as `FastEmbedReranker`.
- **`raki-retrieval` orchestration** (all `-> Result<Vec<String>, DomainError>`, ids best-first):
  - `hybrid_candidates(keyword: &dyn KeywordIndex, vectors: &dyn VectorIndex, embedder: &dyn EmbeddingProvider, query, pool)` — the recall union.
  - `rerank(reranker: &dyn Reranker, query, candidates: &[(String, String)] /* (id, text) */, k)` — body in `crates/raki-retrieval/src/rerank.rs`; final line is `Ok(scored.iter().take(k).map(|s| candidates[s.index].0.clone()).collect())` (the index access to harden).
- **`Reranker` trait** (`raki-domain`): `fn locality(&self) -> Locality; fn model_id(&self) -> String; async fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<RerankScore>, DomainError>` (`#[async_trait]`). `RerankScore { index: usize, score: f32 }`.
- **`FakeReranker`** (`raki_ai::FakeReranker`): unit struct; scores by query/document token overlap (deterministic). **`FakeEmbeddingProvider::new(dim)`**; `embed(&[String]) -> Result<Vec<Vec<f32>>, DomainError>`.
- **`raki-domain`**: `body_to_text(&str) -> String` and `text_to_body(&str) -> String` (ProseMirror JSON ↔ flat text); `Note::new(title: String, body: String, now_ms: i64) -> Note` with fields `id, title, body, created_at, updated_at, deleted_at, version`; `NoteId::parse(&str) -> Result<NoteId, DomainError>`.
- **In-memory index for tests** (the `run_benchmark` pattern): `raki_storage::{Database::open_in_memory()?, SqliteNoteRepository::new(db.clone()), SqliteKeywordIndex::new(db.clone()), SqliteVectorIndex::new(db.clone())}`; `repo.upsert(&note).await?` indexes relational + FTS5; `vectors.upsert(&id_string, &embedding).await?`. The `raki` app crate depends on `raki-storage` and `raki-ai`, so app tests can use all of these.
- **CI excludes the app crate**: the strict gate is `cargo clippy --workspace --exclude raki --all-targets -- -D warnings`. Per-task verification of app changes therefore uses `cargo test -p raki` (compiles test targets, so helpers are exercised and not dead-code).

---

## File Structure

```
crates/raki-retrieval/src/rerank.rs   MODIFY  harden candidates[s.index] → candidates.get(s.index); + OOB unit test
src/commands/notes.rs                 MODIFY  + cap_text, rerank_top_k, search_reranked helpers; consts; rewrite search_notes; tests
src/commands/notes.rs (Cargo dev-dep) MODIFY  add async-trait dev-dependency (test-only Reranker doubles)
src/state.rs                          MODIFY  + reranker: Option<Arc<dyn Reranker>>; import Reranker
src/lib.rs                            MODIFY  construct reranker (degrade-on-error); pass to AppState; import FastEmbedReranker, Reranker
docs/adr/0008-reranker-attached-attach-to-validate.md  CREATE
docs/ROADMAP.md                       MODIFY  R1 → ✅
docs/eval/reranker-deletion-criteria.md  MODIFY  status line
```

**Rollback:** Task 1 (raki-retrieval) is independent. Tasks 2–4 add unused helpers (exercised only by their tests). Task 5 is the single behavior-changing commit (it both adds the `AppState` field and routes `search_notes` through the helper) — reverting Task 5 alone returns production to today's hybrid path while leaving the tested helpers harmlessly in place.

---

## Task 1: Harden the `raki-retrieval` rerank wrapper against out-of-range indices (review #6)

**Files:** Modify `crates/raki-retrieval/src/rerank.rs`.

- [ ] **Step 1: Write the failing OOB test**

Append to (or create) the `#[cfg(test)] mod tests` at the bottom of `crates/raki-retrieval/src/rerank.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use raki_domain::{Locality, RerankScore, Reranker};

    /// Returns one in-range score and one OUT-OF-RANGE index, to prove the wrapper skips
    /// the bad index instead of panicking on `candidates[s.index]`.
    struct OobReranker;

    #[async_trait]
    impl Reranker for OobReranker {
        fn locality(&self) -> Locality {
            Locality::Local
        }
        fn model_id(&self) -> String {
            "oob".to_string()
        }
        async fn rerank(
            &self,
            _query: &str,
            _documents: &[String],
        ) -> Result<Vec<RerankScore>, DomainError> {
            Ok(vec![
                RerankScore { index: 0, score: 0.9 },
                RerankScore { index: 99, score: 0.8 }, // out of range for 1 candidate
            ])
        }
    }

    #[tokio::test]
    async fn rerank_skips_out_of_range_index_without_panicking() {
        let candidates = vec![("id0".to_string(), "doc zero".to_string())];
        let ids = rerank(&OobReranker, "q", &candidates, 10).await.unwrap();
        assert_eq!(ids, vec!["id0".to_string()], "OOB index skipped, in-range kept");
    }
}
```

- [ ] **Step 2: Run to verify it fails (panic)**

Run: `cd src-tauri && cargo test -p raki-retrieval rerank_skips_out_of_range`
Expected: FAIL — panic `index out of bounds: the len is 1 but the index is 99`.

- [ ] **Step 3: Harden the index access**

In `crates/raki-retrieval/src/rerank.rs`, replace the final returned expression of `pub async fn rerank`:

```rust
    Ok(scored
        .iter()
        .take(k)
        .map(|s| candidates[s.index].0.clone())
        .collect())
```

with (filter first, then take, so up to `k` *valid* ids survive):

```rust
    Ok(scored
        .iter()
        .filter_map(|s| candidates.get(s.index).map(|(id, _)| id.clone()))
        .take(k)
        .collect())
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test -p raki-retrieval`
Expected: PASS — new OOB test green; all existing rerank/fusion/metrics tests still green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-retrieval/src/rerank.rs
git commit -m "raki-retrieval: harden rerank wrapper against out-of-range score index"
```

---

## Task 2: `cap_text` size-bound helper (review #3)

**Files:** Modify `src/commands/notes.rs`.

- [ ] **Step 1: Write the failing tests**

Add a test module at the bottom of `src/commands/notes.rs` (or extend it if Task 3/4 added one — keep one `mod tests`):

```rust
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
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p raki cap_text`
Expected: FAIL — `cap_text` not found.

- [ ] **Step 3: Implement `cap_text`**

Near the top of `src/commands/notes.rs` (after the `use` lines), add:

```rust
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
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test -p raki cap_text`
Expected: PASS — all three `cap_text` tests green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/notes.rs
git commit -m "raki: add cap_text helper (bounds rerank candidate size)"
```

---

## Task 3: `rerank_top_k` helper — timeout + error fallbacks (reviews #1, #2)

**Files:** Modify `src/commands/notes.rs`; add `async-trait` dev-dependency to the `raki` app crate.

- [ ] **Step 1: Add the `async-trait` dev-dependency (for test-only Reranker doubles)**

Run: `cd src-tauri && cargo add async-trait --dev -p raki`
(The app crate has no test harness yet; the doubles below impl the `#[async_trait]` `Reranker` trait.)

- [ ] **Step 2: Write the failing tests**

Add to the `mod tests` in `src/commands/notes.rs`:

```rust
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
```

- [ ] **Step 3: Run to verify it fails**

Run: `cd src-tauri && cargo test -p raki rerank_top_k`
Expected: FAIL — `rerank_top_k` not found.

- [ ] **Step 4: Implement `rerank_top_k`**

In `src/commands/notes.rs` (after `cap_text`), add:

```rust
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
```

- [ ] **Step 5: Run to verify it passes**

Run: `cd src-tauri && cargo test -p raki rerank_top_k`
Expected: PASS — Some / None-on-error / None-on-timeout all green (the timeout test completes in well under a second).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/notes.rs src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "raki: add rerank_top_k helper (timeout + error fallback to hybrid)"
```

---

## Task 4: `search_reranked` core search (D3 flow; reviews #1, #3)

**Files:** Modify `src/commands/notes.rs`.

- [ ] **Step 1: Write the failing integration tests**

Add to the `mod tests` in `src/commands/notes.rs`:

```rust
    use raki_ai::FakeEmbeddingProvider;
    use raki_domain::{text_to_body, EmbeddingProvider, Note, NoteRepository, VectorIndex};
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
        let (repo, keyword, vectors, embedder) =
            index_with(&[("Apples", "granny smith apples"), ("Oceans", "deep blue water")]).await;
        let out = search_reranked(&repo, &keyword, &vectors, embedder.as_embedder(), None, "apples")
            .await
            .unwrap();
        assert!(!out.is_empty(), "hybrid recall returns the apples note");
        assert!(out.iter().any(|n| n.title == "Apples"));
        assert!(out.len() <= K);
    }

    #[tokio::test]
    async fn search_reranked_some_reaches_rerank_and_maps_back_to_notes() {
        let (repo, keyword, vectors, embedder) =
            index_with(&[("Apples", "granny smith apples"), ("Oceans", "deep blue water")]).await;
        let out = search_reranked(
            &repo, &keyword, &vectors, embedder.as_embedder(), Some(&FakeReranker), "apples",
        )
        .await
        .unwrap();
        assert!(out.iter().any(|n| n.title == "Apples"), "rerank path returns valid, mapped notes");
    }

    #[tokio::test]
    async fn search_reranked_handles_oversized_body_without_panicking() {
        let big = "word ".repeat(2000); // ~10 KB plain text → capped to MAX_RERANK_DOC_BYTES
        let (repo, keyword, vectors, embedder) = index_with(&[("Big", &big)]).await;
        let out = search_reranked(
            &repo, &keyword, &vectors, embedder.as_embedder(), Some(&FakeReranker), "word",
        )
        .await
        .unwrap();
        assert!(out.iter().any(|n| n.title == "Big"), "oversized note returned, no panic");
    }
```

> Note: the tests call `embedder.as_embedder()` to pass `&dyn EmbeddingProvider`. If `FakeEmbeddingProvider` does not expose such a helper, pass `&embedder as &dyn EmbeddingProvider` instead — both compile; pick the one that matches the codebase (the `&embedder as &dyn _` form needs no new method).

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p raki search_reranked`
Expected: FAIL — `search_reranked` / `K` not found.

- [ ] **Step 3: Implement `search_reranked` + the consts**

In `src/commands/notes.rs` (after `rerank_top_k`), add the consts and the function:

```rust
use std::collections::HashMap;

use raki_domain::{body_to_text, EmbeddingProvider, KeywordIndex, NoteRepository, VectorIndex};

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
    let pool = raki_retrieval::hybrid_candidates(keyword, vectors, embedder, query, POOL).await?;

    // 2. Hydrate pool ids → Notes in pool order; skip any deleted mid-flight.
    let mut hydrated: Vec<Note> = Vec::with_capacity(pool.len());
    for id in &pool {
        let nid = NoteId::parse(id)?;
        if let Some(note) = notes.get(&nid).await? {
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
        candidates.iter().take(K).map(|(id, _)| id.clone()).collect()
    };
    let ranked_ids: Vec<String> = match reranker {
        Some(r) => rerank_top_k(r, query, &candidates, K, RERANK_TIMEOUT)
            .await
            .unwrap_or_else(hybrid_top_k),
        None => hybrid_top_k(),
    };

    // 5. Map ranked ids → Notes, consuming the already-hydrated set (no second fetch).
    let mut by_id: HashMap<String, Note> =
        hydrated.into_iter().map(|n| (n.id.to_string(), n)).collect();
    Ok(ranked_ids
        .iter()
        .filter_map(|id| by_id.remove(id))
        .collect())
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test -p raki search_reranked`
Expected: PASS — none-hybrid, some-rerank, and oversized-body tests green.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/notes.rs
git commit -m "raki: add search_reranked (pool → cap → rerank/fallback → notes)"
```

---

## Task 5: Wire the reranker into `AppState` and `search_notes` (D1, D2, D3)

**Files:** Modify `src/state.rs`, `src/lib.rs`, `src/commands/notes.rs`.

- [ ] **Step 1: Add the `reranker` field to `AppState`**

In `src/state.rs`, extend the `raki_domain` import and add the field after `embedder`:

```rust
use raki_domain::{
    Clock, EgressSettings, EmbeddingProvider, KeywordIndex, NoteRepository, Reranker, VectorIndex,
};
```

```rust
    pub embedder: Arc<dyn EmbeddingProvider>,
    /// Optional local cross-encoder reranker (attach-to-validate, ADR-0008). `None` degrades
    /// search to hybrid-only; best-effort, never required for search to work.
    pub reranker: Option<Arc<dyn Reranker>>,
```

- [ ] **Step 2: Construct the reranker at startup and pass it to `AppState`**

In `src/lib.rs`, extend the `raki_ai` import and add a `raki_domain::Reranker` import:

```rust
use raki_ai::{FakeEmbeddingProvider, FastEmbedProvider, FastEmbedReranker, GatedLlmProvider, MessagesProvider};
use raki_domain::Reranker;
```

Immediately after the embedder block (`lib.rs:82`), add:

```rust
            let reranker: Option<Arc<dyn Reranker>> = match FastEmbedReranker::try_new() {
                Ok(r) => Some(Arc::new(r)),
                Err(e) => {
                    eprintln!("reranker unavailable ({e}); search runs without reranking this session");
                    None
                }
            };
```

Then add `reranker,` to the `app.manage(AppState { ... })` initializer, right after `embedder,`:

```rust
            app.manage(AppState {
                notes,
                keyword,
                vectors,
                embedder,
                reranker,
                clock,
                index,
                gate,
                settings,
                provider,
                model,
                k: 10,
                budget_tokens: 2000,
```

(If `raki_domain` is already imported in `lib.rs` under one `use`, merge `Reranker` into it rather than adding a second line.)

- [ ] **Step 3: Rewrite `search_notes` as a thin adapter over `search_reranked`**

In `src/commands/notes.rs`, replace the body of `search_notes` (`notes.rs:66-86`):

```rust
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
```

(`state.reranker.as_deref()` turns `&Option<Arc<dyn Reranker>>` into `Option<&dyn Reranker>`. The `DomainError` from `search_reranked` converts to `AppError` via the existing `?` impl.)

- [ ] **Step 4: Verify the whole app builds and all tests pass**

Run: `cd src-tauri && cargo test -p raki`
Expected: PASS — all helper + integration tests green; the app compiles with the reranker wired (the `reranker` field is now used, so no dead-code warning).

- [ ] **Step 5: Strict gate on the changed crates**

Run: `cd src-tauri && cargo clippy -p raki -p raki-retrieval --all-targets -- -D warnings && cargo fmt --check`
Expected: PASS — no warnings; formatting clean. (The app crate is normally `--exclude`d from CI; we lint it explicitly here because we changed it.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/state.rs src-tauri/src/lib.rs src-tauri/src/commands/notes.rs
git commit -m "raki: attach local reranker to search_notes (best-effort, hybrid fallback)"
```

---

## Task 6: Documentation — ADR-0008, ROADMAP, kill-switch status (D7)

**Files:** Create `docs/adr/0008-reranker-attached-attach-to-validate.md`; modify `docs/ROADMAP.md`, `docs/eval/reranker-deletion-criteria.md`.

- [ ] **Step 1: Write ADR-0008**

Create `docs/adr/0008-reranker-attached-attach-to-validate.md`:

```markdown
# ADR-0008: Cross-encoder reranker attached to production as attach-to-validate

- **Status:** Accepted
- **Date:** 2026-06-08
- **Deciders:** Jayden
- **Tags:** retrieval, ai, reranker, process

## Context

R0 stood up the SciFact benchmark tier (ADR-0007). On it, the local cross-encoder
(jina-reranker-v1-turbo-en) beats hybrid by **+0.0313 nDCG@10** (also +0.0285 Recall@10,
+0.0319 MAP) — a consistent, multi-metric lift. But SciFact is domain-shifted: the **binding**
keep-or-delete verdict, per `docs/eval/reranker-deletion-criteria.md`, requires **+0.03 nDCG on
≥100 real-labeled personal-notes queries**, which do not exist yet (they arrive via the P1
dogfooding/real-data track).

So we have directional, reproducible evidence the reranker helps, but not the faithful verdict.

## Decision

Attach the reranker to production `search_notes` **as attach-to-validate**:

1. Wire `FastEmbedReranker` into `AppState` as `Option<Arc<dyn Reranker>>`, constructed at startup
   with the embedder's degrade-don't-crash pattern.
2. `search_notes` reranks the 100-candidate hybrid recall union to the top-20, with hard fallbacks
   (missing model, error, 5 s timeout, panic via spawn_blocking's JoinError, out-of-range index) to
   the unchanged hybrid order. Search never breaks; reranking only improves or no-ops.
3. The reranker is local (`Locality::Local`) — no egress, no privacy cost.
4. The kill-switch stays **armed**: the binding verdict is deferred to real-notes ground truth (P1).
   No production telemetry is added; validation is via the eval harness, not metrics.

## Consequences

**Positive**
- Users get the directionally-better ranking now, and dogfooding the reranked experience is how the
  real-notes intuition (and eventually the labeled queries) accrue.
- The hybrid floor is untouched and remains the guaranteed fallback.

**Negative / costs**
- Ships a lever not yet validated on Raki's own distribution — mitigated by the armed kill-switch and
  trivial rollback (the reranker is an `Option`; revert the wiring to return to hybrid-only).
- Adds per-search work (100-note hydration + cross-encoder pass), bounded by `POOL` and a per-candidate
  size cap; latency is watched in dogfooding, with `POOL` as the dial.

## Alternatives considered

- **Don't attach; build the real-notes tier first** — most faithful, but blocks a visible improvement
  on private-data effort (that is the P1 track, pursued in parallel).
- **Attach unconditionally (trust SciFact)** — rejected: SciFact is domain-shifted; the kill-switch
  binds to real data.

## References

- ADR-0006 (staged recall → rerank → generate), ADR-0007 (measurement-gated; benchmark-first).
- `docs/eval/reranker-deletion-criteria.md` (the binding kill-switch).
- `docs/eval/scifact-baseline.md` (the +0.0313 directional basis).
- `docs/superpowers/specs/2026-06-08-r1-reranker-attach-design.md`.
```

- [ ] **Step 2: Mark R1 done in the ROADMAP**

In `docs/ROADMAP.md`, change the R1 heading and status. Replace `### ⬜ R1 — Reranker decision *(precision lever)* — unblocked by R0` with `### ✅ R1 — Reranker decision *(precision lever)*` and append after its existing **Note:** line:

```markdown
**Status:** ✅ Done. Reranker attached to production `search_notes` as **attach-to-validate**
(ADR-0008): local cross-encoder reranks the 100-pool to top-20, best-effort with hybrid fallback
(timeout/error/missing/panic/OOB all degrade to hybrid). Binding keep/delete verdict pending
real-notes ground truth (P1); kill-switch armed. Spec/plan:
`docs/superpowers/specs/2026-06-08-r1-reranker-attach-design.md`.
```

- [ ] **Step 3: Update the kill-switch status line**

In `docs/eval/reranker-deletion-criteria.md`, add a status line near the top (below the title):

```markdown
> **Status (2026-06-08):** Reranker is **attached-pending-validation** in production (ADR-0008) on
> directional SciFact evidence (+0.0313 nDCG@10). This kill-switch remains the **binding** test:
> the reranker stays only if it beats hybrid by +0.03 nDCG on ≥100 real-notes queries, else it is
> removed.
```

- [ ] **Step 4: Commit**

```bash
git add docs/adr/0008-reranker-attached-attach-to-validate.md docs/ROADMAP.md docs/eval/reranker-deletion-criteria.md
git commit -m "docs: ADR-0008 reranker attach-to-validate; R1 done; kill-switch status"
```

---

## Task 7: Full verification, manual smoke + latency, DoD

**Files:** none (verification + optional latency note).

- [ ] **Step 1: Deterministic suite green (CI path + the two changed crates)**

Run: `cd src-tauri && cargo test --workspace --exclude raki && cargo test -p raki && cargo clippy --workspace --exclude raki --all-targets -- -D warnings && cargo clippy -p raki --all-targets -- -D warnings && cargo fmt --check`
Expected: all pass — new raki-retrieval OOB test, the app's helper + integration tests, and the unchanged 30-note `eval_gate` all green; no warnings.

- [ ] **Step 2: Reranker quality sentinel still passes (model + network)**

Run: `cd src-tauri && cargo test -p raki-eval --test benchmark_gate -- --ignored`
Expected: PASS — `run_benchmark`/`rerank` unchanged, so the vector floor + reranker plausibility assertions still hold. (This is the reranker's only quality gate; it is `#[ignore]` by design.)

- [ ] **Step 3: Manual app smoke + latency (needs the real model; cannot be claimed without running)**

Run: `cd src-tauri && cargo tauri dev` (or the project's dev launch), then in the running app:
- Create a handful of notes, run a search, and confirm results render (reranked ordering is active when the model loaded).
- Note search responsiveness; if perceptibly laggy, record it — `POOL` is the dial to lower (D8).
- Optionally confirm graceful degradation: with no network on first run the reranker model can't download → the startup log prints "reranker unavailable …" and search still returns hybrid results.

Record the before/after feel (or a rough timing) as the D8 latency check. Do **not** mark this step done without actually running the app (verification-before-completion).

- [ ] **Step 4: DoD against the spec**

D1 (`AppState.reranker: Option`) ✓ T5 · D2 (degrade-don't-crash startup) ✓ T5 · D3 (pool→cap→timeout-rerank→fallback) ✓ T2–T5 · D4 (best-effort: missing/err/timeout/panic/OOB → hybrid) ✓ T1,T3,T4,T5 · D5 (benchmark_gate sentinel, eval tier unchanged) ✓ T7 · D6 (Some/None/erroring/timeout/large-body/OOB tests) ✓ T1,T3,T4 · D7 (ADR-0008/ROADMAP/kill-switch) ✓ T6 · D8 (latency check + fallback logging; no telemetry) ✓ T3,T5,T7 · D9 (bulk hydration deferred) ✓ (not implemented, by decision).

- [ ] **Step 5: (No commit)** — verification only; all code/docs committed in Tasks 1–6.

---

## Self-Review

**Spec coverage:** D1→T5 (field), D2→T5 (startup), D3→T2/T3/T4 (cap/timeout/flow)+T5 (command), D4→T1+T3+T4+T5 (all fallback arms), D5→T7 (gate run; eval untouched), D6→T1 (OOB)+T3 (Some/Err/timeout)+T4 (none/some/large-body), D7→T6, D8→T3 (fallback eprintln)+T5+T7 (latency smoke), D9→deferred by decision (no task, intentional). Every D-item maps to a task or an explicit deferral.

**Placeholder scan:** none — every code step shows complete code; every run step has a command + expected outcome. The one conditional (`as_embedder()` vs `&embedder as &dyn _` in T4 Step 1) gives a concrete, compiles-either-way instruction, not a TODO. The manual smoke (T7 Step 3) is explicitly marked not-claimable-without-running, matching the spec's manual posture.

**Type/consistency:** `cap_text(&str, usize) -> String` (T2) used in `search_reranked` (T4). `rerank_top_k(&dyn Reranker, &str, &[(String,String)], usize, Duration) -> Option<Vec<String>>` (T3) used in `search_reranked` (T4). `search_reranked(&dyn NoteRepository, &dyn KeywordIndex, &dyn VectorIndex, &dyn EmbeddingProvider, Option<&dyn Reranker>, &str) -> Result<Vec<Note>, DomainError>` (T4) used by `search_notes` (T5). Consts `POOL=100, K=20, MAX_RERANK_DOC_BYTES=4096, RERANK_TIMEOUT=5s` defined once in T4. `AppState.reranker: Option<Arc<dyn Reranker>>` (T5 state.rs) ↔ `.as_deref()` → `Option<&dyn Reranker>` (T5 command) matches T4's param. The hardened `raki_retrieval::rerank` (T1) is the one called inside `rerank_top_k` (T3). Retrieval-fn/`Reranker`/`Note` signatures match the verified-facts block.

**Known confirmations (read-and-match at implementation time):** `FakeEmbeddingProvider` `&dyn EmbeddingProvider` coercion form (T4 note: `as_embedder()` or `&embedder as &dyn _`); that `lib.rs` imports `raki_domain` under a mergeable `use` (T5 Step 2 note); that `AppError: From<DomainError>` covers the `search_reranked` `?` in the command (stated in verified facts — the current `search_notes` already relies on it).
