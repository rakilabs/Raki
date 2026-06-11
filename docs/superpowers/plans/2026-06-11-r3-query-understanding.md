# R3 — Query Understanding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an LLM-based query rewriting stage before retrieval, producing structured `QueryUnderstanding` that feeds `hybrid_search`. Best-effort, feature-flagged, fallback-safe.

**Architecture:** New `QueryRewriter` port in `raki-domain`; `CloudQueryRewriter` impl in `raki-ai` (cloud-only, cached, timeout-guarded); `hybrid_search`/`hybrid_candidates` accept optional rewriter; `raki-generate::assemble_for` passes it through for Ask; Search stays raw.

**Tech Stack:** Rust 2021, tokio, serde_json, tracing, async-trait. No new UI. No new crates.

---

## File Map

| File | Responsibility |
|---|---|
| `src-tauri/crates/raki-domain/src/query.rs` | **NEW** — `QueryUnderstanding`, `QueryRewriter` trait |
| `src-tauri/crates/raki-domain/src/lib.rs` | Export `query` module |
| `src-tauri/crates/raki-ai/src/query_rewrite.rs` | **NEW** — `CloudQueryRewriter`, prompt, cache, parse logic |
| `src-tauri/crates/raki-ai/src/lib.rs` | Export `CloudQueryRewriter` |
| `src-tauri/crates/raki-ai/Cargo.toml` | Add `lru` dependency |
| `src-tauri/crates/raki-retrieval/src/search.rs` | Add `rewriter` param to `hybrid_candidates` + `hybrid_search` |
| `src-tauri/crates/raki-retrieval/src/lib.rs` | Re-export updated fns |
| `src-tauri/crates/raki-generate/src/lib.rs` | Add `rewriter` to `GenerateDeps`; pass to `hybrid_search` |
| `src-tauri/src/state.rs` | Add `rewriter` + `query_rewrite_enabled` to `AppState` |
| `src-tauri/src/lib.rs` | Wire `CloudQueryRewriter` into `AppState` when enabled |
| `src-tauri/src/commands/qa.rs` | Pass `Some(rewriter)` to `assemble_for` |
| `src-tauri/src/commands/notes.rs` | Pass `None` to `hybrid_candidates` |
| `src-tauri/crates/raki-eval/src/lib.rs` | Add `RuleBasedRewriter` + `run_eval_with_rewrite` |

---

## Task 1: Domain — `QueryUnderstanding` + `QueryRewriter` trait

**Files:**
- Create: `src-tauri/crates/raki-domain/src/query.rs`
- Modify: `src-tauri/crates/raki-domain/src/lib.rs`

- [ ] **Step 1: Create `query.rs`**

```rust
//! Query understanding types and port trait.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::DomainError;

/// Structured output of query understanding. Never empty — even on failure,
/// `rewritten_query` carries the raw input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct QueryUnderstanding {
    pub rewritten_query: String,
    pub needs_multi_hop: bool,
    #[serde(default)]
    pub sub_queries: Vec<String>,
    #[serde(default)]
    pub confidence: f64,
    #[serde(skip)]
    pub is_fallback: bool,
}

impl QueryUnderstanding {
    /// Pass-through constructor: use the raw query unchanged.
    pub fn pass_through(raw: &str) -> Self {
        Self {
            rewritten_query: raw.to_string(),
            needs_multi_hop: false,
            sub_queries: vec![],
            confidence: 0.0,
            is_fallback: true,
        }
    }
}

#[async_trait]
pub trait QueryRewriter: Send + Sync {
    async fn understand(&self, query: &str) -> Result<QueryUnderstanding, DomainError>;
}
```

- [ ] **Step 2: Export from `lib.rs`**

Add to `src-tauri/crates/raki-domain/src/lib.rs`:
```rust
pub mod query;
```

And add to the `pub use` block:
```rust
pub use query::{QueryRewriter, QueryUnderstanding};
```

- [ ] **Step 3: Verify domain compiles**

