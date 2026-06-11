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
    // V6: the body flattener changed (space-join → newline-join, and "{}" → empty), so the
    // text we embed changed for every note while content_hash (over raw body) did not.
    // Clear the staleness stamp to force a clean re-embed of the whole corpus on next start.
    // Also rebuild notes_fts so pre-existing notes are indexed with the new flattened text.
    "UPDATE notes SET embedded_hash = NULL;
     DELETE FROM notes_fts WHERE note_id IN (SELECT id FROM notes WHERE deleted_at IS NULL);
     INSERT INTO notes_fts (note_id, title, body)
         SELECT id, title, CASE WHEN body = '{}' THEN '' ELSE body END
         FROM notes WHERE deleted_at IS NULL;",
    // V7: chunk-level vector index. Creates chunk_vectors alongside note_vectors (preserved
    // as stale backup). Clears embedded_hash so the background indexer re-chunks and
    // re-embeds the entire corpus on next start.
    "CREATE VIRTUAL TABLE chunk_vectors USING vec0(
        chunk_id TEXT PRIMARY KEY,
        embedding float[384]
    );
    UPDATE notes SET embedded_hash = NULL;",
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
    fn v6_re_embed_clears_embedded_hash_on_populated_notes() {
        use crate::db::register_sqlite_vec;
        use rusqlite::Connection;

        register_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();
        // Apply V1..V5, then stamp so migrate() resumes at V6.
        for sql in &MIGRATIONS[0..5] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 5i64).unwrap();
        // A note that was already embedded (embedded_hash set) BEFORE the migration.
        conn.execute(
            "INSERT INTO notes (id, title, body, created_at, updated_at, deleted_at, version, content_hash, embedded_hash, embedded_model)
             VALUES ('n1', 'T', '{}', 1, 1, NULL, 1, 'h', 'h', 'm')",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap(); // applies V6

        let embedded_hash: Option<String> = conn
            .query_row("SELECT embedded_hash FROM notes WHERE id = 'n1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            embedded_hash, None,
            "V6 clears the staleness stamp → note re-lists as pending"
        );

        // V6 also rebuilds notes_fts: the '{}' body must be flattened to '' in the index.
        let fts_body: String = conn
            .query_row("SELECT body FROM notes_fts WHERE note_id = 'n1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(fts_body, "");
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

    #[test]
    fn v7_creates_chunk_vectors_without_dropping_note_vectors() {
        use crate::db::register_sqlite_vec;
        use rusqlite::Connection;

        register_sqlite_vec();
        let conn = Connection::open_in_memory().unwrap();

        // Apply V1..V6, then stamp so migrate() resumes at V7.
        for sql in &MIGRATIONS[0..6] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 6i64).unwrap();

        // Populate notes and note_vectors BEFORE the migration.
        conn.execute(
            "INSERT INTO notes (id, title, body, created_at, updated_at, deleted_at, version, content_hash, embedded_hash, embedded_model)
             VALUES ('n1', 'T', 'b', 1, 1, NULL, 1, 'h', 'h', 'm')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO note_vectors (note_id, embedding)
             VALUES ('n1', vec_f32('[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0, 21.0, 22.0, 23.0, 24.0, 25.0, 26.0, 27.0, 28.0, 29.0, 30.0, 31.0, 32.0, 33.0, 34.0, 35.0, 36.0, 37.0, 38.0, 39.0, 40.0, 41.0, 42.0, 43.0, 44.0, 45.0, 46.0, 47.0, 48.0, 49.0, 50.0, 51.0, 52.0, 53.0, 54.0, 55.0, 56.0, 57.0, 58.0, 59.0, 60.0, 61.0, 62.0, 63.0, 64.0, 65.0, 66.0, 67.0, 68.0, 69.0, 70.0, 71.0, 72.0, 73.0, 74.0, 75.0, 76.0, 77.0, 78.0, 79.0, 80.0, 81.0, 82.0, 83.0, 84.0, 85.0, 86.0, 87.0, 88.0, 89.0, 90.0, 91.0, 92.0, 93.0, 94.0, 95.0, 96.0, 97.0, 98.0, 99.0, 100.0, 101.0, 102.0, 103.0, 104.0, 105.0, 106.0, 107.0, 108.0, 109.0, 110.0, 111.0, 112.0, 113.0, 114.0, 115.0, 116.0, 117.0, 118.0, 119.0, 120.0, 121.0, 122.0, 123.0, 124.0, 125.0, 126.0, 127.0, 128.0, 129.0, 130.0, 131.0, 132.0, 133.0, 134.0, 135.0, 136.0, 137.0, 138.0, 139.0, 140.0, 141.0, 142.0, 143.0, 144.0, 145.0, 146.0, 147.0, 148.0, 149.0, 150.0, 151.0, 152.0, 153.0, 154.0, 155.0, 156.0, 157.0, 158.0, 159.0, 160.0, 161.0, 162.0, 163.0, 164.0, 165.0, 166.0, 167.0, 168.0, 169.0, 170.0, 171.0, 172.0, 173.0, 174.0, 175.0, 176.0, 177.0, 178.0, 179.0, 180.0, 181.0, 182.0, 183.0, 184.0, 185.0, 186.0, 187.0, 188.0, 189.0, 190.0, 191.0, 192.0, 193.0, 194.0, 195.0, 196.0, 197.0, 198.0, 199.0, 200.0, 201.0, 202.0, 203.0, 204.0, 205.0, 206.0, 207.0, 208.0, 209.0, 210.0, 211.0, 212.0, 213.0, 214.0, 215.0, 216.0, 217.0, 218.0, 219.0, 220.0, 221.0, 222.0, 223.0, 224.0, 225.0, 226.0, 227.0, 228.0, 229.0, 230.0, 231.0, 232.0, 233.0, 234.0, 235.0, 236.0, 237.0, 238.0, 239.0, 240.0, 241.0, 242.0, 243.0, 244.0, 245.0, 246.0, 247.0, 248.0, 249.0, 250.0, 251.0, 252.0, 253.0, 254.0, 255.0, 256.0, 257.0, 258.0, 259.0, 260.0, 261.0, 262.0, 263.0, 264.0, 265.0, 266.0, 267.0, 268.0, 269.0, 270.0, 271.0, 272.0, 273.0, 274.0, 275.0, 276.0, 277.0, 278.0, 279.0, 280.0, 281.0, 282.0, 283.0, 284.0, 285.0, 286.0, 287.0, 288.0, 289.0, 290.0, 291.0, 292.0, 293.0, 294.0, 295.0, 296.0, 297.0, 298.0, 299.0, 300.0, 301.0, 302.0, 303.0, 304.0, 305.0, 306.0, 307.0, 308.0, 309.0, 310.0, 311.0, 312.0, 313.0, 314.0, 315.0, 316.0, 317.0, 318.0, 319.0, 320.0, 321.0, 322.0, 323.0, 324.0, 325.0, 326.0, 327.0, 328.0, 329.0, 330.0, 331.0, 332.0, 333.0, 334.0, 335.0, 336.0, 337.0, 338.0, 339.0, 340.0, 341.0, 342.0, 343.0, 344.0, 345.0, 346.0, 347.0, 348.0, 349.0, 350.0, 351.0, 352.0, 353.0, 354.0, 355.0, 356.0, 357.0, 358.0, 359.0, 360.0, 361.0, 362.0, 363.0, 364.0, 365.0, 366.0, 367.0, 368.0, 369.0, 370.0, 371.0, 372.0, 373.0, 374.0, 375.0, 376.0, 377.0, 378.0, 379.0, 380.0, 381.0, 382.0, 383.0, 384.0]'))",
            [],
        )
        .unwrap();

        migrate(&conn).unwrap(); // applies V7

        // note_vectors is preserved (stale backup).
        let note_vec_count: i64 = conn
            .query_row("SELECT count(*) FROM note_vectors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(note_vec_count, 1, "note_vectors row is preserved");

        // chunk_vectors exists and is empty.
        let chunk_vec_count: i64 = conn
            .query_row("SELECT count(*) FROM chunk_vectors", [], |r| r.get(0))
            .unwrap();
        assert_eq!(chunk_vec_count, 0, "chunk_vectors is created empty");

        // embedded_hash is cleared so the indexer re-chunks on next start.
        let embedded_hash: Option<String> = conn
            .query_row("SELECT embedded_hash FROM notes WHERE id = 'n1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(
            embedded_hash, None,
            "V7 clears embedded_hash so the corpus is re-chunked and re-embedded"
        );
    }
}
