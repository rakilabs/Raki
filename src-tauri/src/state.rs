//! Application state: the injected ports the command layer delegates to.

use std::sync::Arc;

use raki_ai::GatedLlmProvider;
use raki_domain::{
    Clock, EgressLog, EgressSettings, EmbeddingProvider, KeywordIndex, NoteRepository,
    QueryRewriter, Reranker, SignalBooster, SignalSource, SignalStore, VectorIndex,
};

use crate::indexing::IndexingService;

// signal_source and signal_booster are wired now and consumed by the retrieval
// integration in the next plan step; allow dead_code so the intermediate state
// compiles under `-D warnings`.
#[allow(dead_code)]
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
    /// The only cloud-completion path (wraps MessagesProvider; reads consent live; logs egress).
    pub gate: Arc<GatedLlmProvider>,
    /// Consent + mode mutation surface for the consent commands.
    pub settings: Arc<dyn EgressSettings>,
    /// Audit log query surface for the settings UI.
    pub egress_log: Arc<dyn EgressLog>,
    /// The cloud provider/model the egress decision is attributed to (display + consent key).
    pub provider: String,
    pub model: String,
    /// Number of top search results to retrieve for QA assembly.
    pub k: usize,
    /// Token budget for assembled context.
    pub budget_tokens: usize,
    /// Optional query rewriter (cloud LLM) for the Ask flow only.
    pub rewriter: Option<Arc<dyn QueryRewriter>>,
    /// Source of memory-lifecycle signals for retrieval ranking.
    pub signal_source: Arc<dyn SignalSource>,
    /// Mutable store for memory-lifecycle signals.
    pub signal_store: Arc<dyn SignalStore>,
    /// Multiplicative booster applied to retrieval scores using note signals.
    pub signal_booster: Arc<dyn SignalBooster>,
}