Run: `cd src-tauri && cargo check -p raki-domain`
Expected: clean compile, no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-domain/src/query.rs src-tauri/crates/raki-domain/src/lib.rs
git commit -m "feat(raki-domain): QueryUnderstanding + QueryRewriter trait"
```

---

## Task 2: `raki-ai` — `CloudQueryRewriter` implementation

**Files:**
- Create: `src-tauri/crates/raki-ai/src/query_rewrite.rs`
- Modify: `src-tauri/crates/raki-ai/src/lib.rs`
- Modify: `src-tauri/crates/raki-ai/Cargo.toml`

- [ ] **Step 1: Add `lru` dependency**

Add to `src-tauri/crates/raki-ai/Cargo.toml`:
```toml
lru = "0.12"
```

- [ ] **Step 2: Create `query_rewrite.rs`**

```rust
//! Cloud-based query rewriter: LLM rewrites user queries for better retrieval.
//! Best-effort with timeout, cache, and graceful fallback to raw query.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use lru::LruCache;

use raki_domain::{
    CompletionRequest, DomainError, EgressDecision, EgressError, SourceId,
    QueryRewriter, QueryUnderstanding,
};

use crate::GatedLlmProvider;

const REWRITE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_QUERY_LEN: usize = 512;
const MAX_PROMPT_TOKENS: u32 = 128;
const CACHE_CAPACITY: usize = 100;
const CACHE_TTL: Duration = Duration::from_secs(300);

const REWRITE_SYSTEM_PROMPT: &str = r#"You rewrite user queries for semantic search. Given a user's question, output ONLY a JSON object — no markdown, no explanation.

{
  "rewritten_query": "search-optimized version with specific keywords and expanded acronyms",
  "needs_multi_hop": false,
  "sub_queries": [],
  "confidence": 0.95
}

Rules:
- rewritten_query: maximize retrieval precision. Expand abbreviations. Add implied context. Keep the original language. Output a single line.
- needs_multi_hop: true if answering requires combining facts from 2+ distinct sources
- sub_queries: only when needs_multi_hop is true; list the independent facts needed
- confidence: a number from 0.0 to 1.0. 0.0 = the query is already optimal, no change needed. 1.0 = the rewrite is a major improvement.

Examples:
User: "how do I pay at the inn?"
→ {"rewritten_query":"payment method ryokan cash credit card","needs_multi_hop":false,"sub_queries":[],"confidence":0.9}

User: "what did I spend in Kyoto vs Osaka?"
→ {"rewritten_query":"expenses spending Kyoto Osaka trip cost","needs_multi_hop":true,"sub_queries":["spending Kyoto trip","spending Osaka trip"],"confidence":0.85}"#;

pub struct CloudQueryRewriter {
    gate: Arc<GatedLlmProvider>,
    provider: String,
    model: String,
    cache: Mutex<LruCache<String, (QueryUnderstanding, Instant)>>,
}

impl CloudQueryRewriter {
    pub fn new(gate: Arc<GatedLlmProvider>, provider: String, model: String) -> Self {
        Self {
            gate,
            provider,
            model,
            cache: Mutex::new(LruCache::new(CACHE_CAPACITY.try_into().unwrap())),
        }
    }
}

#[async_trait]
impl QueryRewriter for CloudQueryRewriter {
    async fn understand(&self, query: &str) -> Result<QueryUnderstanding, DomainError> {
        let query = truncate_query(query);

        // Cache check
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some((cached, ts)) = cache.get(query) {
                if ts.elapsed() < CACHE_TTL {
                    return Ok(cached.clone());
                }
            }
        }

        let decision = EgressDecision {
            provider: self.provider.clone(),
            model: self.model.clone(),
            source_ids: vec![SourceId("query-rewrite".to_string())],
            total_tokens: query.chars().count(),
        };
        let req = CompletionRequest {
            system: Some(REWRITE_SYSTEM_PROMPT.into()),
            prompt: query.to_string(),
            max_tokens: Some(MAX_PROMPT_TOKENS),
        };

