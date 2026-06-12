//! Cloud-based query rewriter: LLM rewrites user queries for better retrieval.
//! Best-effort with timeout, cache, and graceful fallback to raw query.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use lru::LruCache;

use raki_domain::{
    CompletionRequest, DomainError, EgressDecision, EgressError, QueryRewriter, QueryUnderstanding,
    SourceId,
};

use crate::GatedLlmProvider;

// Real-world timing against kimi-k2-5: simple queries ~1.5-2s, multi-hop ~8-10s.
// 3s was far too aggressive; 15s gives headroom while still failing fast on genuine hangs.
// For lower latency, set RAKI_QUERY_REWRITE_MODEL to a cheaper/faster model than the QA model.
const REWRITE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_QUERY_LEN: usize = 512;
const MAX_PROMPT_TOKENS: u32 = 128;
const CACHE_CAPACITY: usize = 100;
const CACHE_TTL: Duration = Duration::from_secs(300);

// Tuned for speed on reasoning models: keep the prompt short and the rules explicit.
// Longer prompts caused kimi-k2-5 to take 15-30s on multi-hop queries; this version completes
// in ~1-3s while still producing correct multi-hop rewrites.
const REWRITE_SYSTEM_PROMPT: &str = r#"You rewrite user queries for semantic search. Output ONLY JSON in this exact format:
{"rewritten_query":"...","needs_multi_hop":false,"sub_queries":[],"confidence":0.0}

Rules:
- rewritten_query: add synonyms and context keywords; keep the original language; one line
- needs_multi_hop: true only if the answer requires facts from 2+ distinct sources
- sub_queries: list independent sub-questions when needs_multi_hop is true
- confidence: 0.0 if no change needed, 1.0 if major improvement

Example: "how pay at inn?" → {"rewritten_query":"payment method ryokan inn cash credit card","needs_multi_hop":false,"sub_queries":[],"confidence":0.9}"#;

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
        let effective_query = truncate_query(query);

        // Cache check
        {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((cached, ts)) = cache.get(effective_query) {
                if ts.elapsed() < CACHE_TTL {
                    return Ok(cached.clone());
                }
            }
            cache.pop(effective_query);
        }

        let decision = EgressDecision {
            provider: self.provider.clone(),
            model: self.model.clone(),
            source_ids: vec![SourceId("query-rewrite".to_string())],
            // Conservative over-estimate: characters ≈ 2-4× tokens for CJK/Latin mix.
            // Proper token counting requires a tokenizer; this is a lower-bound proxy.
            total_tokens: effective_query.chars().count(),
        };
        let req = CompletionRequest {
            system: Some(REWRITE_SYSTEM_PROMPT.into()),
            prompt: effective_query.to_string(),
            max_tokens: Some(MAX_PROMPT_TOKENS),
        };

        let start = Instant::now();
        let result =
            tokio::time::timeout(REWRITE_TIMEOUT, self.gate.complete_gated(&decision, req)).await;
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

        if !understanding.is_fallback {
            self.cache.lock().unwrap_or_else(|e| e.into_inner()).put(
                effective_query.to_string(),
                (understanding.clone(), Instant::now()),
            );
        }
        Ok(understanding)
    }
}

fn truncate_query(query: &str) -> &str {
    match query.char_indices().nth(MAX_QUERY_LEN) {
        Some((idx, _)) => &query[..idx],
        None => query,
    }
}

#[derive(serde::Deserialize)]
struct RawUnderstanding {
    rewritten_query: String,
    #[serde(default)]
    needs_multi_hop: bool,
    #[serde(default)]
    sub_queries: Vec<String>,
    #[serde(default)]
    confidence: Option<f64>,
}

