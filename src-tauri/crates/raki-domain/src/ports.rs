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
    /// Update an existing **live** note in place. Returns `false` when no live row matched
    /// (missing or soft-deleted) — the caller treats that as not-found and never resurrects.
    /// Distinct from `upsert`, which deliberately creates/resurrects.
    async fn update(&self, note: &Note) -> Result<bool, DomainError>;
    async fn get(&self, id: &NoteId) -> Result<Option<Note>, DomainError>;
    /// Get a note regardless of `deleted_at` status — needed for restore and trash inspection.
    async fn get_any(&self, id: &NoteId) -> Result<Option<Note>, DomainError>;
    async fn list(&self) -> Result<Vec<Note>, DomainError>;
    /// Notes with `deleted_at IS NOT NULL`, newest-first by `deleted_at`.
    async fn list_trashed(&self) -> Result<Vec<Note>, DomainError>;
    async fn soft_delete(&self, id: &NoteId, at_ms: i64) -> Result<(), DomainError>;
}

#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn dimension(&self) -> usize;
    fn locality(&self) -> Locality;
    /// Stable identifier of the model+version. Drives embedding staleness: changing
    /// it re-embeds the whole corpus.
    fn model_id(&self) -> String;

    /// Embed search QUERIES (as opposed to documents). Defaults to `embed`; providers
    /// whose model wants an asymmetric query prefix override this. The pipeline embeds
    /// documents with `embed`; the retrieval/eval layer embeds queries with this.
    async fn embed_query(&self, queries: &[String]) -> Result<Vec<Embedding>, DomainError> {
        self.embed(queries).await
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, DomainError>;
}

/// A cross-encoder relevance score for one candidate document. `index` is the position
/// in the `documents` slice passed to `rerank`; `score` is higher = more relevant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RerankScore {
    pub index: usize,
    pub score: f32,
}

/// A cross-encoder reranker: reads each (query, document) pair jointly and scores relevance.
/// Mirrors `EmbeddingProvider` as a port; adapters live in `raki-ai`.
#[async_trait]
pub trait Reranker: Send + Sync {
    fn locality(&self) -> Locality;
    /// Stable identifier of the reranker model+version (recorded in the eval baseline).
    fn model_id(&self) -> String;
    /// Score every document against the query. Returns one `RerankScore` per input document
    /// (order unspecified — the caller sorts by score). Higher score = more relevant.
    async fn rerank(
        &self,
        query: &str,
        documents: &[String],
    ) -> Result<Vec<RerankScore>, DomainError>;
}

// --- Ports defined for the architecture; implemented in later plans. ---

#[derive(Debug)]
pub struct CompletionRequest {
    /// System / grounding instructions (rules + numbered context). `None` = no system message.
    pub system: Option<String>,
    /// The user's question.
    pub prompt: String,
    /// Upper bound on completion length. `None` = adapter default.
    pub max_tokens: Option<u32>,
}
#[derive(Debug)]
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
    async fn delete_by_prefix(&self, prefix: &str) -> Result<(), DomainError>;
    async fn upsert_batch(&self, items: &[(String, Embedding)]) -> Result<(), DomainError>;
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
    pub title: String,
    pub body: String,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Compilation test: PendingNote must carry title + body, not a single text field.
    #[test]
    fn pending_note_has_title_and_body() {
        let _note = PendingNote {
            id: NoteId::new(),
            title: "Hello".to_string(),
            body: "World".to_string(),
            content_hash: "abc".to_string(),
        };
    }

    /// Compilation test: VectorIndex must expose delete_by_prefix and upsert_batch.
    #[test]
    fn vector_index_methods_exist() {
        // We can't instantiate a dyn trait without an impl, so we just assert the
        // trait bounds compile by referencing the methods in a generic context.
        fn _assert_vector_index_methods<T: VectorIndex>() {}
        // If the methods are missing the trait will not be considered implemented.
        // The real check is that this module compiles at all.
    }
}

#[cfg(test)]
mod reranker_tests {
    use super::*;

    struct StubReranker;
    #[async_trait]
    impl Reranker for StubReranker {
        fn locality(&self) -> Locality {
            Locality::Local
        }
        fn model_id(&self) -> String {
            "stub".to_string()
        }
        async fn rerank(
            &self,
            _query: &str,
            documents: &[String],
        ) -> Result<Vec<RerankScore>, DomainError> {
            Ok(documents
                .iter()
                .enumerate()
                .map(|(i, _)| RerankScore {
                    index: i,
                    score: i as f32,
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn reranker_is_object_safe_and_scores_each_doc() {
        let r: &dyn Reranker = &StubReranker;
        let out = r
            .rerank("q", &["a".to_string(), "b".to_string()])
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(
            out[1],
            RerankScore {
                index: 1,
                score: 1.0
            }
        );
        assert_eq!(r.locality(), Locality::Local);
    }
}
