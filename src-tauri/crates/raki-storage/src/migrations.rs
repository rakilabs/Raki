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
];

pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    for (i, sql) in MIGRATIONS.iter().enumerate() {
        let version = (i + 1) as i64;
        if current < version {
            conn.execute_batch(&format!(
                "BEGIN; {sql} PRAGMA user_version = {version}; COMMIT;"
            ))?;
        }
    }
    Ok(())
}
