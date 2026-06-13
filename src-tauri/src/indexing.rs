//! The embedding pipeline orchestration: drain stale notes through embed → upsert
//! vector → compare-and-stamp, with per-note failure isolation. Pure of Tauri so it
//! is unit-testable; `IndexingService` adds dependency injection + single-flight.

use std::sync::Arc;

use tokio::sync::Mutex;

use raki_domain::{DomainError, EmbeddingProvider, IndexingStore, VectorIndex};
use raki_memory::indexing::{embed_pending, EmbedConfig, EmbedStats};

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
        let config = EmbedConfig::default();
        embed_pending(
            self.store.as_ref(),
            self.embedder.as_ref(),
            self.vectors.as_ref(),
            &config,
            self.batch,
        )
        .await
    }

    /// Fire-and-forget a pass; if one is already in flight, do nothing. Spawns on Tauri's
    /// managed runtime (not `tokio::spawn`) so it works from the synchronous `setup` hook,
    /// which runs before any Tokio reactor is entered, as well as from async commands.
    pub fn trigger(self: &Arc<Self>) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
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
        db.call(|c| c.query_row("SELECT count(*) FROM chunk_vectors", [], |r| r.get(0)))
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

        let first = embed_pending(
            store.as_ref(),
            embedder.as_ref(),
            vectors.as_ref(),
            &EmbedConfig::default(),
            32,
        )
        .await
        .unwrap();
        assert_eq!(first.embedded, 3);
        assert_eq!(vector_count(&db).await, 3);

        // Nothing stale now → a second pass embeds nothing.
        let second = embed_pending(
            store.as_ref(),
            embedder.as_ref(),
            vectors.as_ref(),
            &EmbedConfig::default(),
            32,
        )
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
        embed_pending(
            store.as_ref(),
            embedder.as_ref(),
            vectors.as_ref(),
            &EmbedConfig::default(),
            32,
        )
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

        let again = embed_pending(
            store.as_ref(),
            embedder.as_ref(),
            vectors.as_ref(),
            &EmbedConfig::default(),
            32,
        )
        .await
        .unwrap();
        assert_eq!(again.embedded, 1);
    }
}
