//! The embedding pipeline orchestration: drain stale notes through chunk → embed →
//! upsert vector → compare-and-stamp, with per-note failure isolation.

use std::collections::HashSet;

use raki_domain::{
    DomainError, EmbeddingProvider, IndexingStore, NoteId, PendingNote, VectorIndex,
};

use crate::chunk_note;

#[derive(Clone, Copy, Debug, Default)]
pub struct EmbedConfig {
    pub use_contextual_prefix: bool,
}

/// Outcome of one drain.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EmbedStats {
    pub embedded: usize,
    pub failed: usize,
}

/// Embed every stale live note for the embedder's model, idempotently. A note whose
/// content changed mid-flight is left stale (re-embeds next call); a note that errors
/// is isolated so one bad note can't stall the batch.
pub async fn embed_pending(
    store: &dyn IndexingStore,
    embedder: &dyn EmbeddingProvider,
    vectors: &dyn VectorIndex,
    config: &EmbedConfig,
    batch: usize,
) -> Result<EmbedStats, DomainError> {
    let model_id = embedder.model_id();
    let mut embedded = 0usize;
    let mut failed: HashSet<NoteId> = HashSet::new();

    loop {
        let pending = store.list_pending(&model_id, batch).await?;
        let todo: Vec<PendingNote> = pending
            .into_iter()
            .filter(|p| !failed.contains(&p.id))
            .collect();
        if todo.is_empty() {
            break;
        }
        for note in todo {
            match embed_one(store, embedder, vectors, config, &note).await {
                Ok(true) => embedded += 1,
                Ok(false) => { /* superseded mid-flight; re-listed next loop with fresh hash */ }
                Err(_) => {
                    failed.insert(note.id);
                }
            }
        }
    }

    Ok(EmbedStats {
        embedded,
        failed: failed.len(),
    })
}

