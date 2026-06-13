# R3 — Generate-Stage Query Understanding

> **Status:** Design approved, post-review revision. Awaits implementation plan.  
> **Depends on:** R2 (chunk-level embeddings, production).  
> **Gated by:** Live-model eval lift ≥ +0.05 MAP on `buried-fact-in-long-note` category.

---

## 1. Goal

Improve retrieval quality by adding an LLM-based **query understanding** stage before embedding + keyword search. The LLM rewrites vague user queries into precise retrieval queries. The rewrite is **opt-in, feature-flagged, and best-effort** — failure silently falls back to the raw query.

This is ADR-0006 Stage 3: "Generate" — the LLM layer that feeds recall.

---

## 2. Non-goals

- **HyDE** (Hypothetical Document Embedding) — out of scope. The prompt is "do NOT answer the question." HyDE deferred.
- **Multi-hop execution** — `sub_queries` field accepted but not acted on (stub). Full multi-hop deferred.
- **Answer formatting** — no intent classification, no response template changes.
- **Local LLM** — cloud-only (Kimi/Anthropic via `MessagesProvider`). Local LLM deferred.
- **Search wiring** — only Ask gets rewriting in this slice. Search stays raw-query.

---

## 3. Architecture

### 3.1 Trait (port in `raki-domain`)

```rust
/// Structured output of query understanding. Never empty — even on failure,
/// `rewritten_query` carries the raw input.
pub struct QueryUnderstanding {
    pub rewritten_query: String,
    pub needs_multi_hop: bool,
    pub sub_queries: Vec<String>,
    pub confidence: f64,
    /// true when the result is a fallback (error, timeout, low confidence).
    /// `hybrid_search` uses this as the gate, not the confidence heuristic.
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

### 3.2 Implementation (in `raki-ai`)

```rust
pub struct CloudQueryRewriter {
    gate: Arc<GatedLlmProvider>,
    provider: String,
    model: String,
    timeout: Duration,
    /// Cache keyed by raw query string. TTL = 5 minutes.
    cache: Mutex<LruCache<String, QueryUnderstanding>>,
}

const REWRITE_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_QUERY_LEN: usize = 512; // Unicode scalars
const MAX_PROMPT_TOKENS: u32 = 128;

#[async_trait]
impl QueryRewriter for CloudQueryRewriter {
    async fn understand(&self, query: &str) -> Result<QueryUnderstanding, DomainError> {
        // 1. Size bound
        let query = if query.chars().count() > MAX_QUERY_LEN {
            &query[..query.char_indices().nth(MAX_QUERY_LEN).map(|(i,_)| i).unwrap_or(query.len())]
        } else {
            query
        };

        // 2. Cache check
        if let Some(cached) = self.cache.lock().unwrap().get(query) {
            return Ok(cached.clone());
        }

        // 3. Build EgressDecision (domain-level type — no dependency rule violation)
        let decision = EgressDecision {
            provider: self.provider.clone(),
            model: self.model.clone(),
            source_ids: vec![SourceId("query-rewrite".to_string())],
            total_tokens: query.chars().count(), // safe over-estimate; CJK-aware
        };
        let req = CompletionRequest {
            system: Some(REWRITE_SYSTEM_PROMPT.into()),
            prompt: query.to_string(),
            max_tokens: Some(MAX_PROMPT_TOKENS),
        };

        // 4. Call with timeout
        let result = tokio::time::timeout(REWRITE_TIMEOUT, self.gate.complete_gated(&decision, req)).await;
        let understanding = match result {
            Ok(Ok((completion, _))) => parse_understanding(&completion.text, query),
            Ok(Err(EgressError::Denied(_))) => Ok(QueryUnderstanding::pass_through(query)),
            Ok(Err(_)) | Err(_) => Ok(QueryUnderstanding::pass_through(query)),
        }?;

        // 5. Cache success (even fallback is cached to avoid hammering the provider)
        self.cache.lock().unwrap().put(query.to_string(), understanding.clone());
        Ok(understanding)
    }
}

