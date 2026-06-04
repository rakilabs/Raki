//! Application state: the injected ports the command layer delegates to.

use std::sync::Arc;

use raki_ai::EgressPolicy;
use raki_domain::{Clock, EmbeddingProvider, NoteRepository};

#[allow(dead_code)]
pub struct AppState {
    pub notes: Arc<dyn NoteRepository>,
    pub clock: Arc<dyn Clock>,
    pub embedder: Arc<dyn EmbeddingProvider>,
    pub egress: EgressPolicy,
}
