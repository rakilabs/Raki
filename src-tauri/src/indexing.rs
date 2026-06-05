//! The embedding pipeline orchestration: drain stale notes through embed → upsert
//! vector → compare-and-stamp, with per-note failure isolation. Pure of Tauri so it
//! is unit-testable; `IndexingService` adds dependency injection + single-flight.

use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;

use raki_domain::{
    DomainError, EmbeddingProvider, IndexingStore, NoteId, PendingNote, VectorIndex,
};

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
            match embed_one(store, embedder, vectors, &note).await {
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
    note: &PendingNote,
) -> Result<bool, DomainError> {
    let mut out = embedder.embed(std::slice::from_ref(&note.text)).await?;
    let emb = out
        .pop()
        .ok_or_else(|| DomainError::Provider("embedder returned no vector".to_string()))?;
    vectors.upsert(&note.id.to_string(), &emb).await?;
    // Stamp last: if we crash before this, the note stays stale and re-embeds.
    store
        .mark_embedded(&note.id, &note.content_hash, &embedder.model_id())
        .await
}

/// DI + single-flight wrapper around `embed_pending`. `trigger` fires a background
/// pass and silently skips if one is already running.
pub struct IndexingService {
    store: Arc<dyn IndexingStore>,
    embedder: Arc<dyn EmbeddingProvider>,
    vectors: Arc<dyn VectorIndex>,
    running: Mutex<()>,
    batch: usize,
}

impl IndexingService {
    pub fn new(
        store: Arc<dyn IndexingStore>,
        embedder: Arc<dyn EmbeddingProvider>,
        vectors: Arc<dyn VectorIndex>,
    ) -> Self {
        Self {
            store,
            embedder,
            vectors,
            running: Mutex::new(()),
            batch: 32,
        }
    }

    /// Backfill missing hashes, then drain. Used at startup and from `trigger`.
    pub async fn run_once(&self) -> Result<EmbedStats, DomainError> {
        self.store.backfill_content_hashes().await?;
        embed_pending(
            self.store.as_ref(),
            self.embedder.as_ref(),
            self.vectors.as_ref(),
            self.batch,
        )
        .await
    }

    /// Fire-and-forget a pass; if one is already in flight, do nothing.
    pub fn trigger(self: &Arc<Self>) {
        let this = self.clone();
        tokio::spawn(async move {
            let Ok(_guard) = this.running.try_lock() else {
                return; // single-flight: a pass is already running
            };
            if let Err(e) = this.run_once().await {
                eprintln!("indexing pass failed: {e}");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_ai::FakeEmbeddingProvider;
    use raki_domain::{Note, NoteRepository};
    use raki_storage::{Database, SqliteIndexingStore, SqliteNoteRepository, SqliteVectorIndex};

    async fn vector_count(db: &Database) -> i64 {
        db.call(|c| c.query_row("SELECT count(*) FROM note_vectors", [], |r| r.get(0)))
            .await
            .unwrap()
    }

    fn wiring(
        db: &Database,
    ) -> (
        Arc<dyn IndexingStore>,
        Arc<dyn EmbeddingProvider>,
        Arc<dyn VectorIndex>,
    ) {
        (
            Arc::new(SqliteIndexingStore::new(db.clone())),
            Arc::new(FakeEmbeddingProvider::new(384)),
            Arc::new(SqliteVectorIndex::new(db.clone())),
        )
    }

    #[tokio::test]
    async fn embeds_all_pending_then_is_idempotent() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        for t in ["Apples", "Oranges", "Pears"] {
            repo.upsert(&Note::new(t.to_string(), "body".to_string(), 1000))
                .await
                .unwrap();
        }
        let (store, embedder, vectors) = wiring(&db);

        let first = embed_pending(store.as_ref(), embedder.as_ref(), vectors.as_ref(), 32)
            .await
            .unwrap();
        assert_eq!(first.embedded, 3);
        assert_eq!(vector_count(&db).await, 3);

        // Nothing stale now → a second pass embeds nothing.
        let second = embed_pending(store.as_ref(), embedder.as_ref(), vectors.as_ref(), 32)
            .await
            .unwrap();
        assert_eq!(
            second,
            EmbedStats {
                embedded: 0,
                failed: 0
            }
        );
    }

    #[tokio::test]
    async fn re_embeds_only_the_edited_note() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        repo.upsert(&Note::new("Keep".to_string(), "body".to_string(), 1000))
            .await
            .unwrap();
        repo.upsert(&Note::new("Edit".to_string(), "body".to_string(), 1000))
            .await
            .unwrap();
        let (store, embedder, vectors) = wiring(&db);
        embed_pending(store.as_ref(), embedder.as_ref(), vectors.as_ref(), 32)
            .await
            .unwrap();

        // Edit one note → exactly one becomes stale.
        let mut edit = repo
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|n| n.title == "Edit")
            .unwrap();
        edit.body = "rewritten".to_string();
        repo.upsert(&edit).await.unwrap();

        let again = embed_pending(store.as_ref(), embedder.as_ref(), vectors.as_ref(), 32)
            .await
            .unwrap();
        assert_eq!(again.embedded, 1);
    }
}