/// Parse LLM response. Strip markdown fences, validate JSON, clamp confidence.
fn parse_understanding(raw: &str, original: &str) -> Result<QueryUnderstanding, DomainError> {
    let text = raw.trim();

    // Strip optional markdown fences
    let text = text.strip_prefix("```json").unwrap_or(text);
    let text = text.strip_prefix("```").unwrap_or(text);
    let text = text.strip_suffix("```").unwrap_or(text);
    let text = text.trim();

    // Try JSON
    if let Ok(mut u) = serde_json::from_str::<QueryUnderstanding>(text) {
        // Validate confidence
        if !u.confidence.is_finite() || !(0.0..=1.0).contains(&u.confidence) {
            u.confidence = 0.0;
        }
        // LLM returned structured data — not a fallback unless it says so
        u.is_fallback = false;
        if u.rewritten_query.trim().is_empty() {
            u.rewritten_query = original.to_string();
            u.is_fallback = true;
        }
        return Ok(u);
    }

    // JSON parse failed: use the raw text as the rewritten query if it's non-empty
    if !text.is_empty() {
        return Ok(QueryUnderstanding {
            rewritten_query: text.to_string(),
            needs_multi_hop: false,
            sub_queries: vec![],
            confidence: 0.5, // passes the is_fallback gate; LLM gave us *something*
            is_fallback: false,
        });
    }

    Ok(QueryUnderstanding::pass_through(original))
}
```

**Architectural note on egress (C3 resolution):** `EgressDecision` lives in `raki-domain`. `GatedLlmProvider::complete_gated` accepts `&EgressDecision` directly. `raki-ai` constructing an `EgressDecision` does **not** import from `raki-memory` and does **not** violate the dependency rule. AGENTS.md's reference to `AssembledContext` describes the *answer-generation* path (where context items exist); query rewriting is a pre-retrieval call with no assembled context, so a domain-level `EgressDecision` is the correct and legal contract.

### 3.3 Integration (in `raki-retrieval`)

`hybrid_search` gains an `Option<&dyn QueryRewriter>` parameter. All existing callers are updated to pass `None` or `Some(rewriter)`:

```rust
pub async fn hybrid_search(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    rewriter: Option<&dyn QueryRewriter>,
    query: &str,
    k: usize,
) -> Result<Vec<NoteId>, DomainError> {
    let effective_query = match rewriter {
        Some(r) => match r.understand(query).await {
            Ok(u) if !u.is_fallback && !u.rewritten_query.trim().is_empty() => {
                if u.needs_multi_hop && !u.sub_queries.is_empty() {
                    // Stub: multi-hop not yet implemented; use first sub-query.
                    u.sub_queries[0].clone()
                } else {
                    u.rewritten_query
                }
            }
            _ => query.to_string(),
        },
        None => query.to_string(),
    };
    // ... existing embed + search using effective_query
}
```

**Callers to update:**
- `raki-memory::assemble_for` (via `GenerateDeps`) → passes `Some(rewriter)` when present
- `raki-app::search_notes` command → passes `None`
- `raki-app::answer_question` command → passes `Some(rewriter)` via `GenerateDeps`
- `raki-eval::run_eval_over` → accepts optional rewriter for A/B testing

### 3.4 Wiring (in `raki-app`)

- `AppState` gains `rewriter: Option<Arc<dyn QueryRewriter>>`.
- `GenerateDeps` gains `rewriter: Option<&'a dyn QueryRewriter>`.
- **Config:** `query_rewrite_enabled: bool` (default `false`). Only construct `CloudQueryRewriter` when `true` AND the provider is consented.
- **Ask** (`answer_question`): when enabled, passes `Some(state.rewriter.as_ref())` via `GenerateDeps`.
- **Search** (`search_notes`): passes `None`.
- **No cloud consent:** `rewriter = None`.

### 3.5 Frontend

No new UI components in this slice. Ask already shows a loading state during `assemble_for` → `send_answer`. The rewrite call adds ~300–800ms to this window; the existing spinner covers it. The `QueryUnderstanding` fields are **not** exposed in the frontend DTO — they are internal retrieval metadata only.

---

## 4. Prompt

```text
You rewrite user queries for semantic search. Given a user's question, output ONLY a JSON
object — no markdown, no explanation.

{
  "rewritten_query": "search-optimized version with specific keywords and expanded acronyms",
  "needs_multi_hop": false,
  "sub_queries": [],
  "confidence": 0.95
}

Rules:
- rewritten_query: maximize retrieval precision. Expand abbreviations. Add implied context.
  Keep the original language. Output a single line.
- needs_multi_hop: true if answering requires combining facts from 2+ distinct sources
- sub_queries: only when needs_multi_hop is true; list the independent facts needed
- confidence: a number from 0.0 to 1.0. 0.0 = the query is already optimal, no change needed.
  1.0 = the rewrite is a major improvement.

Examples:
User: "how do I pay at the inn?"
→ {"rewritten_query":"payment method ryokan cash credit card","needs_multi_hop":false,"sub_queries":[],"confidence":0.9}

User: "what did I spend in Kyoto vs Osaka?"
→ {"rewritten_query":"expenses spending Kyoto Osaka trip cost","needs_multi_hop":true,"sub_queries":["spending Kyoto trip","spending Osaka trip"],"confidence":0.85}
```

---

## 5. Error handling + fallback

| Failure | Behavior | User-visible |
|---|---|---|
| Feature flag off | `rewriter = None` | None — raw query |
| No cloud consent | `rewriter = None` | None — raw query |
| Consent denied | `pass_through(raw)` | None — raw query |
| Timeout (>3s) | `pass_through(raw)` | None — raw query |
| Network error | `pass_through(raw)` | None — raw query |
| Malformed JSON | Strip fences, try JSON; if still failing, use raw text as `rewritten_query` | Slight latency |
| Empty rewritten_query | `pass_through(raw)` | None — raw query |
| Query >512 chars | Truncated to 512 chars before rewrite | None |

