//! The SQLite-backed NoteRepository. The only place note SQL lives.

use async_trait::async_trait;
use rusqlite::params;

use raki_domain::{body_to_text, DomainError, Note, NoteId, NoteRepository};

use crate::db::Database;
use crate::hash::content_hash;

pub struct SqliteNoteRepository {
    db: Database,
}

impl SqliteNoteRepository {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

pub(crate) fn note_id_from_row(id_str: &str) -> rusqlite::Result<NoteId> {
    NoteId::parse(id_str).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))
}

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    let id_str: String = row.get("id")?;
    Ok(Note {
        id: note_id_from_row(&id_str)?,
        title: row.get("title")?,
        body: row.get("body")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        deleted_at: row.get("deleted_at")?,
        version: row.get("version")?,
    })
}

#[async_trait]
impl NoteRepository for SqliteNoteRepository {
    async fn upsert(&self, note: &Note) -> Result<(), DomainError> {
        let n = note.clone();
        self.db
            .call(move |c| {
                let id = n.id.to_string();
                let hash = content_hash(&n.title, &n.body);
                let tx = c.unchecked_transaction()?;

                tx.execute(
                    "INSERT INTO notes (id, title, body, created_at, updated_at, deleted_at, version, content_hash)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(id) DO UPDATE SET
                        title = ?2, body = ?3, updated_at = ?5, deleted_at = ?6, version = ?7, content_hash = ?8",
                    params![id, n.title, n.body, n.created_at, n.updated_at, n.deleted_at, n.version, hash],
                )?;

                // FTS5 has no UPDATE; refresh the row by delete+insert so the index always
                // reflects current content AND liveness — a resurrected note re-enters FTS.
                tx.execute("DELETE FROM notes_fts WHERE note_id = ?1", params![id])?;
                if n.deleted_at.is_none() {
                    tx.execute(
                        "INSERT INTO notes_fts (note_id, title, body) VALUES (?1, ?2, ?3)",
                        params![id, n.title, body_to_text(&n.body)],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }

    async fn get(&self, id: &NoteId) -> Result<Option<Note>, DomainError> {
        let id_str = id.to_string();
        self.db
            .call(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT id, title, body, created_at, updated_at, deleted_at, version
                     FROM notes WHERE id = ?1 AND deleted_at IS NULL",
                )?;
                let mut rows = stmt.query(params![id_str])?;
                rows.next()?.map(row_to_note).transpose()
            })
            .await
    }

    async fn get_any(&self, id: &NoteId) -> Result<Option<Note>, DomainError> {
        let id_str = id.to_string();
        self.db
            .call(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT id, title, body, created_at, updated_at, deleted_at, version
                     FROM notes WHERE id = ?1",
                )?;
                let mut rows = stmt.query(params![id_str])?;
                rows.next()?.map(row_to_note).transpose()
            })
            .await
    }

    async fn list(&self) -> Result<Vec<Note>, DomainError> {
        self.db
            .call(|c| {
                let mut stmt = c.prepare_cached(
                    "SELECT id, title, body, created_at, updated_at, deleted_at, version
                     FROM notes WHERE deleted_at IS NULL ORDER BY updated_at DESC",
                )?;
                let notes = stmt
                    .query_map([], row_to_note)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(notes)
            })
            .await
    }

    async fn list_trashed(&self) -> Result<Vec<Note>, DomainError> {
        self.db
            .call(|c| {
                let mut stmt = c.prepare_cached(
                    "SELECT id, title, body, created_at, updated_at, deleted_at, version
                     FROM notes WHERE deleted_at IS NOT NULL ORDER BY deleted_at DESC",
                )?;
                let notes = stmt
                    .query_map([], row_to_note)?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(notes)
            })
            .await
    }

    async fn update(&self, note: &Note) -> Result<bool, DomainError> {
        let n = note.clone();
        self.db
            .call(move |c| {
                let id = n.id.to_string();
                let hash = content_hash(&n.title, &n.body);
                let tx = c.unchecked_transaction()?;
                // Liveness guard: only a non-deleted row updates. A soft-deleted (or missing)
                // row matches nothing → 0 changes → false, so an edit can never resurrect.
                let affected = tx.execute(
                    "UPDATE notes
                        SET title = ?2, body = ?3, updated_at = ?4, version = ?5, content_hash = ?6
                      WHERE id = ?1 AND deleted_at IS NULL",
                    params![id, n.title, n.body, n.updated_at, n.version, hash],
                )?;
                if affected == 0 {
                    return Ok(false); // tx drops → rollback; nothing changed
                }
                // FTS5 has no UPDATE; refresh by delete+insert with flattened body.
                tx.execute("DELETE FROM notes_fts WHERE note_id = ?1", params![id])?;
                tx.execute(
                    "INSERT INTO notes_fts (note_id, title, body) VALUES (?1, ?2, ?3)",
                    params![id, n.title, body_to_text(&n.body)],
                )?;
                tx.commit()?;
                Ok(true)
            })
            .await
    }

