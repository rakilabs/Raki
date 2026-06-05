//! Port traits. Adapters (storage, ai) implement these; services depend on them.

use async_trait::async_trait;

use crate::error::DomainError;
use crate::ids::NoteId;
use crate::note::Note;

/// Where an AI provider runs — drives the egress policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locality {
    Local,
    Cloud,
}

/// A dense embedding vector.
#[derive(Clone, Debug, PartialEq)]
pub struct Embedding(pub Vec<f32>);

#[async_trait]
pub trait NoteRepository: Send + Sync {
    async fn upsert(&self, note: &Note) -> Result<(), DomainError>;
    async fn get(&self, id: &NoteId) -> Result<Option<Note>, DomainError>;
    async fn list(&self) -> Result<Vec<Note>, DomainError>;
    async fn soft_delete(&self, id: &NoteId, at_ms: i64) -> Result<(), DomainError>;
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn dimension(&self) -> usize;
    fn locality(&self) -> Locality;
    /// Stable identifier of the model+version. Drives embedding staleness: changing
    /// it re-embeds the whole corpus.
    fn model_id(&self) -> String;
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, DomainError>;
}

// --- Ports defined for the architecture; implemented in later plans. ---

pub struct CompletionRequest {
    pub prompt: String,
}
pub struct Completion {
    pub text: String,
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn locality(&self) -> Locality;
    async fn complete(&self, req: CompletionRequest) -> Result<Completion, DomainError>;
}

pub struct VectorHit {
    pub source_id: String,
    pub distance: f32,
}

#[async_trait]
pub trait VectorIndex: Send + Sync {
    async fn upsert(&self, source_id: &str, embedding: &Embedding) -> Result<(), DomainError>;
    async fn query(&self, embedding: &Embedding, k: usize) -> Result<Vec<VectorHit>, DomainError>;
}

pub struct KeywordHit {
    pub source_id: String,
    /// FTS5 bm25 value; lower is a better match. Used by retrieval for rank ordering.
    pub score: f32,
}

#[async_trait]
pub trait KeywordIndex: Send + Sync {
    /// Best-first keyword hits for `query`, at most `k`. Read-only — index writes
    /// happen transactionally inside the repository, not here.
    async fn query(&self, query: &str, k: usize) -> Result<Vec<KeywordHit>, DomainError>;
}

/// A note awaiting (re)embedding: its id, the text to embed, and the content hash
/// that text corresponds to (used for the compare-and-stamp guard).
pub struct PendingNote {
    pub id: NoteId,
    pub text: String,
    pub content_hash: String,
}

#[async_trait]
pub trait IndexingStore: Send + Sync {
    /// One-time: populate `content_hash` for any rows missing it (pre-V3 notes).
    /// Idempotent; a no-op once every live note has a hash.
    async fn backfill_content_hashes(&self) -> Result<(), DomainError>;

    /// Live notes whose embedding is missing or stale for `model_id`, at most `limit`.
    async fn list_pending(
        &self,
        model_id: &str,
        limit: usize,
    ) -> Result<Vec<PendingNote>, DomainError>;

    /// Compare-and-stamp: mark `id` embedded for (`content_hash`, `model_id`) ONLY if
    /// the note's CURRENT content_hash still equals `content_hash`. Returns `true` if
    /// it stamped, `false` if the content changed since `content_hash` was computed
    /// (the note stays stale and re-embeds next pass — never stamp stale as current).
    async fn mark_embedded(
        &self,
        id: &NoteId,
        content_hash: &str,
        model_id: &str,
    ) -> Result<bool, DomainError>;
}