**Critical invariant:** `hybrid_search` with a rewriter must return exactly the same results as without a rewriter when the rewriter fails (`is_fallback = true`). No observable difference to the user.

---

## 6. Testing strategy

### 6.1 Unit tests (`raki-ai`)

- **Happy path:** Fake LLM returns valid JSON → parsed, `is_fallback = false`.
- **Fenced JSON:** Fake LLM returns `` ```json\n{...}\n``` `` → fences stripped, parsed correctly.
- **Malformed JSON:** Fake LLM returns plain text → used as rewritten query, `is_fallback = false`.
- **Empty response:** Fake LLM returns `""` → `pass_through(raw)`.
- **Consent denied:** Fake gate returns `Denied` → `pass_through(raw)`.
- **Timeout:** Fake LLM sleeps 10s → tokio timeout → `pass_through(raw)`.
- **NaN confidence:** Fake LLM returns `"confidence": NaN` → clamped to 0.0.
- **Cache hit:** Second call with same query → returns cached result, no second LLM call.

### 6.2 Integration tests (`raki-retrieval`)

- `hybrid_search` with `Some(FakeRewriter)` → verifies rewritten query is used for embedding.
- `hybrid_search` with rewriter returning `is_fallback = true` → verifies raw query is used.

### 6.3 Eval gate (`raki-eval`)

Add `run_eval_with_rewrite`:
1. Build in-memory index from fixtures.
2. Run `hybrid_search` with **RuleBasedRewriter** (deterministic, no model).
3. Compare against baseline (`hybrid_search` without rewriter).
4. **CI gate:** no regression on any category; non-negative delta overall.

**RuleBasedRewriter** (CI-stable):
```rust
struct RuleBasedRewriter;
impl QueryRewriter for RuleBasedRewriter {
    async fn understand(&self, query: &str) -> Result<QueryUnderstanding, DomainError> {
        // Deterministic rules: "inn" → "ryokan", "spend" → "expenses cost", etc.
        // No LLM call — reproducible across CI runs.
    }
}
```

### 6.4 Live-model test (manual, `#[ignore]`)

- Runs real `CloudQueryRewriter` against fixture corpus.
- Reports per-category MAP delta.
- **Binding exit criterion:** must achieve ≥ +0.05 MAP lift on `buried-fact-in-long-note` vs no-rewriter baseline.
- Record results in `docs/eval/r3-baseline.md`.

### 6.5 Observability

- **Counter:** `query_rewrites_total` with labels `result=[success|fallback]` and `fallback_reason=[timeout|consent_denied|network_error|parse_error|empty|truncated]`.
- **Histogram:** `query_rewrite_duration_seconds` (latency from call start to response).
- **DEBUG log:** one structured log per call with `raw_query`, `rewritten_query`, `is_fallback`, `confidence`, `duration_ms`.
- **Cache metric:** `query_rewrite_cache_hits_total`.

---

## 7. Egress + privacy

- Query rewriting is a **cloud call** that sends the user's raw query text to the provider.
- The `EgressDecision` uses a synthetic `SourceId("query-rewrite")` — no note content is transmitted.
- The call goes through `GatedLlmProvider::complete_gated` → consent check + audit log.
- **Consent:** Query rewriting is a distinct processing purpose from answer generation. The Ask flow must check **separate** consent for rewriting before invoking `CloudQueryRewriter`. If rewriting consent is absent, the flow falls back to raw query and proceeds with answer-generation consent as usual.
- Search does NOT use rewriting → no egress on Search.
- **Fixture privacy:** All eval fixtures are synthetic. No real user queries or note content is sent during testing. Manual live-model tests use a dedicated test API key.

---

## 8. Exit criterion

R3 is **done** when:
1. `QueryRewriter` trait + `CloudQueryRewriter` implementation merged.
2. `hybrid_search` accepts optional rewriter; Ask wired behind feature flag, Search unwired.
3. All fallback paths unit-tested (including fenced JSON, NaN confidence, cache hit).
4. CI deterministic gate passes (RuleBasedRewriter shows non-negative delta).
5. **Live-model `CloudQueryRewriter` achieves ≥ +0.05 MAP lift on `buried-fact-in-long-note` vs no-rewriter baseline.** Recorded in `docs/eval/r3-baseline.md`.
6. Full deterministic suite green: `cargo test`, `clippy -D warnings`, `fmt --check`, `tsc --noEmit`.

---

## 9. Future work (out of scope)

- **HyDE:** Generate hypothetical document, embed it, retrieve against it.
- **Multi-hop execution:** Actually run `sub_queries` independently and union results.
- **Answer formatting:** Add `intent` classification when response templates are in scope.
- **Entity extraction:** Add `entities` field when entity-aware retrieval is scheduled.
- **Local LLM:** Add `LocalQueryRewriter` using Ollama/llama.cpp for local providers.
- **Search wiring:** Opt-in setting to enable query rewriting for Search (with debounce).