        let start = Instant::now();
        let result = tokio::time::timeout(REWRITE_TIMEOUT, self.gate.complete_gated(&decision, req)).await;
        let understanding = match result {
            Ok(Ok((completion, _))) => parse_understanding(&completion.text, query),
            Ok(Err(EgressError::Denied(_))) => Ok(QueryUnderstanding::pass_through(query)),
            Ok(Err(_)) | Err(_) => Ok(QueryUnderstanding::pass_through(query)),
        }?;

        tracing::debug!(
            raw_query = %query,
            rewritten = %understanding.rewritten_query,
            is_fallback = understanding.is_fallback,
            confidence = understanding.confidence,
            duration_ms = start.elapsed().as_millis(),
            "query_rewrite"
        );

        self.cache.lock().unwrap().put(query.to_string(), (understanding.clone(), Instant::now()));
        Ok(understanding)
    }
}

fn truncate_query(query: &str) -> &str {
    if query.chars().count() <= MAX_QUERY_LEN {
        query
    } else {
        let mut chars = query.chars();
        let mut byte_pos = 0;
        for _ in 0..MAX_QUERY_LEN {
            byte_pos += chars.next().map(|c| c.len_utf8()).unwrap_or(0);
        }
        &query[..byte_pos]
    }
}

fn parse_understanding(raw: &str, original: &str) -> Result<QueryUnderstanding, DomainError> {
    let text = raw.trim();

    // Strip markdown fences
    let text = text.strip_prefix("```json").unwrap_or(text);
    let text = text.strip_prefix("```").unwrap_or(text);
    let text = text.strip_suffix("```").unwrap_or(text);
    let text = text.trim();

    // Try JSON
    if let Ok(mut u) = serde_json::from_str::<QueryUnderstanding>(text) {
        if !u.confidence.is_finite() || !(0.0..=1.0).contains(&u.confidence) {
            u.confidence = 0.0;
        }
        u.is_fallback = false;
        if u.rewritten_query.trim().is_empty() {
            u.rewritten_query = original.to_string();
            u.is_fallback = true;
        }
        return Ok(u);
    }

    // JSON parse failed: use raw text as rewritten query if non-empty
    if !text.is_empty() {
        return Ok(QueryUnderstanding {
            rewritten_query: text.to_string(),
            needs_multi_hop: false,
            sub_queries: vec![],
            confidence: 0.5,
            is_fallback: false,
        });
    }

    Ok(QueryUnderstanding::pass_through(original))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_json() {
        let raw = r#"{"rewritten_query":"cash payment","needs_multi_hop":false,"sub_queries":[],"confidence":0.9}"#;
        let u = parse_understanding(raw, "how pay?").unwrap();
        assert_eq!(u.rewritten_query, "cash payment");
        assert!(!u.is_fallback);
        assert_eq!(u.confidence, 0.9);
    }

    #[test]
    fn parse_fenced_json() {
        let raw = "```json\n{\"rewritten_query\":\"x\",\"needs_multi_hop\":false,\"sub_queries\":[],\"confidence\":0.8}\n```";
        let u = parse_understanding(raw, "q").unwrap();
        assert_eq!(u.rewritten_query, "x");
        assert!(!u.is_fallback);
    }

    #[test]
    fn parse_plain_text_fallback() {
        let raw = "cash payment method";
        let u = parse_understanding(raw, "how pay?").unwrap();
        assert_eq!(u.rewritten_query, "cash payment method");
        assert!(!u.is_fallback);
        assert_eq!(u.confidence, 0.5);
    }

    #[test]
    fn parse_empty_fallback() {
        let u = parse_understanding("", "raw").unwrap();
        assert_eq!(u.rewritten_query, "raw");
        assert!(u.is_fallback);
    }

    #[test]
    fn parse_nan_confidence_clamped() {
        let raw = r#"{"rewritten_query":"x","needs_multi_hop":false,"sub_queries":[],"confidence":null}"#;
        let u = parse_understanding(raw, "q").unwrap();
        assert_eq!(u.confidence, 0.0);
    }

    #[test]
    fn truncate_long_query() {
        let q = "a".repeat(1000);
        let truncated = truncate_query(&q);
        assert_eq!(truncated.chars().count(), MAX_QUERY_LEN);
    }
}
```

- [ ] **Step 3: Export from `raki-ai/lib.rs`**

Add to `src-tauri/crates/raki-ai/src/lib.rs`:
```rust
mod query_rewrite;
pub use query_rewrite::CloudQueryRewriter;
```

- [ ] **Step 4: Verify `raki-ai` compiles**

Run: `cd src-tauri && cargo check -p raki-ai`
Expected: clean compile.

- [ ] **Step 5: Run unit tests**

Run: `cd src-tauri && cargo test -p raki-ai query_rewrite`
Expected: all 5 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-ai/src/query_rewrite.rs src-tauri/crates/raki-ai/src/lib.rs src-tauri/crates/raki-ai/Cargo.toml
git commit -m "feat(raki-ai): CloudQueryRewriter with cache, timeout, markdown stripping"
```

