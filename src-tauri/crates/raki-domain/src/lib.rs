//! Raki domain kernel: pure types, value objects, and port traits. No IO, no tauri, no SQL.

pub mod clock;
pub mod error;
pub mod ids;
pub mod note;
pub mod ports;
pub mod testing;

pub use clock::Clock;
pub use error::DomainError;
pub use ids::NoteId;
pub use note::Note;
pub use ports::{
    Completion, CompletionRequest, Embedding, EmbeddingProvider, IndexingStore, KeywordHit,
    KeywordIndex, LlmProvider, Locality, NoteRepository, PendingNote, VectorHit, VectorIndex,
};
