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
    // V3: semantic vector index (sqlite-vec) + embedding staleness tracking on notes.
    // note_vectors is a vec0 virtual table; the embedding pipeline keeps it in sync.
    "CREATE VIRTUAL TABLE note_vectors USING vec0(
        note_id TEXT PRIMARY KEY,
        embedding float[384]
    );
    ALTER TABLE notes ADD COLUMN content_hash TEXT;
    ALTER TABLE notes ADD COLUMN embedded_hash TEXT;
    ALTER TABLE notes ADD COLUMN embedded_model TEXT;",
    // V4: egress audit log + cloud consent + a tiny settings kv (egress mode). Audit/system tables:
    // id + timestamps, no soft-delete/version (not user-data).
    "CREATE TABLE egress_log (
        id TEXT PRIMARY KEY,
        created_at INTEGER NOT NULL,
        provider TEXT NOT NULL,
        model TEXT NOT NULL,
        token_count INTEGER NOT NULL,
        source_ids TEXT NOT NULL,   -- JSON array of source id strings
        success INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE cloud_consent (
        provider TEXT PRIMARY KEY,
        granted_at INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE app_settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) STRICT;",
    // V5: groundedness verdict for a QA answer. Nullable: NULL = not a QA answer / no send.
    "ALTER TABLE egress_log ADD COLUMN grounded INTEGER;",
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
    use crate::migrations::{migrate, MIGRATIONS};

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

    #[test]
    fn v5_grounded_column_applies_to_a_populated_egress_log() {
        use crate::db::register_sqlite_vec;
        use rusqlite::Connection;

        register_sqlite_vec(); // auto-extension → vec0 (V3) resolves on a raw connection
        let conn = Connection::open_in_memory().unwrap();

        // Apply V1..V4 only, then stamp the version so migrate() resumes at V5.
        for sql in &MIGRATIONS[0..4] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 4i64).unwrap();

        // Populate egress_log BEFORE the ALTER (the point of the fixture).
        conn.execute(
            "INSERT INTO egress_log (id, created_at, provider, model, token_count, source_ids, success)
             VALUES ('row1', 1, 'kimi', 'k2', 10, '[]', 1)" ,
            [],
        )
        .unwrap();

        // Apply the remaining migration(s) — V5's ALTER runs on the populated table.
        migrate(&conn).unwrap();

        // NOTE: ADD COLUMN ... INTEGER (nullable) is a metadata-only change in SQLite — it does not
        // rewrite existing rows. This test exists to honor the project's migration contract and to
        // catch the general class (a future backfilling migration would fail here loudly).
        let pre: Option<i64> = conn
            .query_row(
                "SELECT grounded FROM egress_log WHERE id = 'row1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(pre, None, "the pre-existing row gets NULL grounded");
        conn.execute("UPDATE egress_log SET grounded = 0 WHERE id = 'row1'", [])
            .unwrap();
        let post: Option<i64> = conn
            .query_row(
                "SELECT grounded FROM egress_log WHERE id = 'row1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(post, Some(0));
    }

    #[test]
    fn migration_creates_note_vectors_and_columns() {
        let db = Database::open_in_memory().unwrap();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            // note_vectors vec0 table exists and is queryable
            let v: i64 = db
                .call(|c| c.query_row("SELECT count(*) FROM note_vectors", [], |r| r.get(0)))
                .await
                .unwrap();
            assert_eq!(v, 0);
            // the three staleness columns exist on notes
            let cols: i64 = db
                .call(|c| {
                    c.query_row(
                        "SELECT count(*) FROM pragma_table_info('notes')
                         WHERE name IN ('content_hash','embedded_hash','embedded_model')",
                        [],
                        |r| r.get(0),
                    )
                })
                .await
                .unwrap();
            assert_eq!(cols, 3);
        });
    }
}