fn parse_understanding(raw: &str, original: &str) -> Result<QueryUnderstanding, DomainError> {
    let text = raw.trim();

    // Strip markdown fences
    let text = text.strip_prefix("```json").unwrap_or(text);
    let text = text.strip_prefix("```").unwrap_or(text);
    let text = text.strip_suffix("```").unwrap_or(text);
    let text = text.trim();

    // Try JSON
    if let Ok(raw_u) = serde_json::from_str::<RawUnderstanding>(text) {
        let mut confidence = raw_u.confidence.unwrap_or(0.0);
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            confidence = 0.0;
        }
        let is_fallback = raw_u.rewritten_query.trim().is_empty();
        let rewritten_query = if is_fallback {
            original.to_string()
        } else {
            raw_u.rewritten_query
        };
        return Ok(QueryUnderstanding {
            rewritten_query,
            needs_multi_hop: raw_u.needs_multi_hop,
            sub_queries: raw_u.sub_queries,
            confidence,
            is_fallback,
        });
    }

    // JSON parse failed: fall back to original query
    if !text.trim().is_empty() {
        return Ok(QueryUnderstanding {
            rewritten_query: text.to_string(),
            needs_multi_hop: false,
            sub_queries: vec![],
            confidence: 0.5,
            is_fallback: true,
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
        assert!(u.is_fallback);
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
        let raw =
            r#"{"rewritten_query":"x","needs_multi_hop":false,"sub_queries":[],"confidence":null}"#;
        let u = parse_understanding(raw, "q").unwrap();
        assert_eq!(u.confidence, 0.0);
    }

    #[test]
    fn truncate_long_query() {
        let q = "a".repeat(1000);
        let truncated = truncate_query(&q);
        assert_eq!(truncated.chars().count(), MAX_QUERY_LEN);
    }

    use crate::testing::FakeLlmProvider;
    use raki_domain::testing::FixedClock;
    use raki_domain::{DomainError, EgressLog, EgressLogId, EgressRecord, EgressSettings};
    use std::collections::HashSet;
    use std::sync::Arc;

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
    impl EgressSettings for AlwaysConsented {
        async fn consented(&self) -> Result<HashSet<String>, DomainError> {
            Ok(HashSet::from(["test".to_string()]))
        }
        async fn grant(&self, _: &str) -> Result<(), DomainError> {
            Ok(())
        }
        async fn revoke(&self, _: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

    struct ConsentedTo(String);
    #[async_trait::async_trait]
    impl EgressSettings for ConsentedTo {
        async fn consented(&self) -> Result<HashSet<String>, DomainError> {
            Ok(HashSet::from([self.0.clone()]))
        }
        async fn grant(&self, _: &str) -> Result<(), DomainError> {
            Ok(())
        }
        async fn revoke(&self, _: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct NeverConsented;
    #[async_trait::async_trait]
    impl EgressSettings for NeverConsented {
        async fn consented(&self) -> Result<HashSet<String>, DomainError> {
            Ok(HashSet::new())
        }
        async fn grant(&self, _: &str) -> Result<(), DomainError> {
            Ok(())
        }
        async fn revoke(&self, _: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct NoopLog;
    #[async_trait::async_trait]
    impl EgressLog for NoopLog {
        async fn record(&self, _: &EgressRecord) -> Result<(), DomainError> {
            Ok(())
        }
        async fn set_grounded(&self, _: &EgressLogId, _: bool) -> Result<(), DomainError> {
            Ok(())
        }
        async fn list_recent(&self, _: usize) -> Result<Vec<EgressRecord>, DomainError> {
            Ok(vec![])
        }
    }

    #[tokio::test]
    async fn rewriter_returns_structured_output() {
        let rw = make_rewriter(
            r#"{"rewritten_query":"payment cash","needs_multi_hop":false,"sub_queries":[],"confidence":0.9}"#,
        );
        let u = rw.understand("how pay?").await.unwrap();
        assert_eq!(u.rewritten_query, "payment cash");
        assert!(!u.is_fallback);
    }

    #[tokio::test]
    async fn rewriter_caches_results() {
        let fake = Arc::new(FakeLlmProvider::ok(
            r#"{"rewritten_query":"cached","needs_multi_hop":false,"sub_queries":[],"confidence":0.9}"#,
        ));
        let gate = Arc::new(GatedLlmProvider::new(
            fake.clone(),
            Arc::new(AlwaysConsented),
            Arc::new(NoopLog),
            Arc::new(FixedClock(0)),
        ));
        let rw = CloudQueryRewriter::new(gate, "test".into(), "t".into());
        let u1 = rw.understand("same query").await.unwrap();
        let u2 = rw.understand("same query").await.unwrap();
        assert_eq!(u1.rewritten_query, u2.rewritten_query);
        assert_eq!(fake.call_count(), 1);
    }

    #[tokio::test]
    async fn rewriter_fallback_on_provider_error() {
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

    #[tokio::test]
    async fn rewriter_fallback_on_egress_denied() {
        let fake = Arc::new(FakeLlmProvider::ok("ignored"));
        let gate = Arc::new(GatedLlmProvider::new(
            fake,
            Arc::new(NeverConsented),
            Arc::new(NoopLog),
            Arc::new(FixedClock(0)),
        ));
        let rw = CloudQueryRewriter::new(gate, "test".into(), "t".into());
        let u = rw.understand("any").await.unwrap();
        assert!(u.is_fallback);
        assert_eq!(u.rewritten_query, "any");
    }

    #[tokio::test]
    #[ignore = "hits the real cloud endpoint; needs RAKI_LLM_BASE_URL + ANTHROPIC_API_KEY (or KIMI_API_KEY) env vars"]
    async fn live_cloud_query_rewriter_smoke() {
        use crate::MessagesProvider;

        let provider =
            std::env::var("RAKI_QUERY_REWRITE_PROVIDER").unwrap_or_else(|_| "kimi".to_string());
        let model = std::env::var("RAKI_QUERY_REWRITE_MODEL")
            .or_else(|_| std::env::var("RAKI_LLM_MODEL"))
            .unwrap_or_else(|_| "kimi-k2-5".to_string());

        let disable_thinking = provider == "kimi";
        let inner = Arc::new(
            MessagesProvider::from_env_with_options(Some(model.clone()), disable_thinking).unwrap(),
        );
        let gate = Arc::new(GatedLlmProvider::new(
            inner,
            Arc::new(ConsentedTo(provider.clone())),
            Arc::new(NoopLog),
            Arc::new(FixedClock(0)),
        ));
        let rw = CloudQueryRewriter::new(gate, provider, model);

        let u = rw
            .understand("how do I pay at the inn?")
            .await
            .expect("rewriter should not error");

        assert!(
            !u.is_fallback,
            "expected a real rewrite, got fallback: {}",
            u.rewritten_query
        );
        assert!(
            u.rewritten_query.contains("ryokan") || u.rewritten_query.contains("payment"),
            "rewrite should mention ryokan or payment; got: {}",
            u.rewritten_query
        );
    }
}
