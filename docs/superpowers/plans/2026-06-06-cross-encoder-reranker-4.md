# Cross-Encoder Reranker (Slice 4) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a local cross-encoder reranker as a measured, regression-gated stage in the eval (`reranked = hybrid + rerank`), eval-substrate only — no production `search_notes` change — with an honest measurement record and a committed deletion criterion.

**Architecture:** A `Reranker` port in `raki-domain` (mirrors `EmbeddingProvider`), two adapters in `raki-ai` (`FastEmbedReranker` over `fastembed::TextRerank` with `JINARerankerV1TurboEn`; `FakeReranker`, an orchestration-only token-overlap stub). `raki-retrieval` splits `hybrid_search` into `hybrid_candidates().truncate(k)` behind a characterization test and adds a pure `rerank` fn. `raki-eval` integrates `reranked` as a first-class `Method`; the gate floors it additively (measure-then-floor) and the deterministic keyword gate is untouched.

**Tech Stack:** Rust, `fastembed` 5.x (reranking ships in the crate we already use — **no new Cargo dependency**, but a new ~37M-param ONNX runtime model), `tokio`, `serde_json`, `async-trait`.

**Spec:** `docs/superpowers/specs/2026-06-06-cross-encoder-reranker-4-design.md` (D1–D7, D-FALSIFY, D-DELETE). This plan implements all of it.

