//! Standalone smoke test for CloudQueryRewriter.
//!
//! Usage:
//! ```bash
//! export ANTHROPIC_BASE_URL=https://api.kimi.com/coding/
//! export ANTHROPIC_API_KEY=...
//! cargo run -p raki-ai --bin rewrite_smoke -- "how do I pay at the inn?"
//! ```
//!
//! Or with Kimi-specific vars:
//! ```bash
//! export RAKI_LLM_BASE_URL=https://api.kimi.com/coding/
//! export KIMI_API_KEY=...
//! cargo run -p raki-ai --bin rewrite_smoke -- "what did I spend in Kyoto?"
//! ```

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use raki_ai::{CloudQueryRewriter, GatedLlmProvider, MessagesProvider};
use raki_domain::{
    Clock, DomainError, EgressLog, EgressLogId, EgressRecord, EgressSettings, LlmProvider,
    QueryRewriter,
};

struct ConsentedTo(HashSet<String>);

#[async_trait]
impl EgressSettings for ConsentedTo {
    async fn consented(&self) -> Result<HashSet<String>, DomainError> {
        Ok(self.0.clone())
    }
    async fn grant(&self, _: &str) -> Result<(), DomainError> {
        Ok(())
    }
    async fn revoke(&self, _: &str) -> Result<(), DomainError> {
        Ok(())
    }
}

struct NoopLog;

#[async_trait]
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

struct FixedClock(i64);

impl Clock for FixedClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "how do I pay at the inn?".to_string());

    let provider =
        std::env::var("RAKI_QUERY_REWRITE_PROVIDER").unwrap_or_else(|_| "kimi".to_string());
    let model = std::env::var("RAKI_QUERY_REWRITE_MODEL")
        .or_else(|_| std::env::var("RAKI_LLM_MODEL"))
        .unwrap_or_else(|_| "kimi-k2-5".to_string());

    // Disable Kimi K2.5 thinking mode for rewrite to match the app wiring.
    let disable_thinking = provider == "kimi";
    let inner: Arc<dyn LlmProvider> = Arc::new(MessagesProvider::from_env_with_options(
        Some(model.clone()),
        disable_thinking,
    )?);
    let gate = Arc::new(GatedLlmProvider::new(
        inner,
        Arc::new(ConsentedTo(HashSet::from([provider.clone()]))),
        Arc::new(NoopLog),
        Arc::new(FixedClock(0)),
    ));
    let rewriter = CloudQueryRewriter::new(gate, provider.clone(), model.clone());

    println!("provider: {provider}");
    println!("model:    {model}");
    println!("query:    {query}");

    let start = std::time::Instant::now();
    let u = rewriter.understand(&query).await?;

    println!("elapsed:  {:?}", start.elapsed());
    println!("fallback: {}", u.is_fallback);
    println!("conf:     {}", u.confidence);
    println!("multi:    {}", u.needs_multi_hop);
    println!("rewrite:  {}", u.rewritten_query);
    if !u.sub_queries.is_empty() {
        println!("sub-queries:");
        for sq in &u.sub_queries {
            println!("  - {sq}");
        }
    }

    Ok(())
}
