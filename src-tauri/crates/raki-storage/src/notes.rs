//! The SQLite-backed NoteRepository. The only place note SQL lives.

use async_trait::async_trait;
use rusqlite::params;

use raki_domain::{DomainError, Note, NoteId, NoteRepository};

use crate::db::Database;

pub struct SqliteNoteRepository {
    db: Database,
}

impl SqliteNoteRepository {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

fn row_to_note(row: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    let id_str: String = row.get("id")?;
    let id =
        NoteId::parse(&id_str).map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
    Ok(Note {
        id,
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
                let tx = c.unchecked_transaction()?;
                tx.execute(
                    "INSERT INTO notes (id, title, body, created_at, updated_at, deleted_at, version)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                     ON CONFLICT(id) DO UPDATE SET
                        title = ?2, body = ?3, updated_at = ?5, deleted_at = ?6, version = ?7",
                    params![id, n.title, n.body, n.created_at, n.updated_at, n.deleted_at, n.version],
                )?;
                // FTS5 has no UPDATE; refresh the row by delete+insert. Only index live notes.
                tx.execute("DELETE FROM notes_fts WHERE note_id = ?1", params![id])?;
                if n.deleted_at.is_none() {
                    tx.execute(
                        "INSERT INTO notes_fts (note_id, title, body) VALUES (?1, ?2, ?3)",
                        params![id, n.title, n.body],
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

    async fn soft_delete(&self, id: &NoteId, at_ms: i64) -> Result<(), DomainError> {
        let id_str = id.to_string();
        self.db
            .call(move |c| {
                let tx = c.unchecked_transaction()?;
                let changed = tx.execute(
                    "UPDATE notes SET deleted_at = ?2, version = version + 1
                     WHERE id = ?1 AND deleted_at IS NULL",
                    params![id_str, at_ms],
                )?;
                if changed > 0 {
                    tx.execute("DELETE FROM notes_fts WHERE note_id = ?1", params![id_str])?;
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
}