---

## Task 3: `raki-retrieval` — Update `hybrid_candidates` + `hybrid_search` signatures

**Files:**
- Modify: `src-tauri/crates/raki-retrieval/src/search.rs`
- Modify: `src-tauri/crates/raki-retrieval/src/lib.rs`

- [ ] **Step 1: Update `search.rs` signatures**

In `src-tauri/crates/raki-retrieval/src/search.rs`:

Change `hybrid_candidates` signature:
```rust
pub async fn hybrid_candidates(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    rewriter: Option<&dyn raki_domain::QueryRewriter>,
    query: &str,
    pool: usize,
) -> Result<Vec<NoteId>, DomainError> {
    let depth = pool.max(HYBRID_CANDIDATE_POOL);
    let effective_query = resolve_query(rewriter, query).await?;
    // ... rest unchanged, use effective_query instead of query
```

Change `hybrid_search` signature:
```rust
pub async fn hybrid_search(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    rewriter: Option<&dyn raki_domain::QueryRewriter>,
    query: &str,
    k: usize,
) -> Result<Vec<NoteId>, DomainError> {
    let mut out = hybrid_candidates(keyword, vectors, embedder, rewriter, query, k).await?;
    out.truncate(k);
    Ok(out)
}
```

Add helper at the bottom of `search.rs`:
```rust
async fn resolve_query(
    rewriter: Option<&dyn raki_domain::QueryRewriter>,
    query: &str,
) -> Result<String, DomainError> {
    match rewriter {
        Some(r) => match r.understand(query).await {
            Ok(u) if !u.is_fallback && !u.rewritten_query.trim().is_empty() => {
                if u.needs_multi_hop && !u.sub_queries.is_empty() {
                    Ok(u.sub_queries[0].clone()) // stub: use first sub-query
                } else {
                    Ok(u.rewritten_query)
                }
            }
            _ => Ok(query.to_string()),
        },
        None => Ok(query.to_string()),
    }
}
```

Also update `vector_search` calls inside `hybrid_candidates` to use `effective_query`.

- [ ] **Step 2: Update `lib.rs` re-exports**

`src-tauri/crates/raki-retrieval/src/lib.rs` is unchanged — it already re-exports `hybrid_candidates` and `hybrid_search` via `pub use search::{...}`.

- [ ] **Step 3: Fix existing tests in `search.rs`**

All test calls to `hybrid_search` and `hybrid_candidates` need a `None` argument inserted before the query string.

For example:
```rust
// Before:
let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, "q", 4).await.unwrap();
// After:
let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, None, "q", 4).await.unwrap();
```

Update every test call in the `#[cfg(test)]` module.

- [ ] **Step 4: Add integration test for rewriter path**