    async fn soft_delete(&self, id: &NoteId, at_ms: i64) -> Result<(), DomainError> {
        let id_str = id.to_string();
        self.db
            .call(move |c| {
                let tx = c.unchecked_transaction()?;
                // Clear the embedding staleness keys: the vector is destroyed below, so
                // the keys must not claim it still exists (else a resurrected note with
                // unchanged content would never re-embed).
                let changed = tx.execute(
                    "UPDATE notes SET deleted_at = ?2, version = version + 1,
                        embedded_hash = NULL, embedded_model = NULL
                     WHERE id = ?1 AND deleted_at IS NULL",
                    params![id_str, at_ms],
                )?;
                if changed > 0 {
                    tx.execute("DELETE FROM notes_fts WHERE note_id = ?1", params![id_str])?;
                    tx.execute(
                        "DELETE FROM chunk_vectors WHERE chunk_id LIKE ?1",
                        params![format!("{}#%", id_str)],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{Note, NoteId, NoteRepository};

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
    async fn update_changes_a_live_note_and_refreshes_fts() {
        use raki_domain::text_to_body;
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let note = Note::new("Trip".into(), text_to_body("old"), 1000);
        repo.upsert(&note).await.unwrap();

        let new_body = text_to_body("new plan cash");
        let edited = note.edit("Trip".into(), new_body.clone(), 2000);
        assert!(repo.update(&edited).await.unwrap(), "live row updated");

        let got = repo.get(&note.id).await.unwrap().unwrap();
        assert_eq!(got.body, new_body);
        assert_eq!(got.version, 2);
        assert_eq!(
            fts_count(&db, &note.id.to_string()).await,
            1,
            "still indexed"
        );
    }

    #[tokio::test]
    async fn update_refuses_to_resurrect_a_soft_deleted_note() {
        use raki_domain::text_to_body;
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let note = Note::new("T".into(), text_to_body("x"), 1000);
        repo.upsert(&note).await.unwrap();
        repo.soft_delete(&note.id, 1500).await.unwrap();

        let edited = note.edit("T".into(), text_to_body("resurrected"), 2000);
        assert!(!repo.update(&edited).await.unwrap(), "no live row → false");
        assert!(repo.get(&note.id).await.unwrap().is_none(), "still deleted");
        assert_eq!(
            fts_count(&db, &note.id.to_string()).await,
            0,
            "not re-indexed"
        );
    }

    #[tokio::test]
    async fn upsert_then_get_roundtrips() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db);
        let id = NoteId::new();
        repo.upsert(&sample(id, "Hello")).await.unwrap();

        let got = repo.get(&id).await.unwrap().expect("note exists");
        assert_eq!(got.title, "Hello");
    }

    #[tokio::test]
    async fn list_excludes_soft_deleted() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db);
        let keep = NoteId::new();
        let gone = NoteId::new();
        repo.upsert(&sample(keep, "Keep")).await.unwrap();
        repo.upsert(&sample(gone, "Gone")).await.unwrap();

        repo.soft_delete(&gone, 2000).await.unwrap();

        let listed = repo.list().await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].title, "Keep");
    }

    #[tokio::test]
    async fn get_any_returns_deleted_notes() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db);
        let id = NoteId::new();
        repo.upsert(&sample(id, "Gone")).await.unwrap();
        repo.soft_delete(&id, 2000).await.unwrap();

        assert!(
            repo.get(&id).await.unwrap().is_none(),
            "get filters deleted"
        );
        let any = repo
            .get_any(&id)
            .await
            .unwrap()
            .expect("get_any sees deleted");
        assert_eq!(any.title, "Gone");
        assert_eq!(any.deleted_at, Some(2000));
    }

    #[tokio::test]
    async fn list_trashed_includes_only_deleted_newest_first() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db);
        let live = NoteId::new();
        let old = NoteId::new();
        let recent = NoteId::new();
        repo.upsert(&sample(live, "Live")).await.unwrap();
        repo.upsert(&sample(old, "Old")).await.unwrap();
        repo.upsert(&sample(recent, "Recent")).await.unwrap();

        repo.soft_delete(&old, 1000).await.unwrap();
        repo.soft_delete(&recent, 3000).await.unwrap();

        let trashed = repo.list_trashed().await.unwrap();
        assert_eq!(trashed.len(), 2);
        assert_eq!(trashed[0].title, "Recent");
        assert_eq!(trashed[1].title, "Old");
    }

    async fn fts_count(db: &Database, note_id: &str) -> i64 {
        let id = note_id.to_string();
        db.call(move |c| {
            c.query_row(
                "SELECT count(*) FROM notes_fts WHERE note_id = ?1",
                params![id],
                |r| r.get(0),
            )
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn upsert_indexes_into_fts() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let id = NoteId::new();
        repo.upsert(&sample(id, "Hello")).await.unwrap();
        assert_eq!(fts_count(&db, &id.to_string()).await, 1);
    }

    #[tokio::test]
    async fn soft_delete_removes_from_fts() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let id = NoteId::new();
        repo.upsert(&sample(id, "Hello")).await.unwrap();
        repo.soft_delete(&id, 2000).await.unwrap();
        assert_eq!(fts_count(&db, &id.to_string()).await, 0);
    }

    async fn content_hash_of(db: &Database, id: &str) -> Option<String> {
        let id = id.to_string();
        db.call(move |c| {
            c.query_row(
                "SELECT content_hash FROM notes WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn upsert_writes_content_hash_and_updates_on_edit() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let id = NoteId::new();

        let mut note = sample(id, "Title");
        repo.upsert(&note).await.unwrap();
        let h1 = content_hash_of(&db, &id.to_string())
            .await
            .expect("hash set");

        note.body = "different body".to_string();
        repo.upsert(&note).await.unwrap();
        let h2 = content_hash_of(&db, &id.to_string())
            .await
            .expect("hash set");

        assert_ne!(h1, h2, "editing body changes the content hash");
    }

    async fn vector_count(db: &Database, note_id: &str) -> i64 {
        let id = note_id.to_string();
        db.call(move |c| {
            c.query_row(
                "SELECT count(*) FROM chunk_vectors WHERE chunk_id LIKE ?1",
                rusqlite::params![format!("{}#%", id)],
                |r| r.get(0),
            )
        })
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn soft_delete_removes_vector() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let id = NoteId::new();
        repo.upsert(&sample(id, "Hello")).await.unwrap();

        // Insert a placeholder 384-dim vector blob directly (SqliteVectorIndex is Task 5).
        let id_str = id.to_string();
        db.call(move |c| {
            let blob = vec![0u8; 384 * 4];
            c.execute(
                "INSERT INTO chunk_vectors (chunk_id, embedding) VALUES (?1, ?2)",
                rusqlite::params![format!("{}#0", id_str), blob],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(vector_count(&db, &id.to_string()).await, 1);

        repo.soft_delete(&id, 2000).await.unwrap();
        assert_eq!(vector_count(&db, &id.to_string()).await, 0);
    }

    #[tokio::test]
    async fn fts_body_is_flattened_prose_not_json() {
        use raki_domain::text_to_body;
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let note = Note::new("Trip".into(), text_to_body("pay cash at the ryokan"), 1000);
        repo.upsert(&note).await.unwrap();
        let id = note.id.to_string();
        let fts_body: String = db
            .call(move |c| {
                c.query_row(
                    "SELECT body FROM notes_fts WHERE note_id = ?1",
                    rusqlite::params![id],
                    |r| r.get(0),
                )
            })
            .await
            .unwrap();
        assert_eq!(fts_body, "pay cash at the ryokan");
        assert!(
            !fts_body.contains("paragraph"),
            "no structural keys in the index"
        );
    }

    #[tokio::test]
    async fn resurrection_reindexes_fts_and_marks_pending() {
        use crate::indexing::SqliteIndexingStore;
        use raki_domain::IndexingStore;

        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let store = SqliteIndexingStore::new(db.clone());
        let id = NoteId::new();

        // Live + embedded: in FTS, not pending.
        repo.upsert(&sample(id, "Hello")).await.unwrap();
        let hash = store.list_pending("m", 10).await.unwrap()[0]
            .content_hash
            .clone();
        store.mark_embedded(&id, &hash, "m").await.unwrap();
        assert_eq!(fts_count(&db, &id.to_string()).await, 1);
        assert!(store.list_pending("m", 10).await.unwrap().is_empty());

        // Soft-delete drops the FTS row, the vector, and the staleness keys.
        repo.soft_delete(&id, 2000).await.unwrap();
        assert_eq!(fts_count(&db, &id.to_string()).await, 0);

        // Resurrect with identical content (deleted_at -> None): must re-enter FTS and
        // become pending again, because its vector was destroyed on delete.
        repo.upsert(&sample(id, "Hello")).await.unwrap();
        assert_eq!(
            fts_count(&db, &id.to_string()).await,
            1,
            "resurrected note must re-enter FTS"
        );
        assert_eq!(
            store.list_pending("m", 10).await.unwrap().len(),
            1,
            "resurrected note must re-embed"
        );
    }
}