async fn embed_one(
    store: &dyn IndexingStore,
    embedder: &dyn EmbeddingProvider,
    vectors: &dyn VectorIndex,
    config: &EmbedConfig,
    note: &PendingNote,
) -> Result<bool, DomainError> {
    let chunks = chunk_note(&note.title, &note.body, config.use_contextual_prefix);
    if chunks.is_empty() {
        vectors.delete_by_prefix(&format!("{}:", note.id)).await?;
        return store
            .mark_embedded(&note.id, &note.content_hash, &embedder.model_id())
            .await;
    }

    let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    let embeddings = embedder.embed(&texts).await?;
    if embeddings.len() != chunks.len() {
        return Err(DomainError::Provider(format!(
            "embedder returned {} embeddings for {} chunks",
            embeddings.len(),
            chunks.len()
        )));
    }

    vectors.delete_by_prefix(&format!("{}:", note.id)).await?;

    let items: Vec<(String, raki_domain::Embedding)> = chunks
        .into_iter()
        .zip(embeddings)
        .map(|(chunk, emb)| (format!("{}:{}", note.id, chunk.block_id), emb))
        .collect();

    vectors.upsert_batch(&items).await?;

    // Stamp returned false: content was superseded or note was deleted mid-flight.
    // The note will re-index on the next pass; temporary zero-vector state is acceptable.
    let stamped = store
        .mark_embedded(&note.id, &note.content_hash, &embedder.model_id())
        .await?;

    Ok(stamped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use raki_domain::{
        DomainError, Embedding, EmbeddingProvider, IndexingStore, Locality, NoteId, PendingNote,
        VectorHit, VectorIndex,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    struct FakeEmbeddingProvider {
        dim: usize,
        call_count: AtomicUsize,
    }

    impl FakeEmbeddingProvider {
        fn new(dim: usize) -> Self {
            Self {
                dim,
                call_count: AtomicUsize::new(0),
            }
        }
        fn call_count(&self) -> usize {
            self.call_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EmbeddingProvider for FakeEmbeddingProvider {
        fn dimension(&self) -> usize {
            self.dim
        }
        fn locality(&self) -> Locality {
            Locality::Local
        }
        fn model_id(&self) -> String {
            format!("fake-{}", self.dim)
        }
        async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, DomainError> {
            self.call_count.fetch_add(1, Ordering::SeqCst);
            Ok(inputs
                .iter()
                .map(|s| Embedding(vec![s.len() as f32; self.dim]))
                .collect())
        }
    }

    struct FakeVectorIndex {
        upserts: AtomicUsize,
        deletes: AtomicUsize,
    }

    impl FakeVectorIndex {
        fn new() -> Self {
            Self {
                upserts: AtomicUsize::new(0),
                deletes: AtomicUsize::new(0),
            }
        }
        fn upsert_count(&self) -> usize {
            self.upserts.load(Ordering::SeqCst)
        }
        fn delete_count(&self) -> usize {
            self.deletes.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl VectorIndex for FakeVectorIndex {
        async fn upsert(
            &self,
            _source_id: &str,
            _embedding: &Embedding,
        ) -> Result<(), DomainError> {
            self.upserts.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn query(
            &self,
            _embedding: &Embedding,
            _k: usize,
        ) -> Result<Vec<VectorHit>, DomainError> {
            Ok(vec![])
        }
        async fn delete_by_prefix(&self, _prefix: &str) -> Result<(), DomainError> {
            self.deletes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn upsert_batch(&self, items: &[(String, Embedding)]) -> Result<(), DomainError> {
            self.upserts.fetch_add(items.len(), Ordering::SeqCst);
            Ok(())
        }
    }

    struct FakeIndexingStore {
        stamp_result: Mutex<bool>,
    }

    #[async_trait]
    impl IndexingStore for FakeIndexingStore {
        async fn backfill_content_hashes(&self) -> Result<(), DomainError> {
            Ok(())
        }
        async fn list_pending(
            &self,
            _model_id: &str,
            _limit: usize,
        ) -> Result<Vec<PendingNote>, DomainError> {
            Ok(vec![])
        }
        async fn mark_embedded(
            &self,
            _id: &NoteId,
            _content_hash: &str,
            _model_id: &str,
        ) -> Result<bool, DomainError> {
            Ok(*self.stamp_result.lock().unwrap())
        }
    }

    #[tokio::test]
    async fn embed_one_chunks_and_upserts_batch() {
        let embedder = FakeEmbeddingProvider::new(4);
        let vectors = FakeVectorIndex::new();
        let store = FakeIndexingStore {
            stamp_result: Mutex::new(true),
        };

        let note = PendingNote {
            id: NoteId::new(),
            title: "My Note".to_string(),
            body: r#"{"type":"doc","content":[
                {"type":"paragraph","content":[{"type":"text","text":"first paragraph"}]},
                {"type":"paragraph","content":[{"type":"text","text":"second paragraph"}]}
            ]}"#
            .to_string(),
            content_hash: "abc".to_string(),
        };

        let config = EmbedConfig {
            use_contextual_prefix: false,
        };

        let result = embed_one(&store, &embedder, &vectors, &config, &note)
            .await
            .unwrap();
        assert!(result);

        // 2 paragraphs → 2 chunks → 2 upserts
        assert_eq!(vectors.upsert_count(), 2, "expected 2 chunk upserts");
        // delete_by_prefix called once before upsert
        assert_eq!(vectors.delete_count(), 1, "expected 1 delete_by_prefix");
        // embedder called once with batch of 2
        assert_eq!(embedder.call_count(), 1, "expected 1 batch embed call");
    }

    #[tokio::test]
    async fn embed_one_skips_empty_chunks() {
        let embedder = FakeEmbeddingProvider::new(4);
        let vectors = FakeVectorIndex::new();
        let store = FakeIndexingStore {
            stamp_result: Mutex::new(true),
        };

        let note = PendingNote {
            id: NoteId::new(),
            title: "".to_string(),
            body: "".to_string(),
            content_hash: "hash".to_string(),
        };

        let config = EmbedConfig::default();

        let result = embed_one(&store, &embedder, &vectors, &config, &note)
            .await
            .unwrap();
        assert!(result);

        // No embedder call because chunks are empty
        assert_eq!(
            embedder.call_count(),
            0,
            "expected no embedder call for empty chunks"
        );
        // delete_by_prefix called once for cleanup
        assert_eq!(vectors.delete_count(), 1, "expected 1 delete_by_prefix");
        // No upserts
        assert_eq!(vectors.upsert_count(), 0, "expected no upserts");
    }

    #[tokio::test]
    async fn embed_one_handles_stamp_false() {
        let embedder = FakeEmbeddingProvider::new(4);
        let vectors = FakeVectorIndex::new();
        let store = FakeIndexingStore {
            stamp_result: Mutex::new(false),
        };

        let note = PendingNote {
            id: NoteId::new(),
            title: "My Note".to_string(),
            body: r#"{"type":"doc","content":[
                {"type":"paragraph","content":[{"type":"text","text":"hello"}]}
            ]}"#
            .to_string(),
            content_hash: "hash".to_string(),
        };

        let config = EmbedConfig::default();

        let result = embed_one(&store, &embedder, &vectors, &config, &note)
            .await
            .unwrap();
        assert!(!result);

        // One delete before upsert; no compensating delete after stamp failure
        assert_eq!(
            vectors.delete_count(),
            1,
            "expected exactly 1 delete_by_prefix"
        );
        assert_eq!(vectors.upsert_count(), 1, "expected 1 upsert");
        assert_eq!(embedder.call_count(), 1, "expected 1 embedder call");
    }
}
