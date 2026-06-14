//! Raki domain kernel: pure types, value objects, and port traits. No IO, no tauri, no SQL.

pub mod answer;
pub mod body;
pub mod clock;
pub mod egress;
pub mod error;
pub mod groundedness;
pub mod ids;
pub mod note;
pub mod ports;
pub mod query;
pub mod testing;

pub use answer::{Answer, AnswerState, EgressPreview};
pub use body::{assign_block_ids, body_to_blocks, body_to_text, normalize_body, text_to_body};
pub use clock::Clock;
pub use egress::{
    EgressDecision, EgressDenied, EgressError, EgressLog, EgressLogId, EgressRecord,
    EgressSettings, GatedLlmProvider, SourceId,
};
pub use error::DomainError;
pub use groundedness::evaluate;
pub use ids::NoteId;
pub use note::Note;
pub use ports::{
    Completion, CompletionRequest, Embedding, EmbeddingProvider, IndexingStore, KeywordHit,
    KeywordIndex, LlmProvider, Locality, MixerConfig, NoteRepository, NoteSignals, PendingNote,
    RerankScore, Reranker, SignalBooster, SignalBreakdown, SignalSource, SignalStore, VectorHit,
    VectorIndex,
};
pub use query::{QueryRewriter, QueryUnderstanding};