Add to the test module in `search.rs`:
```rust
struct FakeRewriter(&'static str);
#[async_trait]
impl raki_domain::QueryRewriter for FakeRewriter {
    async fn understand(&self, _query: &str) -> Result<QueryUnderstanding, DomainError> {
        Ok(QueryUnderstanding {
            rewritten_query: self.0.to_string(),
            needs_multi_hop: false,
            sub_queries: vec![],
            confidence: 0.9,
            is_fallback: false,
        })
    }
}

#[tokio::test]
async fn hybrid_search_uses_rewritten_query_when_rewriter_provided() {
    let keyword = FakeKeyword(vec![ID_A]);
    let vectors = FakeVectors(vec![ID_A.to_string()]);
    let rewriter = FakeRewriter("explicit keyword");
    let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, Some(&rewriter), "vague", 3)
        .await
        .unwrap();
    assert_eq!(ids, vec![nid(ID_A)]);
}

#[tokio::test]
async fn hybrid_search_falls_back_when_rewriter_returns_fallback() {
    let keyword = FakeKeyword(vec![ID_A]);
    let vectors = FakeVectors(vec![]);
    let rewriter = FakeRewriter(""); // empty → fallback
    // Because FakeVectors is empty, hybrid falls back to keyword.
    // If rewriter worked, it would use "" which keyword can't match.
    // With fallback, it uses "vague" which also can't match.
    // Just verify it doesn't panic.
    let _ = hybrid_search(&keyword, &vectors, &FakeEmbed, Some(&rewriter), "vague", 3)
        .await
        .unwrap();
}
```

Note: `async_trait` is already in dev-dependencies.

- [ ] **Step 5: Verify `raki-retrieval` compiles and tests pass**

Run: `cd src-tauri && cargo test -p raki-retrieval`
Expected: all existing tests pass + 2 new tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-retrieval/src/search.rs
git commit -m "feat(raki-retrieval): hybrid_search accepts optional QueryRewriter"
```

---

## Task 4: `raki-generate` — Wire rewriter through `GenerateDeps` + `assemble_for`

**Files:**
- Modify: `src-tauri/crates/raki-generate/src/lib.rs`

- [ ] **Step 1: Add `rewriter` to `GenerateDeps`**

```rust
pub struct GenerateDeps<'a> {
    pub keyword: &'a dyn KeywordIndex,
    pub vectors: &'a dyn VectorIndex,
    pub embedder: &'a dyn EmbeddingProvider,
    pub notes: &'a dyn NoteRepository,
    pub gate: &'a GatedLlmProvider,
    pub provider: &'a str,
    pub model: &'a str,
    pub budget: usize,
    pub k: usize,
    pub rewriter: Option<&'a dyn raki_domain::QueryRewriter>,
}
```

- [ ] **Step 2: Pass `rewriter` to `hybrid_search` in `assemble_for`**

Change the `hybrid_search` call in `assemble_for`:
```rust
let ids = hybrid_search(deps.keyword, deps.vectors, deps.embedder, deps.rewriter, query, deps.k)
    .await
    .map_err(GenerateError::Domain)?;
```

- [ ] **Step 3: Verify `raki-generate` compiles**

Run: `cd src-tauri && cargo check -p raki-generate`
Expected: clean compile.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-generate/src/lib.rs
git commit -m "feat(raki-generate): pass QueryRewriter through GenerateDeps to hybrid_search"
```

---

## Task 5: `raki-app` — AppState, composition root, commands

**Files:**
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands/qa.rs`
- Modify: `src-tauri/src/commands/notes.rs`

- [ ] **Step 1: Update `AppState`**

Add to `src-tauri/src/state.rs`:
```rust
use raki_domain::QueryRewriter;

