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
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((cached, ts)) = cache.get(query) {
                if ts.elapsed() < CACHE_TTL {
                    return Ok(cached.clone());
                }
            }
            cache.pop(query);
        }

        let decision = EgressDecision {
            provider: self.provider.clone(),
            model: self.model.clone(),
            source_ids: vec![SourceId("query-rewrite".to_string())],
            // Conservative over-estimate: characters ≈ 2-4× tokens for CJK/Latin mix.
            // Proper token counting requires a tokenizer; this is a lower-bound proxy.
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

        self.cache.lock().unwrap_or_else(|e| e.into_inner()).put(query.to_string(), (understanding.clone(), Instant::now()));
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

    // JSON parse failed: use raw text as rewritten query if non-empty
    if !text.trim().is_empty() {
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
