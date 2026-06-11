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
