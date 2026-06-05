//! Application state: the injected ports the command layer delegates to.

use std::sync::Arc;

use raki_ai::EgressPolicy;
use raki_domain::{Clock, EmbeddingProvider, KeywordIndex, NoteRepository, VectorIndex};

use crate::indexing::IndexingService;

#[allow(dead_code)]
pub struct AppState {
    pub notes: Arc<dyn NoteRepository>,
    pub keyword: Arc<dyn KeywordIndex>,
    pub vectors: Arc<dyn VectorIndex>,
    pub embedder: Arc<dyn EmbeddingProvider>,
    pub clock: Arc<dyn Clock>,
    pub egress: EgressPolicy,
    pub index: Arc<IndexingService>,
}
