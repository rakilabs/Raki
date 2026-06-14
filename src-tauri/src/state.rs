//! Application state: the injected ports the command layer delegates to.

use std::sync::Arc;

use raki_domain::{
    Clock, EgressLog, EgressSettings, EmbeddingProvider, KeywordIndex, NoteRepository,
    QueryRewriter, Reranker, SignalBooster, SignalSource, SignalStore, VectorIndex,
};
use raki_memory::AnswerService;

use crate::indexing::IndexingService;

pub struct AppState {
    pub notes: Arc<dyn NoteRepository>,
    pub keyword: Arc<dyn KeywordIndex>,
    pub vectors: Arc<dyn VectorIndex>,
    pub embedder: Arc<dyn EmbeddingProvider>,
    /// Optional local cross-encoder reranker (attach-to-validate, ADR-0008). `None` degrades
    /// search to hybrid-only; best-effort, never required for search to work.
    pub reranker: Option<Arc<dyn Reranker>>,
    pub clock: Arc<dyn Clock>,
    pub index: Arc<IndexingService>,
    /// Per-provider consent mutation surface for the consent commands.
    pub settings: Arc<dyn EgressSettings>,
    /// Audit log query surface for the settings UI.
    pub egress_log: Arc<dyn EgressLog>,
    /// Grounded answer orchestration service.
    pub answer_service: Arc<AnswerService>,
    /// Optional query rewriter (cloud LLM) for the Ask flow only.
    pub rewriter: Option<Arc<dyn QueryRewriter>>,
    /// Source of memory-lifecycle signals for retrieval ranking.
    // Wired now; consumed by the retrieval integration in the next plan step.
    #[allow(dead_code)]
    pub signal_source: Arc<dyn SignalSource>,
    /// Mutable store for memory-lifecycle signals.
    pub signal_store: Arc<dyn SignalStore>,
    /// Multiplicative booster applied to retrieval scores using note signals.
    // Wired now; consumed by the retrieval integration in the next plan step.
    #[allow(dead_code)]
    pub signal_booster: Arc<dyn SignalBooster>,
}
