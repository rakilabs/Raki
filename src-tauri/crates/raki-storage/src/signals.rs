//! SQLite-backed signal storage.

use async_trait::async_trait;
use rusqlite::params;
use std::collections::HashMap;

use raki_domain::{DomainError, NoteId, NoteSignals, SignalSource, SignalStore};

use crate::db::Database;

pub struct SqliteSignalSource {
    db: Database,
}

impl SqliteSignalSource {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SignalSource for SqliteSignalSource {
    async fn get(&self, note_ids: &[NoteId]) -> Result<HashMap<NoteId, NoteSignals>, DomainError> {
        let ids: Vec<String> = note_ids.iter().map(|id| id.to_string()).collect();
        self.db
            .call(move |c| {
                let mut map = HashMap::new();
                for id in &ids {
                    let note_id = NoteId::parse(id)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

                    let row = c.query_row(
                        "SELECT n.pinned, s.view_count, s.last_accessed_at
                         FROM notes n
                         LEFT JOIN note_signals s
                            ON s.note_id = n.id AND s.deleted_at IS NULL
                         WHERE n.id = ?1 AND n.deleted_at IS NULL",
                        params![id],
                        |r| {
                            Ok((
                                r.get::<_, i64>(0)?,
                                r.get::<_, Option<i64>>(1)?,
                                r.get::<_, Option<i64>>(2)?,
                            ))
                        },
                    );

                    match row {
                        Ok((pinned, view_count, last_accessed_at_ms)) => {
                            map.insert(
                                note_id,
                                NoteSignals {
                                    pinned: pinned != 0,
                                    view_count: view_count.unwrap_or(0) as u64,
                                    last_accessed_at_ms,
                                },
                            );
                        }
                        Err(rusqlite::Error::QueryReturnedNoRows) => {
                            map.insert(note_id, NoteSignals::default());
                        }
                        Err(e) => return Err(e),
                    }
                }
                Ok(map)
            })
            .await
    }
}

pub struct SqliteSignalStore {
    db: Database,
}

impl SqliteSignalStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn not_found_err() -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(Box::new(DomainError::NotFound))
}

