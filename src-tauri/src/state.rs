//! Application state: the injected ports the command layer delegates to.

use std::sync::Arc;

use raki_ai::GatedLlmProvider;
use raki_domain::{
    Clock, EgressSettings, EmbeddingProvider, KeywordIndex, NoteRepository, VectorIndex,
};

use crate::indexing::IndexingService;

pub struct AppState {
    pub notes: Arc<dyn NoteRepository>,
    pub keyword: Arc<dyn KeywordIndex>,
    pub vectors: Arc<dyn VectorIndex>,
    pub embedder: Arc<dyn EmbeddingProvider>,
    pub clock: Arc<dyn Clock>,
    pub index: Arc<IndexingService>,
    /// The only cloud-completion path (wraps MessagesProvider; reads consent live; logs egress).
    pub gate: Arc<GatedLlmProvider>,
    /// Consent + mode mutation surface for the consent commands.
    pub settings: Arc<dyn EgressSettings>,
    /// The cloud provider/model the egress decision is attributed to (display + consent key).
    pub provider: String,
    pub model: String,
    /// Number of top search results to retrieve for QA assembly.
    pub k: usize,
    /// Token budget for assembled context.
    pub budget_tokens: usize,
}
