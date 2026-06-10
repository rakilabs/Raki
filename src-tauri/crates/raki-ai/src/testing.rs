//! Reusable test doubles for the AI crate.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use raki_domain::{Completion, CompletionRequest, DomainError, LlmProvider, Locality};

/// An `LlmProvider` that returns a canned reply (or a canned error) and counts calls.
pub struct FakeLlmProvider {
    pub reply: Result<String, String>, // Ok(text) or Err(message → DomainError::Provider)
    pub calls: Arc<AtomicUsize>,
    pub locality: Locality,
}

impl FakeLlmProvider {
    pub fn ok(text: &str) -> Self {
        Self {
            reply: Ok(text.to_string()),
            calls: Arc::new(AtomicUsize::new(0)),
            locality: Locality::Cloud,
        }
    }
    pub fn failing(msg: &str) -> Self {
        Self {
            reply: Err(msg.to_string()),
            calls: Arc::new(AtomicUsize::new(0)),
            locality: Locality::Cloud,
        }
    }
    pub fn with_locality(mut self, locality: Locality) -> Self {
        self.locality = locality;
        self
    }
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for FakeLlmProvider {
    fn locality(&self) -> Locality {
        self.locality
    }
    async fn complete(&self, _req: CompletionRequest) -> Result<Completion, DomainError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.reply {
            Ok(text) => Ok(Completion { text: text.clone() }),
            Err(msg) => Err(DomainError::Provider(msg.clone())),
        }
    }
}
