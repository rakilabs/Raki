//! Forward-only migrations tracked by `PRAGMA user_version`. Never edit a shipped
//! migration — append a new one.

use rusqlite::Connection;

const MIGRATIONS: &[&str] = &[
    // V1: notes
    "CREATE TABLE notes (
        id TEXT PRIMARY KEY,
        title TEXT NOT NULL,
        body TEXT NOT NULL,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        deleted_at INTEGER,
        version INTEGER NOT NULL
    ) STRICT;
    CREATE INDEX idx_notes_updated ON notes(updated_at) WHERE deleted_at IS NULL;",
    // V2: full-text search over live notes. Kept in sync transactionally by the repository.
    "CREATE VIRTUAL TABLE notes_fts USING fts5(
        note_id UNINDEXED,
        title,
        body,
        tokenize = 'unicode61'
    );
    INSERT INTO notes_fts (note_id, title, body)
        SELECT id, title, body FROM notes WHERE deleted_at IS NULL;",
];

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as i64;
        if current < version {
            // Migration SQL and the version bump commit (or roll back) together.
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", version)?;
            tx.commit()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::db::Database;

    #[tokio::test]
    async fn migration_creates_fts_table() {
        let db = Database::open_in_memory().unwrap();
        // open_in_memory runs migrate(); notes_fts must exist and be queryable.
        let count: i64 = db
            .call(|c| c.query_row("SELECT count(*) FROM notes_fts", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
