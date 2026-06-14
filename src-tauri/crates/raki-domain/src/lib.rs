//! Raki domain kernel: pure types, value objects, and port traits. No IO, no tauri, no SQL.

pub mod body;
pub mod clock;
pub mod egress;
pub mod error;
pub mod ids;
pub mod note;
pub mod ports;
pub mod query;
pub mod testing;

pub use body::{body_to_text, text_to_body};
pub use clock::Clock;
pub use egress::{
    EgressDecision, EgressDenied, EgressError, EgressLog, EgressLogId, EgressRecord,
    EgressSettings, GatedLlmProvider, SourceId,
};
pub use error::DomainError;
pub use ids::NoteId;
pub use note::Note;
pub use ports::{
    Completion, CompletionRequest, Embedding, EmbeddingProvider, IndexingStore, KeywordHit,
    KeywordIndex, LlmProvider, Locality, MixerConfig, NoteRepository, NoteSignals, PendingNote,
    RerankScore, Reranker, SignalBooster, SignalBreakdown, SignalSource, SignalStore, VectorHit,
    VectorIndex,
};
pub use query::{QueryRewriter, QueryUnderstanding};