#[async_trait]
impl SignalStore for SqliteSignalStore {
    async fn record_view(&self, note_id: &NoteId, now_ms: i64) -> Result<(), DomainError> {
        let id = note_id.to_string();
        self.db
            .call(move |c| {
                let tx = c.unchecked_transaction()?;
                let live: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM notes WHERE id = ?1 AND deleted_at IS NULL)",
                    params![id],
                    |r| r.get(0),
                )?;
                if !live {
                    return Err(not_found_err());
                }
                tx.execute(
                    "INSERT INTO note_signals (id, note_id, view_count, last_accessed_at, created_at, updated_at, version)
                     VALUES (lower(hex(randomblob(16))), ?1, 1, ?2, ?2, ?2, 1)
                     ON CONFLICT(note_id) DO UPDATE SET
                        view_count = CASE
                            WHEN note_signals.last_accessed_at IS NULL
                              OR excluded.last_accessed_at - note_signals.last_accessed_at > 60000
                            THEN note_signals.view_count + 1
                            ELSE note_signals.view_count
                        END,
                        last_accessed_at = excluded.last_accessed_at,
                        updated_at = excluded.updated_at,
                        version = version + 1",
                    params![id, now_ms],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    async fn touch(&self, note_id: &NoteId, now_ms: i64) -> Result<(), DomainError> {
        let id = note_id.to_string();
        self.db
            .call(move |c| {
                let tx = c.unchecked_transaction()?;
                let live: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM notes WHERE id = ?1 AND deleted_at IS NULL)",
                    params![id],
                    |r| r.get(0),
                )?;
                if !live {
                    return Err(not_found_err());
                }
                tx.execute(
                    "INSERT INTO note_signals (id, note_id, view_count, last_accessed_at, created_at, updated_at, version)
                     VALUES (lower(hex(randomblob(16))), ?1, 0, ?2, ?2, ?2, 1)
                     ON CONFLICT(note_id) DO UPDATE SET
                        last_accessed_at = excluded.last_accessed_at,
                        updated_at = excluded.updated_at,
                        version = version + 1",
                    params![id, now_ms],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::notes::SqliteNoteRepository;
    use raki_domain::{DomainError, Note, NoteId, NoteRepository, SignalSource, SignalStore};

    fn sample(id: NoteId, title: &str) -> Note {
        Note {
            id,
            title: title.to_string(),
            body: "{}".to_string(),
            created_at: 1000,
            updated_at: 1000,
            deleted_at: None,
            version: 1,
        }
    }

    #[tokio::test]
    async fn record_view_increments_and_sets_last_accessed() {
        let db = Database::open_in_memory().unwrap();
        let notes = SqliteNoteRepository::new(db.clone());
        let store = SqliteSignalStore::new(db.clone());
        let source = SqliteSignalSource::new(db.clone());
        let id = NoteId::new();
        notes.upsert(&sample(id, "T")).await.unwrap();

        // More than 60 seconds apart so both calls increment the counter.
        store.record_view(&id, 0).await.unwrap();
        store.record_view(&id, 61_000).await.unwrap();

        let signals = source.get(&[id]).await.unwrap();
        let s = signals.get(&id).unwrap();
        assert_eq!(s.view_count, 2);
        assert_eq!(s.last_accessed_at_ms, Some(61_000));
    }

    #[tokio::test]
    async fn record_view_rate_limits_within_one_minute() {
        let db = Database::open_in_memory().unwrap();
        let notes = SqliteNoteRepository::new(db.clone());
        let store = SqliteSignalStore::new(db.clone());
        let source = SqliteSignalSource::new(db.clone());
        let id = NoteId::new();
        notes.upsert(&sample(id, "T")).await.unwrap();

        store.record_view(&id, 0).await.unwrap();
        store.record_view(&id, 30_000).await.unwrap();
        store.record_view(&id, 60_000).await.unwrap();

        let s = source.get(&[id]).await.unwrap().remove(&id).unwrap();
        assert_eq!(
            s.view_count, 1,
            "only the first view within one minute counts"
        );
        assert_eq!(
            s.last_accessed_at_ms,
            Some(60_000),
            "last_accessed_at still updates"
        );
    }

    #[tokio::test]
    async fn record_view_rejects_deleted_note() {
        let db = Database::open_in_memory().unwrap();
        let notes = SqliteNoteRepository::new(db.clone());
        let store = SqliteSignalStore::new(db.clone());
        let id = NoteId::new();
        notes.upsert(&sample(id, "T")).await.unwrap();
        notes.soft_delete(&id, 3000).await.unwrap();

        let err = store.record_view(&id, 5000).await.unwrap_err();
        assert!(err.to_string().contains("not found") || err.to_string().contains("deleted"));
    }

    #[tokio::test]
    async fn touch_updates_last_accessed_without_incrementing_view_count() {
        let db = Database::open_in_memory().unwrap();
        let notes = SqliteNoteRepository::new(db.clone());
        let store = SqliteSignalStore::new(db.clone());
        let source = SqliteSignalSource::new(db.clone());
        let id = NoteId::new();
        notes.upsert(&sample(id, "T")).await.unwrap();

        store.record_view(&id, 5000).await.unwrap();
        store.touch(&id, 6000).await.unwrap();

        let s = source.get(&[id]).await.unwrap().remove(&id).unwrap();
        assert_eq!(s.view_count, 1);
        assert_eq!(s.last_accessed_at_ms, Some(6000));
    }

    #[tokio::test]
    async fn touch_rejects_deleted_note() {
        let db = Database::open_in_memory().unwrap();
        let notes = SqliteNoteRepository::new(db.clone());
        let store = SqliteSignalStore::new(db.clone());
        let id = NoteId::new();
        notes.upsert(&sample(id, "T")).await.unwrap();
        notes.soft_delete(&id, 3000).await.unwrap();

        let err = store.touch(&id, 5000).await.unwrap_err();
        assert!(matches!(err, DomainError::NotFound));
    }

    #[tokio::test]
    async fn get_returns_defaults_for_missing_id() {
        let db = Database::open_in_memory().unwrap();
        let source = SqliteSignalSource::new(db);
        let id = NoteId::new();

        let s = source.get(&[id]).await.unwrap().remove(&id).unwrap();
        assert_eq!(s, NoteSignals::default());
    }

    #[tokio::test]
    async fn get_returns_pinned_true_for_pinned_note() {
        let db = Database::open_in_memory().unwrap();
        let notes = SqliteNoteRepository::new(db.clone());
        let source = SqliteSignalSource::new(db.clone());
        let id = NoteId::new();
        notes.upsert(&sample(id, "T")).await.unwrap();
        let id_str = id.to_string();
        db.call(move |c| {
            c.execute("UPDATE notes SET pinned = 1 WHERE id = ?1", params![id_str])?;
            Ok(())
        })
        .await
        .unwrap();

        let s = source.get(&[id]).await.unwrap().remove(&id).unwrap();
        assert!(s.pinned);
        assert_eq!(s.view_count, 0);
        assert_eq!(s.last_accessed_at_ms, None);
    }

    #[tokio::test]
    async fn get_returns_stored_view_count_and_last_accessed() {
        let db = Database::open_in_memory().unwrap();
        let notes = SqliteNoteRepository::new(db.clone());
        let store = SqliteSignalStore::new(db.clone());
        let source = SqliteSignalSource::new(db.clone());
        let id = NoteId::new();
        notes.upsert(&sample(id, "T")).await.unwrap();

        // Spaced apart so both calls increment despite the one-minute rate limit.
        store.record_view(&id, 0).await.unwrap();
        store.record_view(&id, 61_000).await.unwrap();

        let s = source.get(&[id]).await.unwrap().remove(&id).unwrap();
        assert_eq!(s.view_count, 2);
        assert_eq!(s.last_accessed_at_ms, Some(61_000));
    }
}
