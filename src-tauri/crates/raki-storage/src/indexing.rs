//! The note-side of the embedding pipeline: which notes need (re)embedding, and the
//! compare-and-stamp that records an embedding without clobbering newer content.

use async_trait::async_trait;
use rusqlite::params;

use raki_domain::{body_to_text, DomainError, IndexingStore, NoteId, PendingNote};

use crate::db::Database;
use crate::hash::content_hash;
use crate::notes::note_id_from_row;

pub struct SqliteIndexingStore {
    db: Database,
}

impl SqliteIndexingStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl IndexingStore for SqliteIndexingStore {
    async fn backfill_content_hashes(&self) -> Result<(), DomainError> {
        self.db
            .call(|c| {
                let tx = c.unchecked_transaction()?;
                let rows: Vec<(String, String, String)> = {
                    let mut stmt = tx.prepare(
                        "SELECT id, title, body FROM notes
                         WHERE content_hash IS NULL AND deleted_at IS NULL",
                    )?;
                    let collected = stmt
                        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                        .collect::<rusqlite::Result<Vec<_>>>()?;
                    collected
                };
                for (id, title, body) in rows {
                    let hash = content_hash(&title, &body);
                    tx.execute(
                        "UPDATE notes SET content_hash = ?2 WHERE id = ?1",
                        params![id, hash],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }

    async fn list_pending(
        &self,
        model_id: &str,
        limit: usize,
    ) -> Result<Vec<PendingNote>, DomainError> {
        let model = model_id.to_string();
        self.db
            .call(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT id, title, body, content_hash
                     FROM notes
                     WHERE deleted_at IS NULL
                       AND content_hash IS NOT NULL
                       AND (embedded_hash IS NULL
                            OR embedded_hash != content_hash
                            OR embedded_model IS NULL
                            OR embedded_model != ?1)
                     LIMIT ?2",
                )?;
                let rows = stmt
                    .query_map(params![model, limit as i64], |row| {
                        let id: String = row.get(0)?;
                        let title: String = row.get(1)?;
                        let body: String = row.get(2)?;
                        let content_hash: String = row.get(3)?;
                        Ok((id, title, body, content_hash))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;

                rows.into_iter()
                    .map(|(id, title, body, content_hash)| {
                        Ok(PendingNote {
                            id: note_id_from_row(&id)?,
                            text: format!("{title}\n\n{}", body_to_text(&body)),
                            content_hash,
                        })
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn mark_embedded(
        &self,
        id: &NoteId,
        content_hash: &str,
        model_id: &str,
    ) -> Result<bool, DomainError> {
        let id = id.to_string();
        let hash = content_hash.to_string();
        let model = model_id.to_string();
        self.db
            .call(move |c| {
                // Compare-and-stamp: only mark clean if content still matches the hash
                // we actually embedded.
                let affected = c.execute(
                    "UPDATE notes SET embedded_hash = ?2, embedded_model = ?3
                     WHERE id = ?1 AND content_hash = ?2 AND deleted_at IS NULL",
                    params![id, hash, model],
                )?;
                Ok(affected == 1)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{IndexingStore, Note, NoteId, NoteRepository};

    use crate::db::Database;
    use crate::notes::SqliteNoteRepository;

    const MODEL: &str = "test-model";

    async fn seed(db: &Database, title: &str) -> NoteId {
        let repo = SqliteNoteRepository::new(db.clone());
        let id = NoteId::new();
        repo.upsert(&Note::new(title.to_string(), "body".to_string(), 1000))
            .await
            .unwrap();
        // Note::new generates its own id; fetch the one actually stored via list.
        let _ = id;
        repo.list().await.unwrap()[0].id
    }

    #[tokio::test]
    async fn lists_pending_then_stops_after_stamp() {
        let db = Database::open_in_memory().unwrap();
        let id = seed(&db, "Hello").await;
        let store = SqliteIndexingStore::new(db.clone());

        let pending = store.list_pending(MODEL, 10).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
        let hash = pending[0].content_hash.clone();

        let stamped = store.mark_embedded(&id, &hash, MODEL).await.unwrap();
        assert!(stamped);
        assert!(store.list_pending(MODEL, 10).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn mark_embedded_is_a_noop_when_content_changed() {
        let db = Database::open_in_memory().unwrap();
        let id = seed(&db, "Hello").await;
        let store = SqliteIndexingStore::new(db.clone());
        let repo = SqliteNoteRepository::new(db.clone());

        let pending = store.list_pending(MODEL, 10).await.unwrap();
        let stale_hash = pending[0].content_hash.clone();

        // The note is edited AFTER we captured stale_hash (simulates the race).
        let mut edited = repo.get(&id).await.unwrap().unwrap();
        edited.body = "rewritten".to_string();
        repo.upsert(&edited).await.unwrap();

        let stamped = store.mark_embedded(&id, &stale_hash, MODEL).await.unwrap();
        assert!(
            !stamped,
            "must not stamp an embedding for superseded content"
        );
        assert_eq!(
            store.list_pending(MODEL, 10).await.unwrap().len(),
            1,
            "still pending"
        );
    }

    #[tokio::test]
    async fn changing_model_makes_notes_pending_again() {
        let db = Database::open_in_memory().unwrap();
        let id = seed(&db, "Hello").await;
        let store = SqliteIndexingStore::new(db.clone());

        let hash = store.list_pending(MODEL, 10).await.unwrap()[0]
            .content_hash
            .clone();
        store.mark_embedded(&id, &hash, MODEL).await.unwrap();
        assert!(store.list_pending(MODEL, 10).await.unwrap().is_empty());

        // A different model id ⇒ everything is stale again.
        assert_eq!(
            store.list_pending("other-model", 10).await.unwrap().len(),
            1
        );
    }

    #[tokio::test]
    async fn backfill_populates_missing_hashes() {
        let db = Database::open_in_memory().unwrap();
        // Insert a row directly with NULL content_hash (simulates a pre-V3 note).
        db.call(|c| {
            c.execute(
                "INSERT INTO notes (id, title, body, created_at, updated_at, deleted_at, version, content_hash)
                 VALUES ('00000000-0000-7000-8000-000000000000', 'T', 'B', 1, 1, NULL, 1, NULL)",
                [],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        let store = SqliteIndexingStore::new(db.clone());
        store.backfill_content_hashes().await.unwrap();

        let null_hashes: i64 = db
            .call(|c| {
                c.query_row(
                    "SELECT count(*) FROM notes WHERE content_hash IS NULL AND deleted_at IS NULL",
                    [],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(null_hashes, 0);
    }
}