**Verified facts (read before starting):**
- `RerankerModel` variants in `fastembed` 5.x: `BGERerankerBase` (BAAI/bge-reranker-base, ~278M), `BGERerankerV2M3` (multilingual, larger), `JINARerankerV1TurboEn` (jinaai/jina-reranker-v1-turbo-en, English, ~37M — **the D1 pick: smallest/cheapest, corpus is English**), `JINARerankerV2BaseMultiligual` (note the crate's misspelling). API: `TextRerank::try_new(RerankInitOptions::new(model))?` then `model.rerank(query, documents, return_documents: bool, batch_size: Option<usize>) -> Result<Vec<RerankResult>>` where `RerankResult { index: usize, score: f32, document: Option<String> }`.
- `run_eval(embedder, k)` lives in `raki-eval/src/lib.rs:233`; callers are `raki-eval/src/main.rs:47` and `raki-eval/tests/eval_gate.rs` (two tests: `keyword_snapshot_is_deterministic`, `real_model_gate`). Changing its signature forces updating all three together (one compile unit).
- `QueryResult` (`lib.rs:99`) has strict serde (no defaults). Adding `reranked` invalidates the committed `docs/eval/snapshot.json` until regenerated — so the struct change + all callers + regen are **one commit** (Task 6).
- Current eval numbers (from the 3b audit, k=3): vec/hyb nDCG `lexical-cluster` 0.92, `paraphrase-distractor` 0.91, `dense-near-duplicate` 1.00; recall ≈ 1.0 everywhere except `coverage`. These are the only places reranked can move.

---

## File Structure

```
raki-domain/src/ports.rs        MODIFY  + Reranker trait + RerankScore (mirrors EmbeddingProvider)
raki-domain/src/lib.rs          MODIFY  export Reranker, RerankScore
raki-ai/src/rerank.rs           CREATE  FastEmbedReranker (fastembed::TextRerank, JINARerankerV1TurboEn)
raki-ai/src/fake_rerank.rs      CREATE  FakeReranker (token-overlap; orchestration-stub-only)
raki-ai/src/lib.rs              MODIFY  mod + exports
raki-retrieval/src/search.rs    MODIFY  extract hybrid_candidates; characterization test
raki-retrieval/src/rerank.rs    CREATE  pure rerank(reranker, query, candidates, k)
raki-retrieval/src/lib.rs       MODIFY  mod rerank; export hybrid_candidates, rerank
raki-eval/src/lib.rs            MODIFY  Method::Reranked, reranked fields, run_eval(reranker) arg, scoring
raki-eval/src/main.rs           MODIFY  construct reranker; rr column "(= hybrid+rerank)"; delta line; baseline rr
raki-eval/tests/eval_gate.rs    MODIFY  construct rerankers; Reranked in snapshot methods, floors, nDCG loop
docs/eval/snapshot.json         REGEN   additive (new reranked block); fingerprint unchanged
docs/eval/baseline.md           REGEN   + rr column + reranker model id
docs/eval/judge-log.md          MODIFY  author-once reranked−hybrid nDCG delta record (D-FALSIFY)
docs/eval/reranker-deletion-criteria.md  CREATE  the D-DELETE tracked ticket
```

---

## Task 1: `Reranker` port in `raki-domain`

**Files:**
- Modify: `src-tauri/crates/raki-domain/src/ports.rs`
- Modify: `src-tauri/crates/raki-domain/src/lib.rs`

- [ ] **Step 1: Write the failing test**

In `ports.rs`, at the bottom add a `#[cfg(test)]` module with a hand-written stub proving the trait is object-safe and usable:

```rust
#[cfg(test)]
mod reranker_tests {
    use super::*;

    struct StubReranker;
    #[async_trait]
    impl Reranker for StubReranker {
        fn locality(&self) -> Locality { Locality::Local }
        fn model_id(&self) -> String { "stub".to_string() }
        async fn rerank(&self, _query: &str, documents: &[String]) -> Result<Vec<RerankScore>, DomainError> {
            Ok(documents.iter().enumerate().map(|(i, _)| RerankScore { index: i, score: i as f32 }).collect())
        }
    }

    #[tokio::test]
    async fn reranker_is_object_safe_and_scores_each_doc() {
        let r: &dyn Reranker = &StubReranker;
        let out = r.rerank("q", &["a".to_string(), "b".to_string()]).await.unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[1], RerankScore { index: 1, score: 1.0 });
        assert_eq!(r.locality(), Locality::Local);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-domain reranker_is_object_safe`
Expected: FAIL — `cannot find trait Reranker` / `RerankScore`.

- [ ] **Step 3: Add the trait + struct**

In `ports.rs`, after the `EmbeddingProvider` block (around line 44), add:

```rust
/// A cross-encoder relevance score for one candidate document. `index` is the position
/// in the `documents` slice passed to `rerank`; `score` is higher = more relevant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RerankScore {
    pub index: usize,
    pub score: f32,
}

/// A cross-encoder reranker: reads each (query, document) pair jointly and scores relevance.
/// Mirrors `EmbeddingProvider` as a port; adapters live in `raki-ai`.
#[async_trait]
pub trait Reranker: Send + Sync {
    fn locality(&self) -> Locality;
    /// Stable identifier of the reranker model+version (recorded in the eval baseline).
    fn model_id(&self) -> String;
    /// Score every document against the query. Returns one `RerankScore` per input document
    /// (order unspecified — the caller sorts by score). Higher score = more relevant.
    async fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<RerankScore>, DomainError>;
}
```

- [ ] **Step 4: Export from the crate root**

In `raki-domain/src/lib.rs`, find the `pub use` line that re-exports from `ports` (it already exports `EmbeddingProvider`, `Locality`, etc.) and add `Reranker, RerankScore` to it. (If exports are listed individually, add `pub use ports::{Reranker, RerankScore};` alongside the existing `ports::` re-exports.)

- [ ] **Step 5: Run the test to verify it passes**

Run: `cd src-tauri && cargo test -p raki-domain reranker_is_object_safe`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-domain/src/ports.rs src-tauri/crates/raki-domain/src/lib.rs
git commit -m "Add Reranker port + RerankScore to raki-domain"
```

---

## Task 2: `FakeReranker` (orchestration stub) in `raki-ai`

**Files:**
- Create: `src-tauri/crates/raki-ai/src/fake_rerank.rs`
- Modify: `src-tauri/crates/raki-ai/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/crates/raki-ai/src/fake_rerank.rs`:

```rust
//! `FakeReranker`: a deterministic, model-free reranker stub for offline `run_eval` and
//! unit tests.
//!
//! ORCHESTRATION STUB ONLY. It scores by query/document token overlap, which is
//! STRUCTURALLY UNCORRELATED with how a real cross-encoder scores (query, doc) pairs. It
//! exists to prove the plumbing — index→id mapping, truncation, empty pools — runs
//! deterministically without loading a model. It says NOTHING about real reranking quality
//! or real-model failure modes; those are validated only by `FastEmbedReranker`'s
//! `#[ignore]` integration test.

use std::collections::HashSet;

use async_trait::async_trait;

use raki_domain::{DomainError, Locality, RerankScore, Reranker};

pub struct FakeReranker;

/// Lowercase ascii-alphanumeric token set. Pure; shared shape with the harness's intent.
fn tokens(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

#[async_trait]
impl Reranker for FakeReranker {
    fn locality(&self) -> Locality {
        Locality::Local
    }
    fn model_id(&self) -> String {
        "fake-reranker".to_string()
    }
    async fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<RerankScore>, DomainError> {
        let q = tokens(query);
        Ok(documents
            .iter()
            .enumerate()
            .map(|(index, doc)| {
                let overlap = tokens(doc).intersection(&q).count();
                RerankScore { index, score: overlap as f32 }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_reranker_scores_by_token_overlap_deterministically() {
        let docs = vec![
            "nothing relevant here".to_string(),     // 0 overlap with "red apple"
            "a red apple and a green apple".to_string(), // overlaps red, apple
            "red things".to_string(),                // overlaps red
        ];
        let a = FakeReranker.rerank("red apple", &docs).await.unwrap();
        let b = FakeReranker.rerank("red apple", &docs).await.unwrap();
        assert_eq!(a, b, "deterministic");
        assert_eq!(a[0].score, 0.0);
        assert!(a[1].score > a[2].score, "more overlap scores higher");
    }
}
```

- [ ] **Step 2: Wire the module + export**

In `src-tauri/crates/raki-ai/src/lib.rs`, add `mod fake_rerank;` with the other `mod` lines and `pub use fake_rerank::FakeReranker;` with the other `pub use` lines.

- [ ] **Step 3: Run the test to verify it fails then passes**

Run: `cd src-tauri && cargo test -p raki-ai fake_reranker_scores_by_token_overlap`
Expected: PASS (new file + test compile and pass together).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-ai/src/fake_rerank.rs src-tauri/crates/raki-ai/src/lib.rs
git commit -m "Add FakeReranker orchestration stub to raki-ai"
```

---

## Task 3: `FastEmbedReranker` (real model) in `raki-ai`

**Files:**
- Create: `src-tauri/crates/raki-ai/src/rerank.rs`
- Modify: `src-tauri/crates/raki-ai/src/lib.rs`

- [ ] **Step 1: Create the adapter + the `#[ignore]` edge-case test**

Create `src-tauri/crates/raki-ai/src/rerank.rs`:

```rust
//! The fastembed-backed `Reranker`: in-process ONNX cross-encoder, model
//! `jina-reranker-v1-turbo-en` (English, ~37M params — the smallest fastembed reranker;
//! quality differences vs larger rerankers are noise at the eval's scale, so we pick the
//! cheapest). Swap the `RerankerModel` variant to change models. Downloads once, cached.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fastembed::{RerankInitOptions, RerankerModel, TextRerank};

use raki_domain::{DomainError, Locality, RerankScore, Reranker};

/// Stable reranker model identifier (recorded in the eval baseline).
pub const RERANKER_MODEL_ID: &str = "jina-reranker-v1-turbo-en";

pub struct FastEmbedReranker {
    model: Arc<Mutex<TextRerank>>,
}

impl FastEmbedReranker {
    pub fn try_new() -> Result<Self, DomainError> {
        let model = TextRerank::try_new(RerankInitOptions::new(RerankerModel::JINARerankerV1TurboEn))
            .map_err(|e| DomainError::Provider(format!("fastembed reranker init: {e}")))?;
        Ok(Self { model: Arc::new(Mutex::new(model)) })
    }
}

#[async_trait]
impl Reranker for FastEmbedReranker {
    fn locality(&self) -> Locality {
        Locality::Local
    }
    fn model_id(&self) -> String {
        RERANKER_MODEL_ID.to_string()
    }
    async fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<RerankScore>, DomainError> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }
        let model = self.model.clone();
        let q = query.to_string();
        let docs = documents.to_vec();
        let results = tokio::task::spawn_blocking(move || {
            let guard = model.lock().unwrap();
            let refs: Vec<&str> = docs.iter().map(|s| s.as_str()).collect();
            // return_documents = false (we only need index + score); default batch size.
            guard.rerank(q.as_str(), refs, false, None)
        })
        .await
        .map_err(|e| DomainError::Provider(format!("rerank join: {e}")))?
        .map_err(|e| DomainError::Provider(format!("rerank: {e}")))?;
        Ok(results
            .into_iter()
            .map(|r| RerankScore { index: r.index, score: r.score })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "downloads the jina-reranker model on first run; run explicitly with --ignored"]
    async fn fastembed_reranker_orders_relevant_first_and_survives_edges() {
        let r = FastEmbedReranker::try_new().expect("reranker init");
        assert_eq!(r.locality(), Locality::Local);
        assert_eq!(r.model_id(), "jina-reranker-v1-turbo-en");

        // Relevance ordering: the panda doc should outscore the unrelated one.
        let docs = vec![
            "the giant panda is a bear endemic to china".to_string(),
            "mortgage refinance break-even is closing costs over monthly savings".to_string(),
        ];
        let scores = r.rerank("what is a panda?", &docs).await.unwrap();
        assert_eq!(scores.len(), 2);
        let panda = scores.iter().find(|s| s.index == 0).unwrap().score;
        let other = scores.iter().find(|s| s.index == 1).unwrap().score;
        assert!(panda > other, "relevant doc scores higher");

        // Edge cases (where real ONNX rerankers panic): empty pool, empty doc, oversized text.
        assert!(r.rerank("q", &[]).await.unwrap().is_empty());
        let big = "lorem ipsum ".repeat(4000); // far past the model's token window
        let edge = vec!["".to_string(), big];
        let out = r.rerank("anything", &edge).await.unwrap();
        assert_eq!(out.len(), 2, "no panic on empty/oversized docs; one score each");
        assert!(out.iter().all(|s| s.score.is_finite()), "no NaN/inf scores");
    }
}
```

- [ ] **Step 2: Wire the module + export**

In `src-tauri/crates/raki-ai/src/lib.rs`, add `mod rerank;` and `pub use rerank::{FastEmbedReranker, RERANKER_MODEL_ID};`.

- [ ] **Step 3: Verify it compiles (non-ignored tests pass)**

Run: `cd src-tauri && cargo test -p raki-ai`
Expected: PASS (the real-model test is `#[ignore]`d, so this only checks compilation + the fake tests). If `guard.rerank(...)`'s argument types don't match the installed `fastembed` version, adjust the `refs`/return-flag types to match its `rerank` signature (query + documents + `return_documents: bool` + `batch_size: Option<usize>`).

- [ ] **Step 4: Run the real-model edge-case test once**

Run: `cd src-tauri && cargo test -p raki-ai fastembed_reranker_orders_relevant_first -- --ignored`
Expected: PASS (downloads the jina reranker on first run, then orders panda first and survives the edge cases).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-ai/src/rerank.rs src-tauri/crates/raki-ai/src/lib.rs
git commit -m "Add FastEmbedReranker (jina-reranker-v1-turbo-en) to raki-ai"
```

---

## Task 4: Extract `hybrid_candidates` behind a characterization test

**Files:**
- Modify: `src-tauri/crates/raki-retrieval/src/search.rs`
- Modify: `src-tauri/crates/raki-retrieval/src/lib.rs`

- [ ] **Step 1: Add the characterization test (pins current behavior)**

In `search.rs` `#[cfg(test)] mod tests`, add a test that pins `hybrid_search`'s exact output on a richer fixture. Write it now, against the CURRENT (un-refactored) code:

```rust
    #[tokio::test]
    async fn hybrid_search_output_is_characterized() {
        // Vector is authoritative [c, b, e]; keyword [a, b, c, d] backfills only the
        // ids vector missed (a, d), in keyword order, after the vector block.
        let keyword = FakeKeyword(vec!["a", "b", "c", "d"]);
        let vectors = FakeVectors(vec!["c", "b", "e"]);
        let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, "q", 4).await.unwrap();
        assert_eq!(
            ids,
            vec!["c".to_string(), "b".to_string(), "e".to_string(), "a".to_string()],
            "vector order preserved; keyword-only ids backfill in order; truncated to k=4"
        );
    }
```

- [ ] **Step 2: Run it on the current code (capture the baseline)**

Run: `cd src-tauri && cargo test -p raki-retrieval hybrid_search_output_is_characterized`
Expected: PASS — this proves the assertion captures real current behavior (a characterization test passes before AND after the refactor; a post-refactor failure would mean behavior changed).

- [ ] **Step 3: Extract `hybrid_candidates`; reimplement `hybrid_search` on top of it**

In `search.rs`, replace the `hybrid_search` function (lines ~17-33) with:

```rust
/// The recall **union** — vector-primary, keyword-backfilled — UNtruncated. This is the
/// candidate pool the precision stage (rerank) reorders. `pool` is the depth pulled from
/// each retriever; the union is at least `HYBRID_CANDIDATE_POOL` deep so backfill ids exist.
pub async fn hybrid_candidates(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    query: &str,
    pool: usize,
) -> Result<Vec<String>, DomainError> {
    let depth = pool.max(HYBRID_CANDIDATE_POOL);
    let mut out = vector_search(vectors, embedder, query, depth).await?;
    for id in search(keyword, query, depth).await? {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

/// Hybrid retrieval, **vector-primary**: `hybrid_candidates` truncated to `k`. The embedding
/// model is the stronger retriever on clean text, so vector's ranking is authoritative and
/// keyword only *backfills* ids vector did not return — provably never worse than vector
/// alone, while keyword gives cold-start and exact-token coverage. The cross-encoder rerank
/// stage (eval, ADR-0006) reorders `hybrid_candidates` for precision.
pub async fn hybrid_search(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    query: &str,
    k: usize,
) -> Result<Vec<String>, DomainError> {
    let mut out = hybrid_candidates(keyword, vectors, embedder, query, k).await?;
    out.truncate(k);
    Ok(out)
}
```

- [ ] **Step 4: Add a `hybrid_candidates` test**

In the same test module, add:

```rust
    #[tokio::test]
    async fn hybrid_candidates_returns_the_untruncated_union() {
        let keyword = FakeKeyword(vec!["a", "b"]);
        let vectors = FakeVectors(vec!["b", "c"]);
        let ids = hybrid_candidates(&keyword, &vectors, &FakeEmbed, "q", 20).await.unwrap();
        assert_eq!(
            ids,
            vec!["b".to_string(), "c".to_string(), "a".to_string()],
            "full union, vector-first, keyword backfill, no truncation"
        );
    }
```

- [ ] **Step 5: Export `hybrid_candidates`**

In `raki-retrieval/src/lib.rs`, change `pub use search::{hybrid_search, search, vector_search};` to also export `hybrid_candidates`:

```rust
pub use search::{hybrid_candidates, hybrid_search, search, vector_search};
```

- [ ] **Step 6: Run all retrieval tests (characterization still green after refactor)**

Run: `cd src-tauri && cargo test -p raki-retrieval`
Expected: PASS — `hybrid_search_output_is_characterized` still passes (behavior preserved), plus the new `hybrid_candidates` test and the existing hybrid tests.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/crates/raki-retrieval/src/search.rs src-tauri/crates/raki-retrieval/src/lib.rs
git commit -m "Extract hybrid_candidates from hybrid_search (characterization-tested)"
```

---

## Task 5: Pure `rerank` function in `raki-retrieval`

**Files:**
- Create: `src-tauri/crates/raki-retrieval/src/rerank.rs`
- Modify: `src-tauri/crates/raki-retrieval/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/crates/raki-retrieval/src/rerank.rs`:

```rust
//! The precision seam: reorder the recall union by a cross-encoder `Reranker`. Pure —
//! depends only on the port, never on a concrete model.

use raki_domain::{DomainError, Reranker};

/// Reorder `candidates` ((id, text) — the recall union) by reranker score, best-first,
/// and return the top-`k` ids. Equal scores preserve the candidates' incoming order
/// (stable sort), so the recall ranking is the tie-break.
pub async fn rerank(
    reranker: &dyn Reranker,
    query: &str,
    candidates: &[(String, String)],
    k: usize,
) -> Result<Vec<String>, DomainError> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let docs: Vec<String> = candidates.iter().map(|(_, text)| text.clone()).collect();
    let mut scored = reranker.rerank(query, &docs).await?;
    // Stable sort by score descending; NaN treated as lowest (Equal keeps incoming order).
    scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scored
        .iter()
        .take(k)
        .filter_map(|s| candidates.get(s.index).map(|(id, _)| id.clone()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use raki_domain::{Locality, RerankScore};

    /// Scores each doc by its index (higher index = higher score), so it REVERSES the
    /// incoming order — proving rerank actually reorders by score, not position.
    struct ReverseReranker;
    #[async_trait]
    impl Reranker for ReverseReranker {
        fn locality(&self) -> Locality { Locality::Local }
        fn model_id(&self) -> String { "reverse".to_string() }
        async fn rerank(&self, _q: &str, docs: &[String]) -> Result<Vec<RerankScore>, DomainError> {
            Ok(docs.iter().enumerate().map(|(i, _)| RerankScore { index: i, score: i as f32 }).collect())
        }
    }

    fn cands(ids: &[&str]) -> Vec<(String, String)> {
        ids.iter().map(|id| (id.to_string(), format!("text-{id}"))).collect()
    }

    #[tokio::test]
    async fn rerank_reorders_by_score_desc_and_truncates() {
        let out = rerank(&ReverseReranker, "q", &cands(&["a", "b", "c"]), 2).await.unwrap();
        assert_eq!(out, vec!["c".to_string(), "b".to_string()], "highest score first, top-2");
    }

    #[tokio::test]
    async fn rerank_empty_candidates_is_empty() {
        let out = rerank(&ReverseReranker, "q", &[], 3).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn rerank_k_larger_than_len_returns_all_reordered() {
        let out = rerank(&ReverseReranker, "q", &cands(&["a", "b"]), 10).await.unwrap();
        assert_eq!(out, vec!["b".to_string(), "a".to_string()]);
    }
}
```

- [ ] **Step 2: Wire the module + export**

In `raki-retrieval/src/lib.rs`, add `mod rerank;` (with the other `mod` lines) and add `rerank` to the `pub use`:

```rust
pub use rerank::rerank;
```

- [ ] **Step 3: Run the tests**

Run: `cd src-tauri && cargo test -p raki-retrieval rerank`
Expected: PASS (all three rerank tests).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-retrieval/src/rerank.rs src-tauri/crates/raki-retrieval/src/lib.rs
git commit -m "Add pure rerank() reordering fn to raki-retrieval"
```

---

## Task 6: Integrate `reranked` through the eval (atomic: lib + binary + gate callers + regen)

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`
- Modify: `src-tauri/crates/raki-eval/src/main.rs`
- Modify: `src-tauri/crates/raki-eval/tests/eval_gate.rs`
- Regenerate: `docs/eval/snapshot.json`, `docs/eval/baseline.md`

> This is one commit because the `run_eval` signature change + the `QueryResult` schema change + the strict-snapshot regen must move together to stay green.

- [ ] **Step 1: Add `Reranked` to the `Method` enum + `method()` arm**

In `lib.rs`, in `pub enum Method` (line ~111) add `Reranked,` after `Hybrid`. In `impl QueryResult::method` (line ~118) add the arm `Method::Reranked => &self.reranked,`.

- [ ] **Step 2: Add `reranked` fields to the result/report structs**

In `lib.rs`: add `pub reranked: MethodScores,` to `CategoryReport` (after `hybrid`, line ~74); add `pub overall_reranked: MethodScores,` to `Report` (after `overall_hybrid`, line ~82); add `pub reranked: MethodResult,` to `QueryResult` (after `hybrid`, line ~105).

- [ ] **Step 3: Update imports + add the rerank pool constant**

In `lib.rs`, extend the `raki_domain` use (line 48) to include `Reranker`, and the `raki_retrieval` use (lines 49-52) to include `hybrid_candidates` and `rerank`:

```rust
use raki_domain::{DomainError, EmbeddingProvider, Note, NoteId, NoteRepository, Reranker, VectorIndex};
use raki_retrieval::{
    average_precision_at_k, hybrid_candidates, hybrid_search, ndcg_at_k, recall_at_k, rerank,
    reciprocal_rank, search, vector_search,
};
```

Add a constant near `COVERAGE_K` (line ~228):

```rust
/// Candidate depth the reranker reorders — the recall union pulled before rerank. Mirrors
/// `raki_retrieval::HYBRID_CANDIDATE_POOL`; production may later use a latency-bounded window.
const RERANK_POOL: usize = 20;
```

- [ ] **Step 4: Thread the reranker through `run_eval` and score `reranked`**

Change the `run_eval` signature (line 233) to accept the reranker:

```rust
pub async fn run_eval(
    embedder: Arc<dyn EmbeddingProvider>,
    reranker: Arc<dyn Reranker>,
    k: usize,
) -> Result<EvalRun, DomainError> {
```

Inside, build a fixture-id→document-text map alongside `fixture_of`. Add the map declaration next to `fixture_of` (line 245):

```rust
    let mut text_of: HashMap<String, String> = HashMap::new();
```

and insert into it inside the corpus loop, right after `let doc = format!(...)` (line 258):

```rust
        text_of.insert(cn.id.clone(), doc.clone());
```

In the per-query loop, after computing `hy` (line ~300), compute the reranked ranking from the recall union:

```rust
        let pool_ids = to_fixture(
            &hybrid_candidates(&keyword, &vectors, embedder.as_ref(), &q.query, RERANK_POOL).await?,
            &fixture_of,
        );
        let candidates: Vec<(String, String)> = pool_ids
            .iter()
            .filter_map(|fid| text_of.get(fid).map(|t| (fid.clone(), t.clone())))
            .collect();
        let rr = rerank(reranker.as_ref(), &q.query, &candidates, cov_k.max(k)).await?;
```

Add the `reranked` field to the pushed `QueryResult` (after the `hybrid:` field, line ~317):

```rust
            reranked: MethodResult {
                scores: score_one(&rr, &relevant, k, q),
                ranked: truncate(&rr, k),
            },
```

- [ ] **Step 5: Aggregate `reranked`**

In `aggregate` (line ~360), add `reranked: mean_scores(qrs.iter().map(|q| q.reranked.scores)),` to the `CategoryReport` literal, and after `overall_hybrid` (line 370) add:

```rust
    let overall_reranked = mean_scores(per_query.iter().map(|q| q.reranked.scores));
```

and add `overall_reranked,` to the returned `Report` literal (after `overall_hybrid`, line ~377).

- [ ] **Step 6: Update the lib harness test to pass a `FakeReranker` and assert reranked**

In `lib.rs` tests, update the import (line 417) and the `run_eval` call (line 422) in `harness_scores_every_category_with_fake_embedder`:

```rust
    use raki_ai::{FakeEmbeddingProvider, FakeReranker};
    ...
        let run = run_eval(Arc::new(FakeEmbeddingProvider::new(384)), Arc::new(FakeReranker), 5)
            .await
            .unwrap();
```

After the existing hybrid range assertions (line ~446), add:

```rust
        // reranked is computed, in range, for every scored category, and carries nDCG on
        // graded categories (the metric it is meant to move).
        for c in &report.by_category {
            assert!(c.reranked.recall >= 0.0 && c.reranked.recall <= 1.0);
        }
        assert!(report.overall_reranked.recall >= 0.0 && report.overall_reranked.recall <= 1.0);
        for cat in ["dense-near-duplicate", "paraphrase-distractor"] {
            let q = run.per_query.iter().find(|q| q.category == cat)
                .unwrap_or_else(|| panic!("missing {cat}"));
            assert!(q.reranked.scores.ndcg.is_some(), "{cat} reranked must carry nDCG (graded)");
        }
```

- [ ] **Step 7: Update `main.rs` — construct the reranker, render the `rr` column + delta, baseline column**

In `main.rs`: import `FastEmbedReranker` and `RERANKER_MODEL_ID` (line 7) and `Reranker` (line 8):

```rust
use raki_ai::{FastEmbedProvider, FastEmbedReranker};
use raki_domain::EmbeddingProvider;
```

Replace `row` (line 16) to take a 4th method and widen:

```rust
fn row(label: &str, kw: MethodScores, vc: MethodScores, hy: MethodScores, rr: MethodScores) {
    println!(
        "{label:<24} | kw R{:.2} N{} | vec R{:.2} N{} | hyb R{:.2} N{} | rr R{:.2} N{}",
        kw.recall, fmt_opt(kw.ndcg),
        vc.recall, fmt_opt(vc.ndcg),
        hy.recall, fmt_opt(hy.ndcg),
        rr.recall, fmt_opt(rr.ndcg),
    );
}
```

In `main`, construct the reranker and pass it (replace lines 44-47):

```rust
    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let reranker = Arc::new(FastEmbedReranker::try_new()?);
    let model_id = embedder.model_id();
    let reranker_id = reranker.model_id();
    let k = 3;
    let run = run_eval(embedder, reranker, k).await?;
```

Update the header line (line 50) to note the reranked column is hybrid+rerank, and update every `row(...)` call to pass the 4th method (`c.reranked` and `report.overall_reranked`), and widen the separator to `"-".repeat(96)`. After the OVERALL row, print the headline delta:

```rust
    println!(
        "\nreranked = hybrid + rerank ({reranker_id}). nDCG delta vs hybrid (graded categories):"
    );
    for c in report.by_category.iter().filter(|c| c.hybrid.ndcg.is_some()) {
        if let (Some(rr), Some(hy)) = (c.reranked.ndcg, c.hybrid.ndcg) {
            println!("  {:<24} {:+.3}", c.category, rr - hy);
        }
    }
```

In the per-query dump (line ~72) add `println!("    rr  {:?}", q.reranked.ranked);`.

Pass `reranker_id` into `write_artifacts`/`baseline_md`: change `write_artifacts(&run, &model_id, &date)?` (line 82) to `write_artifacts(&run, &model_id, &reranker_id, &date)?`, thread `reranker_id: &str` through `write_artifacts` and `baseline_md`. In `baseline_md`, add a line after the model-id line (line 115): `writeln!(s, "- Reranker model id: `{reranker_id}`").unwrap();`, change the per-category header/rows to add an `rr R/M/N/Cov` column (extend the `|` table and add `cell(c.reranked)` / `cell(r.overall_reranked)`).

- [ ] **Step 8: Update both gate callers to construct rerankers (compile only; floors come in Task 8)**

In `tests/eval_gate.rs`: import `FakeReranker, FastEmbedReranker` (line 14) and in `keyword_snapshot_is_deterministic` (line 51) pass `Arc::new(FakeReranker)`; in `real_model_gate` (line 65) pass `Arc::new(FastEmbedReranker::try_new()?)`:

```rust
use raki_ai::{FakeEmbeddingProvider, FakeReranker, FastEmbedProvider, FastEmbedReranker};
...
    let run = run_eval(Arc::new(FakeEmbeddingProvider::new(384)), Arc::new(FakeReranker), 3).await?;
...
    let run = run_eval(Arc::new(FastEmbedProvider::try_new()?), Arc::new(FastEmbedReranker::try_new()?), 3).await?;
```

- [ ] **Step 9: Compile + run the fake-path tests (gate still red on old snapshot — expected)**

Run: `cd src-tauri && cargo test -p raki-eval --lib`
Expected: PASS (the fake harness test now asserts reranked). The integration gate test `keyword_snapshot_is_deterministic` will FAIL to deserialize the old `snapshot.json` (missing `reranked`) — that is expected and fixed by the regen in the next step.

- [ ] **Step 10: Regenerate the snapshot + baseline (the make-it-green step)**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report -- --write --date=2026-06-06`
Then: `cd src-tauri && cargo test -p raki-eval`
Expected: PASS — `snapshot.json` now carries the `reranked` per-query block; `keyword_snapshot_is_deterministic` is green again; `real_model_gate` still uses the old (unchanged) floors and passes.

- [ ] **Step 11: Verify the fixtures fingerprint did NOT change (additive regen, not a corpus change)**

Run: `cd src-tauri && git diff docs/eval/baseline.md | grep -i fingerprint`
Expected: no change to the fingerprint line (fixtures untouched — only the schema grew). If the fingerprint changed, a fixture was accidentally edited; stop and investigate.

- [ ] **Step 12: Commit**

```bash
git add src-tauri/crates/raki-eval/src/lib.rs src-tauri/crates/raki-eval/src/main.rs src-tauri/crates/raki-eval/tests/eval_gate.rs docs/eval/snapshot.json docs/eval/baseline.md
git commit -m "Integrate reranked as a first-class eval method (= hybrid + rerank)"
```

---

## Task 7: Author-once real-model measurement (record, don't tune) — D-FALSIFY

**Files:**
- Modify: `docs/eval/judge-log.md`

- [ ] **Step 1: Run the real-model report and read the delta**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report`
Read the `reranked = hybrid + rerank ... nDCG delta vs hybrid (graded categories)` block and the OVERALL row. Note the per-category `reranked − hybrid` nDCG deltas for `lexical-cluster`, `dense-near-duplicate`, `paraphrase-distractor`, and whether reranked recall/MAP held vs hybrid.

- [ ] **Step 2: Record the measurement (fill the real numbers)**

Append to `docs/eval/judge-log.md`:

```markdown
## 2026-06-06 — Slice 4 author-once reranker measurement (D-FALSIFY)

Real model, k=3, corpus = 30 notes / 25 queries. Reranker: `jina-reranker-v1-turbo-en`
over the hybrid recall union (pool 20). `reranked` is `hybrid + rerank`.

reranked − hybrid nDCG@3 delta (graded categories):
- lexical-cluster:        <+/-X.XXX>
- dense-near-duplicate:   <+/-X.XXX>
- paraphrase-distractor:  <+/-X.XXX>

Recall@3 held vs hybrid: <yes/no, with any category that moved>.

Verdict (D-FALSIFY): <a positive delta is a measured ordering win on a toy corpus; a
~nil/negative delta is the recorded finding — the cross-encoder does not lift bge-small's
*visible* ordering at this scale, and recall-rescue (its real job) is unmeasurable here>.
No corpus tuning was done to produce a delta. The keep/kill decision is governed by D-DELETE
(`docs/eval/reranker-deletion-criteria.md`), which is decided on REAL ground truth, not this set.
```

Replace every `<...>` with the observed values and the honest verdict.

- [ ] **Step 3: Commit**

```bash
git add docs/eval/judge-log.md
git commit -m "Record Slice 4 author-once reranker measurement (D-FALSIFY)"
```

---

## Task 8: Additive downward floors for `reranked` — D6

**Files:**
- Modify: `src-tauri/crates/raki-eval/tests/eval_gate.rs`

- [ ] **Step 1: Read the reranked numbers to floor against**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report`
From the OVERALL row and per-category rows read: reranked non-coverage recall and MAP, and the MIN reranked nDCG across the three ordering categories.

- [ ] **Step 2: Add reranked floor constants (~0.10 below observed; never above)**

In `tests/eval_gate.rs`, after the existing `HYB_*` / `ORDERING_NDCG_FLOOR` constants (line ~29), add (use YOUR observed numbers, not these literals):

```rust
// Slice 4 (additive): reranked = hybrid + rerank. Floors ~0.10 below observed; existing
// floors are unchanged (this is not a downward re-baseline of the others). Measure-then-floor.
const RR_RECALL_FLOOR: f64 = 0.90; // ~0.10 below observed reranked non-coverage recall
const RR_MAP_FLOOR: f64 = 0.90; // ~0.10 below observed reranked non-coverage MAP
```

If an observed value is below a literal above, lower the constant to ~0.10 below the actual observed value (a floor must pass on the committed baseline). Do not raise existing floors.

- [ ] **Step 3: Add `Reranked` to the real-model snapshot check + per-method floors + nDCG loop**

In `real_model_gate`, change the snapshot methods to include `Reranked`:

```rust
    let regressions =
        snapshot_regressions(&run.per_query, &baseline, &[Method::Vector, Method::Hybrid, Method::Reranked]);
```

Add `Reranked` to the per-method floor loop:

```rust
    for (m, rf, mf) in [
        (Method::Keyword, KW_RECALL_FLOOR, KW_MAP_FLOOR),
        (Method::Vector, VEC_RECALL_FLOOR, VEC_MAP_FLOOR),
        (Method::Hybrid, HYB_RECALL_FLOOR, HYB_MAP_FLOOR),
        (Method::Reranked, RR_RECALL_FLOOR, RR_MAP_FLOOR),
    ] {
```

Add `Reranked` to the ordering-nDCG floor loop's inner method list:

```rust
        for m in [Method::Vector, Method::Hybrid, Method::Reranked] {
```

- [ ] **Step 4: Run the real-model gate**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate -- --ignored`
Expected: PASS. If a reranked floor fails, it was set too high — lower it to ~0.10 below the actual observed value (never tune the corpus to meet a floor).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/tests/eval_gate.rs
git commit -m "Gate reranked additively: snapshot + per-method + ordering-nDCG floors"
```

---

## Task 9: The D-DELETE deletion criterion (tracked ticket)

**Files:**
- Create: `docs/eval/reranker-deletion-criteria.md`

- [ ] **Step 1: Write the deletion ticket**

Create `docs/eval/reranker-deletion-criteria.md`:

```markdown
# Reranker deletion criterion (Slice 4, D-DELETE)

Status: OPEN — decided once real-notes ground truth exists.

The cross-encoder reranker (Slice 4) was built as an eval-substrate integration test on a
synthetic 30-note corpus that **cannot see its primary value** (recall-rescue): vector
recall@3 ≈ 1.0, so the relevant note is already in the top-k and there is nothing to rescue.
A nil delta on the *synthetic* set is therefore an expected, acceptable finding (D-FALSIFY)
and does **not** trigger deletion.

This ticket is the kill-switch, committed before attachment so the experiment cannot quietly
become permanent architecture.

## Tripwire

When real-notes ground truth exists (≥ ~100 labeled real queries sampled from actual use):

- Re-measure `reranked` vs `hybrid` on that ground truth.
- If `reranked` does NOT beat `hybrid` on nDCG by a stable, meaningful margin
  (**default +0.03**, re-set once the real query distribution is known) across the real set,
  then **remove**: the `Reranker` port (`raki-domain`), `FastEmbedReranker` + `FakeReranker`
  (`raki-ai`), the pure `rerank` fn (`raki-retrieval`), and the `reranked` eval method
  (`Method::Reranked`, the struct fields, `run_eval`'s reranker arg, the gate floors, the
  report column, the snapshot block).
- `hybrid_candidates` stays regardless — it is a clean recall primitive independent of rerank.

## Why a fixed tripwire now

D-FALSIFY (record the result honestly) is only a virtue if acted on. Writing the deletion
criterion before the result is known prevents the sunk-cost fallacy: the reranker survives by
earning a measured win on real data, not by already existing.
```

- [ ] **Step 2: Commit**

```bash
git add docs/eval/reranker-deletion-criteria.md
git commit -m "Add reranker deletion criterion (D-DELETE kill-switch)"
```

---

## Task 10: Verification + Definition of Done

- [ ] **Step 1: Full deterministic sweep (mirrors required CI)**

Run: `cd src-tauri && cargo test --workspace --exclude raki && cargo fmt --check && cargo clippy --workspace --exclude raki --all-targets -- -D warnings`
Expected: all pass, clean. (`--exclude raki` avoids the GUI crate's GTK deps.)

- [ ] **Step 2: Real-model gate green**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate -- --ignored`
Expected: PASS (vector/hybrid/reranked snapshots + all floors, including the new reranked floors).

- [ ] **Step 3: Real-model reranker edge-case test green**

Run: `cd src-tauri && cargo test -p raki-ai fastembed_reranker_orders_relevant_first -- --ignored`
Expected: PASS (relevance ordering + empty/oversized/empty-pool edges, no panic, finite scores).

- [ ] **Step 4: Artifacts consistent & idempotent**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report -- --write --date=2026-06-06` then (repo root) `git status --short docs/eval`.
Expected: clean (regeneration is idempotent). Confirm the baseline fingerprint is unchanged from before Slice 4 (fixtures untouched).

- [ ] **Step 5: DoD against the spec**

Confirm each: D1 (jina-turbo, smallest, swappable) ✓ Task 3 · D2 (Reranker port + both adapters; FakeReranker stub comment) ✓ Tasks 1-3 · D3 (hybrid_candidates split + characterization test) ✓ Task 4 · D4 (pure rerank fn) ✓ Task 5 · D5 (reranked first-class method, headline = reranked−hybrid delta) ✓ Tasks 6-7 · D6 (additive gate: snapshot + floors + nDCG loop; deterministic gate untouched) ✓ Tasks 6, 8 · D7 (one-time additive snapshot/baseline regen, fingerprint unchanged) ✓ Task 6 · D-FALSIFY (recorded measurement) ✓ Task 7 · D-DELETE (tracked ticket) ✓ Task 9. Production `search_notes` unchanged (eval-only) ✓ (no app-crate edits in any task).

- [ ] **Step 6: Frontend untouched (sanity)**

Run (repo root): `bun run typecheck && bun run build`
Expected: green (no frontend files changed this slice).

---

## Self-Review

**Spec coverage:** D1 → Task 3 (`JINARerankerV1TurboEn`, swappable, smallest). D2 → Tasks 1-3 (port + `FastEmbedReranker` + `FakeReranker` with the orchestration-stub comment). D3 → Task 4 (`hybrid_candidates` extraction behind the characterization test). D4 → Task 5 (pure `rerank`). D5 → Tasks 6-7 (first-class `Method::Reranked`; report labels it `= hybrid + rerank`; headline is the `reranked − hybrid` nDCG delta in both the report and judge-log). D6 → Tasks 6 + 8 (deterministic keyword gate untouched; real-model gate extended additively). D7 → Task 6 (one-time additive regen; fingerprint-unchanged check). D-FALSIFY → Task 7. D-DELETE → Task 9. Eval-only (no `search_notes`) → no task edits the `raki` app crate.

**Placeholder scan:** the only intentional placeholders are the `<...>` measured values in Task 7 (unknown until the run) and the example floor literals in Task 8 (explicitly "use YOUR observed numbers"). Every code step shows full code.

**Type/consistency:** `Reranker` / `RerankScore` (Task 1) are used identically in `FakeReranker`/`FastEmbedReranker` (Tasks 2-3), `rerank` (Task 5), and `run_eval` (Task 6). `rerank(reranker, query, candidates: &[(String,String)], k)` signature matches between Task 5's definition and Task 6's call. `Method::Reranked` is spelled identically in the enum, `method()`, the gate snapshot list, the floor loop, and the nDCG loop. `hybrid_candidates(keyword, vectors, embedder, query, pool)` matches between Task 4's definition and Task 6's call. `RERANK_POOL`/`RERANKER_MODEL_ID`/`RR_*_FLOOR` names are consistent across tasks.

**Known sequencing note:** the strict snapshot means Task 6 is intentionally one atomic commit (schema + all `run_eval` callers + regen); within it, `cargo test -p raki-eval` is briefly red after Step 9 and green after the Step 10 regen — the commit only happens at Step 12, green. The reranked floors stay absent (gate uses unchanged floors) through Tasks 6-7 and are added once in Task 8 (measure-then-floor), so no commit is red.

---

## Execution Handoff

(Presented to the user after saving.)