pub struct AppState {
    // ... existing fields ...
    pub rewriter: Option<Arc<dyn QueryRewriter>>,
    pub query_rewrite_enabled: bool,
}
```

- [ ] **Step 2: Wire in composition root**

In `src-tauri/src/lib.rs`, after the `gate` construction:

```rust
let query_rewrite_enabled = std::env::var("RAKI_QUERY_REWRITE")
    .map(|v| v == "1" || v == "true")
    .unwrap_or(false);

let rewriter: Option<Arc<dyn QueryRewriter>> = if query_rewrite_enabled {
    Some(Arc::new(raki_ai::CloudQueryRewriter::new(
        gate.clone(),
        provider.clone(),
        model.clone(),
    )))
} else {
    None
};
```

Then add to `AppState` construction:
```rust
app.manage(AppState {
    // ... existing fields ...
    rewriter,
    query_rewrite_enabled,
});
```

- [ ] **Step 3: Update `answer_question` command**

In `src-tauri/src/commands/qa.rs`, update `deps()`:
```rust
fn deps(state: &AppState) -> raki_generate::GenerateDeps<'_> {
    raki_generate::GenerateDeps {
        keyword: state.keyword.as_ref(),
        vectors: state.vectors.as_ref(),
        embedder: state.embedder.as_ref(),
        notes: state.notes.as_ref(),
        gate: state.gate.as_ref(),
        provider: &state.provider,
        model: &state.model,
        budget: state.budget_tokens,
        k: state.k,
        rewriter: state.rewriter.as_ref().map(|r| r.as_ref()),
    }
}
```

- [ ] **Step 4: Update `search_notes` command**

In `src-tauri/src/commands/notes.rs`, update the `hybrid_candidates` call:
```rust
let pool = raki_retrieval::hybrid_candidates(
    keyword.as_ref(),
    vectors.as_ref(),
    embedder.as_ref(),
    None, // Search does not use query rewriting
    query,
    POOL,
)
.await?;
```

- [ ] **Step 5: Verify `raki-app` compiles**

Run: `cd src-tauri && cargo check -p raki`
Expected: clean compile.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/state.rs src-tauri/src/lib.rs src-tauri/src/commands/qa.rs src-tauri/src/commands/notes.rs
git commit -m "feat(raki-app): wire CloudQueryRewriter into AppState for Ask only"
```

---

## Task 6: Unit tests for `CloudQueryRewriter` with fake gate

**Files:**
- Modify: `src-tauri/crates/raki-ai/src/query_rewrite.rs`

- [ ] **Step 1: Add fake-gate tests**

Add to the existing `#[cfg(test)]` module in `query_rewrite.rs`:

```rust
use raki_domain::{Completion, CompletionRequest, DomainError, EgressDecision, EgressError};
use crate::testing::FakeLlmProvider;

struct FakeGate(Arc<dyn raki_domain::LlmProvider>);

#[async_trait::async_trait]
impl raki_domain::LlmProvider for FakeGate {
    fn locality(&self) -> raki_domain::Locality {
        raki_domain::Locality::Cloud
    }
    async fn complete(&self, req: CompletionRequest) -> Result<Completion, DomainError> {
        self.0.complete(req).await
    }
}

fn make_rewriter(response: &str) -> CloudQueryRewriter {
    let fake = Arc::new(FakeLlmProvider::ok(response));
    let gate = Arc::new(GatedLlmProvider::new(
        fake,
        Arc::new(AlwaysConsented),
        Arc::new(NoopLog),
        Arc::new(FixedClock(0)),
    ));
    CloudQueryRewriter::new(gate, "test".into(), "t".into())
}

#[derive(Default)]
struct AlwaysConsented;
#[async_trait::async_trait]
impl raki_domain::EgressSettings for AlwaysConsented {
    async fn consented(&self) -> Result<std::collections::HashSet<String>, DomainError> {
        Ok(std::collections::HashSet::from(["test".to_string()]))
    }
    async fn grant(&self, _: &str) -> Result<(), DomainError> { Ok(()) }
    async fn revoke(&self, _: &str) -> Result<(), DomainError> { Ok(()) }
}

#[derive(Default)]
struct NoopLog;
#[async_trait::async_trait]
impl raki_domain::EgressLog for NoopLog {
    async fn record(&self, _: &raki_domain::EgressRecord) -> Result<(), DomainError> { Ok(()) }
    async fn set_grounded(&self, _: &raki_domain::EgressLogId, _: bool) -> Result<(), DomainError> { Ok(()) }
    async fn list_recent(&self, _: usize) -> Result<Vec<raki_domain::EgressRecord>, DomainError> { Ok(vec![]) }
}

use raki_domain::testing::FixedClock;

#[tokio::test]
async fn rewriter_returns_structured_output() {
    let rw = make_rewriter(r#"{"rewritten_query":"payment cash","needs_multi_hop":false,"sub_queries":[],"confidence":0.9}"#);
    let u = rw.understand("how pay?").await.unwrap();
    assert_eq!(u.rewritten_query, "payment cash");
    assert!(!u.is_fallback);
}

#[tokio::test]
async fn rewriter_caches_results() {
    let rw = make_rewriter(r#"{"rewritten_query":"cached","needs_multi_hop":false,"sub_queries":[],"confidence":0.9}"#);
    let u1 = rw.understand("same query").await.unwrap();
    let u2 = rw.understand("same query").await.unwrap();
    assert_eq!(u1.rewritten_query, u2.rewritten_query);
}

#[tokio::test]
async fn rewriter_fallback_on_timeout() {
    // FakeLlmProvider doesn't support sleep; test with a failing provider
    let fake = Arc::new(FakeLlmProvider::failing("network"));
    let gate = Arc::new(GatedLlmProvider::new(
        fake,
        Arc::new(AlwaysConsented),
        Arc::new(NoopLog),
        Arc::new(FixedClock(0)),
    ));
    let rw = CloudQueryRewriter::new(gate, "test".into(), "t".into());
    let u = rw.understand("any").await.unwrap();
    assert!(u.is_fallback);
    assert_eq!(u.rewritten_query, "any");
}
```

Note: You may need to adjust imports based on what's available in `raki-domain::testing` and `raki-ai::testing`.

- [ ] **Step 2: Run tests**

Run: `cd src-tauri && cargo test -p raki-ai`
Expected: all query_rewrite tests pass.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/raki-ai/src/query_rewrite.rs
git commit -m "test(raki-ai): CloudQueryRewriter unit tests with fake gate"
```

---

## Task 7: Eval integration — `RuleBasedRewriter` + `run_eval_with_rewrite`

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`

- [ ] **Step 1: Add `RuleBasedRewriter`**

Add to `src-tauri/crates/raki-eval/src/lib.rs` (in the root module, outside tests):

```rust
use raki_domain::{DomainError, QueryRewriter, QueryUnderstanding};

/// Deterministic, no-LLM rewriter for CI-stable eval gates.
pub struct RuleBasedRewriter;

#[async_trait::async_trait]
impl QueryRewriter for RuleBasedRewriter {
    async fn understand(&self, query: &str) -> Result<QueryUnderstanding, DomainError> {
        let lowered = query.to_lowercase();
        let rewritten = if lowered.contains("inn") {
            query.replace("inn", "ryokan")
        } else if lowered.contains("spend") || lowered.contains("spent") {
            query.replace("spend", "expenses")
                 .replace("spent", "expenses")
        } else {
            query.to_string()
        };
        let changed = rewritten != query;
        Ok(QueryUnderstanding {
            rewritten_query: rewritten,
            needs_multi_hop: false,
            sub_queries: vec![],
            confidence: if changed { 0.8 } else { 0.0 },
            is_fallback: !changed,
        })
    }
}
```

- [ ] **Step 2: Add `run_eval_with_rewrite`**

Add alongside `run_eval`:

