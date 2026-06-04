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