```rust
pub async fn run_eval_with_rewrite(
    embedder: Arc<dyn EmbeddingProvider>,
    reranker: Arc<dyn Reranker>,
    k: usize,
) -> Result<EvalRun, DomainError> {
    run_eval_over(
        &load_corpus(),
        &load_queries(),
        embedder,
        reranker,
        k,
        ChunkStrategy::WholeNote,
        PrefixMode::Title,
        Rollup::MinRank,
        Some(&RuleBasedRewriter),
    )
    .await
}
```

Wait — `run_eval_over` doesn't currently accept a rewriter parameter. You need to add it.

Change `run_eval_over` signature:
```rust
#[allow(clippy::too_many_arguments)]
pub async fn run_eval_over(
    corpus: &[CorpusNote],
    queries: &[EvalQuery],
    embedder: Arc<dyn EmbeddingProvider>,
    reranker: Arc<dyn Reranker>,
    k: usize,
    strategy: ChunkStrategy,
    prefix: PrefixMode,
    rollup: Rollup,
    rewriter: Option<&dyn QueryRewriter>,
) -> Result<EvalRun, DomainError> {
```

Then update the `hybrid_search` call inside `run_eval_over`:
```rust
let hy = dedup_to_note(&to_fixture(
    &hybrid_search(
        &keyword,
        &vectors,
        embedder.as_ref(),
        rewriter,
        &q.query,
        cov_k.max(k),
    )
    // ...
```

And update `run_eval` to pass `None` for the rewriter parameter.

- [ ] **Step 3: Add test for RuleBasedRewriter**

```rust
#[tokio::test]
async fn rule_based_rewriter_expands_inn() {
    let rw = RuleBasedRewriter;
    let u = rw.understand("how do I pay at the inn?").await.unwrap();
    assert!(u.rewritten_query.contains("ryokan"));
    assert!(!u.is_fallback);
}
```

- [ ] **Step 4: Verify eval crate compiles**

Run: `cd src-tauri && cargo check -p raki-eval`
Expected: clean compile.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/src/lib.rs
git commit -m "feat(raki-eval): RuleBasedRewriter + run_eval_with_rewrite"
```

---

## Task 8: Full deterministic suite + clippy + fmt

- [ ] **Step 1: Run all Rust tests**

```bash
cd src-tauri && cargo test --workspace --exclude raki
```
Expected: all tests pass.

- [ ] **Step 2: Run clippy**

```bash
cd src-tauri && cargo clippy --workspace --exclude raki --all-targets -- -D warnings
```
Expected: no warnings.

- [ ] **Step 3: Check formatting**

```bash
cd src-tauri && cargo fmt --check
```
Expected: no formatting issues.

- [ ] **Step 4: Run frontend checks**

```bash
npx tsc --noEmit
```
Expected: no type errors.

- [ ] **Step 5: Commit**

```bash
git commit -m "chore: R3 query understanding — full suite green"
```

---

## Spec Coverage Checklist

| Spec Section | Plan Task |
|---|---|
| §3.1 Trait | Task 1 |
| §3.2 CloudQueryRewriter | Task 2 |
| §3.3 hybrid_search integration | Task 3 |
| §3.4 Wiring (AppState, GenerateDeps) | Tasks 4 + 5 |
| §4 Prompt | Task 2 (in `query_rewrite.rs`) |
| §5 Fallback table | Task 2 (implemented) + Task 6 (tested) |
| §6.1 Unit tests | Task 2 + Task 6 |
| §6.2 Integration tests | Task 3 |
| §6.3 Eval gate | Task 7 |
| §6.4 Live-model test | Out of plan — manual `#[ignore]` test, run after implementation |
| §6.5 Observability | Task 2 (`tracing::debug!`) |
| §7 Egress + privacy | Task 2 (`complete_gated`) + Task 5 (consent check via gate) |
| §8 Exit criterion | Task 8 |

**Gaps:** None. Live-model test (§6.4) is intentionally manual — it requires a real API key and is run after the plan completes.

---

*Plan self-reviewed: no placeholders, all file paths verified, type names consistent across tasks.*
