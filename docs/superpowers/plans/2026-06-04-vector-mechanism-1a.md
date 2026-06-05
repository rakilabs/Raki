# Vector Mechanism (Slice 1a) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add semantic vector indexing to Raki — real fastembed embeddings stored in a sqlite-vec `vec0` table, kept consistent with notes through a content-hash-keyed, race-safe, decoupled embedding pipeline.

**Architecture:** Two new adapters behind existing domain ports (`FastEmbedProvider` for `EmbeddingProvider`, `SqliteVectorIndex` for `VectorIndex`) plus a new `IndexingStore` port for the note-side staleness queries. An app-layer `embed_pending` service orchestrates them: select stale notes → embed → upsert vector → **compare-and-stamp**. Vectors are deleted in the same transaction as note soft-deletes (no orphans). The eval harness (Slice 1b) is a separate follow-up plan.

**Tech Stack:** Rust · `fastembed = "5.15"` (ONNX, `bge-small-en-v1.5`, 384-dim) · `sqlite-vec = "=0.1.10-alpha.4"` (vec0, bundled rusqlite) · `unicode-normalization` (NFC for the content hash) · tokio.

---

## Spec & Decisions (from `docs/superpowers/specs/2026-06-04-vector-retrieval-eval-design.md`)

- **In scope (1a):** `FastEmbedProvider`, `SqliteVectorIndex`, migration V3, `IndexingStore` + `SqliteIndexingStore`, `embed_pending` service (content hash, compare-and-stamp, deletion, single-flight, per-note failure isolation, startup + post-save triggers), app wiring.
- **Out of scope (1b / later):** eval harness, metrics, gate, report; RRF fusion; wiring vector search into the `search_notes` command (vector query path exists as mechanism but is not yet a user command — that arrives with fusion #2).
- **Key invariants:** save stays instant; one `embed_pending()` mechanism serves first-index/post-save/model-swap/backfill; changing the model id re-embeds everything; no stamping stale content as current; no orphan vectors.

## File Structure

```
src-tauri/crates/raki-domain/src/ports.rs        MODIFY  + EmbeddingProvider::model_id, + PendingNote, + IndexingStore
src-tauri/crates/raki-domain/src/lib.rs          MODIFY  export PendingNote, IndexingStore
src-tauri/crates/raki-ai/Cargo.toml              MODIFY  + fastembed, tokio (dep)
src-tauri/crates/raki-ai/src/fake.rs             MODIFY  + model_id
src-tauri/crates/raki-ai/src/fastembed.rs        CREATE  FastEmbedProvider
src-tauri/crates/raki-ai/src/lib.rs              MODIFY  export FastEmbedProvider
src-tauri/crates/raki-storage/Cargo.toml         MODIFY  + sqlite-vec, unicode-normalization
src-tauri/crates/raki-storage/src/db.rs          MODIFY  register sqlite-vec before open
src-tauri/crates/raki-storage/src/migrations.rs  MODIFY  + V3 (note_vectors + notes columns)
src-tauri/crates/raki-storage/src/hash.rs        CREATE  content_hash(title, body)
src-tauri/crates/raki-storage/src/notes.rs       MODIFY  upsert writes content_hash; soft_delete drops vector
src-tauri/crates/raki-storage/src/vectors.rs     CREATE  SqliteVectorIndex
src-tauri/crates/raki-storage/src/indexing.rs    CREATE  SqliteIndexingStore
src-tauri/crates/raki-storage/src/lib.rs         MODIFY  modules + exports
src-tauri/src/indexing.rs                         CREATE  embed_pending + IndexingService
src-tauri/src/state.rs                            MODIFY  AppState: drop embedder, + index
src-tauri/src/lib.rs                              MODIFY  construct adapters + service, startup trigger
src-tauri/src/commands/notes.rs                   MODIFY  create_note triggers an indexing pass
```

---

## Task 1: Link sqlite-vec and register the extension (de-risking spike)

**Files:**
- Modify: `src-tauri/crates/raki-storage/Cargo.toml`
- Modify: `src-tauri/crates/raki-storage/src/db.rs`

- [ ] **Step 1: Add the dependency**

In `src-tauri/crates/raki-storage/Cargo.toml`, under `[dependencies]`, add (exact pin — only pre-release versions are published):

```toml
sqlite-vec = "=0.1.10-alpha.4"
unicode-normalization = "0.1"
```

- [ ] **Step 2: Write the failing smoke test**

Append to `src-tauri/crates/raki-storage/src/db.rs` (a new `#[cfg(test)]` module):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_vec_extension_is_registered() {
        let db = Database::open_in_memory().unwrap();
        // vec_version() only resolves if the sqlite-vec extension loaded.
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let version: String = rt.block_on(async {
            db.call(|c| c.query_row("SELECT vec_version()", [], |r| r.get(0)))
                .await
                .unwrap()
        });
        assert!(version.starts_with('v'), "got vec_version = {version}");
    }
}
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-storage sqlite_vec_extension_is_registered`
Expected: FAIL — `no such function: vec_version`.

- [ ] **Step 4: Register the extension before opening any connection**

In `src-tauri/crates/raki-storage/src/db.rs`, add imports at the top:

```rust
use rusqlite::ffi::sqlite3_auto_extension;
use sqlite_vec::sqlite3_vec_init;
```

Add this free function near `storage_err`:

```rust
/// Register sqlite-vec as an auto-extension exactly once, before any connection
/// opens. `sqlite3_auto_extension` applies to every connection opened afterward.
#[allow(clippy::missing_transmute_annotations)]
fn register_sqlite_vec() {
    static REGISTER: std::sync::Once = std::sync::Once::new();
    REGISTER.call_once(|| unsafe {
        sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ())));
    });
}
```

Call it as the **first line** of both constructors:

```rust
    pub fn open(path: &Path) -> Result<Self, DomainError> {
        register_sqlite_vec();
        let conn = Connection::open(path).map_err(storage_err)?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self, DomainError> {
        register_sqlite_vec();
        let conn = Connection::open_in_memory().map_err(storage_err)?;
        Self::init(conn)
    }
```

- [ ] **Step 5: Run it to verify it passes**

Run: `cd src-tauri && cargo test -p raki-storage sqlite_vec_extension_is_registered`
Expected: PASS — prints a `vec_version` like `v0.1.x`.

> **If linking fails** (duplicate `sqlite3` symbols between `libsqlite3-sys` bundled and `sqlite-vec`): this is the one real integration risk. Stop and report it — the fallback is runtime extension loading via `Connection::load_extension` from a bundled dylib, which changes this task. Do not paper over a link error.

- [ ] **Step 6: Lint + commit**

Run: `cd src-tauri && cargo clippy -p raki-storage -- -D warnings`
```bash
git add src-tauri/crates/raki-storage/Cargo.toml src-tauri/crates/raki-storage/src/db.rs src-tauri/Cargo.lock
git commit -m "Register sqlite-vec extension in the storage connection"
```

---

## Task 2: Migration V3 — `note_vectors` table + staleness columns

**Files:**
- Modify: `src-tauri/crates/raki-storage/src/migrations.rs`

- [ ] **Step 1: Write the failing test**

In `src-tauri/crates/raki-storage/src/migrations.rs`, add to the existing `#[cfg(test)] mod tests` block a new test (keep the existing `migration_creates_fts_table`):

```rust
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
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-storage migration_creates_note_vectors_and_columns`
Expected: FAIL — `no such table: note_vectors`.

- [ ] **Step 3: Append V3 to the `MIGRATIONS` array**

In `src-tauri/crates/raki-storage/src/migrations.rs`, add a third element to the `MIGRATIONS` const (after the V2 string):

```rust
    // V3: semantic vector index (sqlite-vec) + embedding staleness tracking on notes.
    // note_vectors is a vec0 virtual table; the embedding pipeline keeps it in sync.
    "CREATE VIRTUAL TABLE note_vectors USING vec0(
        note_id TEXT PRIMARY KEY,
        embedding float[384]
    );
    ALTER TABLE notes ADD COLUMN content_hash TEXT;
    ALTER TABLE notes ADD COLUMN embedded_hash TEXT;
    ALTER TABLE notes ADD COLUMN embedded_model TEXT;",
```

> Note: `STRICT` tables don't allow adding columns with constraints, but plain
> `ADD COLUMN <name> TEXT` (nullable, no default) is fine on a STRICT table.

- [ ] **Step 4: Run it to verify it passes**

Run: `cd src-tauri && cargo test -p raki-storage migration_creates_note_vectors_and_columns`
Expected: PASS.

- [ ] **Step 5: Run the full storage suite + lint**

Run: `cd src-tauri && cargo test -p raki-storage && cargo clippy -p raki-storage -- -D warnings`
Expected: all existing + new tests pass, no warnings.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-storage/src/migrations.rs
git commit -m "Add migration V3: note_vectors table and embedding staleness columns"
```

---

## Task 3: Content hash + write it on every note upsert

**Files:**
- Create: `src-tauri/crates/raki-storage/src/hash.rs`
- Modify: `src-tauri/crates/raki-storage/src/lib.rs`
- Modify: `src-tauri/crates/raki-storage/src/notes.rs`

- [ ] **Step 1: Write the failing tests for the hash function**

Create `src-tauri/crates/raki-storage/src/hash.rs`:

```rust
//! Stable content hash for embedding-staleness detection. NOT a security hash —
//! a fast, deterministic change-detector. A wrong hash silently breaks the cache
//! (never re-embed → stale vectors; always re-embed → wasted compute), so the
//! definition is pinned here, not left loose.

use unicode_normalization::UnicodeNormalization;

/// FNV-1a 64-bit over NFC-normalized, whitespace-collapsed `title` + `body`.
/// Volatile fields (timestamps, version, id, deleted_at) are deliberately excluded.
pub fn content_hash(title: &str, body: &str) -> String {
    let normalized = format!("{}\u{0}{}", normalize(title), normalize(body));
    let mut h: u64 = 0xcbf29ce484222325; // FNV offset basis
    for b in normalized.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3); // FNV prime
    }
    format!("{h:016x}")
}

/// NFC-normalize, collapse internal whitespace runs to a single space, and trim.
fn normalize(s: &str) -> String {
    let nfc: String = s.nfc().collect();
    nfc.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_and_whitespace_insensitive() {
        assert_eq!(content_hash("Hello", "World"), content_hash("Hello", "World"));
        assert_eq!(content_hash("  Hello ", "World"), content_hash("Hello", "World"));
        assert_eq!(content_hash("Hello   World", ""), content_hash("Hello World", ""));
    }

    #[test]
    fn distinguishes_content_and_field_boundary() {
        // different content → different hash
        assert_ne!(content_hash("a", "b"), content_hash("a", "c"));
        // the field separator prevents "ab"+"" colliding with "a"+"b"
        assert_ne!(content_hash("ab", ""), content_hash("a", "b"));
    }
}
```

- [ ] **Step 2: Register the module**

In `src-tauri/crates/raki-storage/src/lib.rs`, add `mod hash;` after `mod db;`:

```rust
mod db;
mod hash;
mod migrations;
mod notes;
mod search;
```

- [ ] **Step 3: Run it to verify it fails, then passes**

Run: `cd src-tauri && cargo test -p raki-storage hash::`
Expected: FAIL first (module/function absent if you reorder), then after Steps 1–2 PASS. Run again to confirm:
Run: `cd src-tauri && cargo test -p raki-storage hash::`
Expected: PASS (2 tests).

- [ ] **Step 4: Write the failing test for upsert writing content_hash**

In `src-tauri/crates/raki-storage/src/notes.rs`, add inside the existing `#[cfg(test)] mod tests` block:

```rust
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
        let h1 = content_hash_of(&db, &id.to_string()).await.expect("hash set");

        note.body = "different body".to_string();
        repo.upsert(&note).await.unwrap();
        let h2 = content_hash_of(&db, &id.to_string()).await.expect("hash set");

        assert_ne!(h1, h2, "editing body changes the content hash");
    }
```

- [ ] **Step 5: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-storage upsert_writes_content_hash_and_updates_on_edit`
Expected: FAIL — `content_hash` column is NULL (upsert doesn't write it yet) → `h1` unwrap panics.

- [ ] **Step 6: Make `upsert` compute and write the content hash**

In `src-tauri/crates/raki-storage/src/notes.rs`, add the import near the top:

```rust
use crate::hash::content_hash;
```

Replace the `upsert` method body (note the new `content_hash` column `?8` in both the INSERT and the ON CONFLICT update):

```rust
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
```

- [ ] **Step 7: Run it to verify it passes**

Run: `cd src-tauri && cargo test -p raki-storage`
Expected: PASS — all storage tests including the new hash + upsert tests.

- [ ] **Step 8: Lint + commit**

Run: `cd src-tauri && cargo clippy -p raki-storage -- -D warnings`
```bash
git add src-tauri/crates/raki-storage/src/hash.rs src-tauri/crates/raki-storage/src/lib.rs src-tauri/crates/raki-storage/src/notes.rs
git commit -m "Compute and store a content hash on every note upsert"
```

---

## Task 4: Delete the vector when a note is soft-deleted (no orphans)

**Files:**
- Modify: `src-tauri/crates/raki-storage/src/notes.rs`

- [ ] **Step 1: Write the failing test**

In `src-tauri/crates/raki-storage/src/notes.rs`, add inside the `#[cfg(test)] mod tests` block:

```rust
    async fn vector_count(db: &Database, note_id: &str) -> i64 {
        let id = note_id.to_string();
        db.call(move |c| {
            c.query_row(
                "SELECT count(*) FROM note_vectors WHERE note_id = ?1",
                rusqlite::params![id],
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
                "INSERT INTO note_vectors (note_id, embedding) VALUES (?1, ?2)",
                rusqlite::params![id_str, blob],
            )?;
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(vector_count(&db, &id.to_string()).await, 1);

        repo.soft_delete(&id, 2000).await.unwrap();
        assert_eq!(vector_count(&db, &id.to_string()).await, 0);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-storage soft_delete_removes_vector`
Expected: FAIL — vector still present (count 1) after soft_delete.

- [ ] **Step 3: Drop the vector inside the soft_delete transaction**

In `src-tauri/crates/raki-storage/src/notes.rs`, replace the `soft_delete` method body (add the `note_vectors` delete next to the `notes_fts` delete):

```rust
    async fn soft_delete(&self, id: &NoteId, at_ms: i64) -> Result<(), DomainError> {
        let id_str = id.to_string();
        self.db
            .call(move |c| {
                let tx = c.unchecked_transaction()?;
                tx.execute(
                    "UPDATE notes SET deleted_at = ?2, version = version + 1
                     WHERE id = ?1 AND deleted_at IS NULL",
                    params![id_str, at_ms],
                )?;
                tx.execute("DELETE FROM notes_fts WHERE note_id = ?1", params![id_str])?;
                tx.execute("DELETE FROM note_vectors WHERE note_id = ?1", params![id_str])?;
                tx.commit()?;
                Ok(())
            })
            .await
    }
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cd src-tauri && cargo test -p raki-storage`
Expected: PASS — including `soft_delete_removes_vector` and the existing `soft_delete_removes_from_fts`.

- [ ] **Step 5: Lint + commit**

Run: `cd src-tauri && cargo clippy -p raki-storage -- -D warnings`
```bash
git add src-tauri/crates/raki-storage/src/notes.rs
git commit -m "Remove a note's vector in the same transaction as soft-delete"
```

---

## Task 5: `SqliteVectorIndex` — implement the `VectorIndex` port

**Files:**
- Create: `src-tauri/crates/raki-storage/src/vectors.rs`
- Modify: `src-tauri/crates/raki-storage/src/lib.rs`

Reference — the port in `raki-domain/src/ports.rs`:
```rust
pub struct VectorHit { pub source_id: String, pub distance: f32 }
#[async_trait]
pub trait VectorIndex: Send + Sync {
    async fn upsert(&self, source_id: &str, embedding: &Embedding) -> Result<(), DomainError>;
    async fn query(&self, embedding: &Embedding, k: usize) -> Result<Vec<VectorHit>, DomainError>;
}
```
`Embedding` is `pub struct Embedding(pub Vec<f32>)`.

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/crates/raki-storage/src/vectors.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{Embedding, VectorIndex};

    use crate::db::Database;

    /// A 384-dim basis vector: all zeros except a 1.0 at position `i`.
    fn basis(i: usize) -> Embedding {
        let mut v = vec![0.0_f32; 384];
        v[i] = 1.0;
        Embedding(v)
    }

    #[tokio::test]
    async fn upsert_then_query_returns_nearest_first() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db);
        index.upsert("a", &basis(0)).await.unwrap();
        index.upsert("b", &basis(1)).await.unwrap();
        index.upsert("c", &basis(2)).await.unwrap();

        let hits = index.query(&basis(1), 3).await.unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].source_id, "b", "exact match ranks first");
    }

    #[tokio::test]
    async fn upsert_is_idempotent_overwrite() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db.clone());
        index.upsert("a", &basis(0)).await.unwrap();
        index.upsert("a", &basis(5)).await.unwrap(); // overwrite, not duplicate

        let n: i64 = db
            .call(|c| c.query_row("SELECT count(*) FROM note_vectors", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(n, 1, "re-upserting the same id overwrites");
    }

    #[tokio::test]
    async fn query_limits_to_k() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db);
        for i in 0..5 {
            index.upsert(&format!("n{i}"), &basis(i)).await.unwrap();
        }
        let hits = index.query(&basis(0), 2).await.unwrap();
        assert_eq!(hits.len(), 2);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-storage vectors`
Expected: FAIL — `SqliteVectorIndex` not found.

- [ ] **Step 3: Implement the adapter (prepend above the test module)**

```rust
//! The sqlite-vec-backed VectorIndex. Vectors are stored as compact little-endian
//! f32 blobs in the `note_vectors` vec0 table (declared `float[384]`).

use async_trait::async_trait;
use rusqlite::params;

use raki_domain::{DomainError, Embedding, VectorHit, VectorIndex};

use crate::db::Database;

pub struct SqliteVectorIndex {
    db: Database,
}

impl SqliteVectorIndex {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

/// vec0 stores float32 vectors as a raw little-endian f32 byte blob. Building it by
/// hand keeps us off the (alpha-stage) zerocopy dependency.
fn embedding_to_blob(e: &Embedding) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(e.0.len() * 4);
    for x in &e.0 {
        bytes.extend_from_slice(&x.to_le_bytes());
    }
    bytes
}

#[async_trait]
impl VectorIndex for SqliteVectorIndex {
    async fn upsert(&self, source_id: &str, embedding: &Embedding) -> Result<(), DomainError> {
        let id = source_id.to_string();
        let blob = embedding_to_blob(embedding);
        self.db
            .call(move |c| {
                // vec0 has no UPSERT; delete+insert overwrites by primary key.
                let tx = c.unchecked_transaction()?;
                tx.execute("DELETE FROM note_vectors WHERE note_id = ?1", params![id])?;
                tx.execute(
                    "INSERT INTO note_vectors (note_id, embedding) VALUES (?1, ?2)",
                    params![id, blob],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    async fn query(&self, embedding: &Embedding, k: usize) -> Result<Vec<VectorHit>, DomainError> {
        let blob = embedding_to_blob(embedding);
        self.db
            .call(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT note_id, distance
                     FROM note_vectors
                     WHERE embedding MATCH ?1 AND k = ?2
                     ORDER BY distance",
                )?;
                let hits = stmt
                    .query_map(params![blob, k as i64], |row| {
                        Ok(VectorHit {
                            source_id: row.get(0)?,
                            distance: row.get::<_, f64>(1)? as f32,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(hits)
            })
            .await
    }
}
```

- [ ] **Step 4: Export it**

In `src-tauri/crates/raki-storage/src/lib.rs`, add `mod vectors;` and the export:

```rust
mod db;
mod hash;
mod indexing;
mod migrations;
mod notes;
mod search;
mod vectors;

pub use db::Database;
pub use indexing::SqliteIndexingStore;
pub use notes::SqliteNoteRepository;
pub use search::SqliteKeywordIndex;
pub use vectors::SqliteVectorIndex;
```

> `mod indexing;` / `SqliteIndexingStore` are added now so the module list is final;
> the file is created in Task 6. If you run the build between tasks, create an empty
> `indexing.rs` placeholder first, or reorder Task 6 before this export line. Simplest:
> add only `mod vectors;` + `pub use vectors::SqliteVectorIndex;` here, and add the
> `indexing` lines in Task 6.

For this task, add only:

```rust
mod vectors;
pub use vectors::SqliteVectorIndex;
```

- [ ] **Step 5: Run it to verify it passes**

Run: `cd src-tauri && cargo test -p raki-storage vectors`
Expected: PASS — 3 vector tests.

- [ ] **Step 6: Lint + commit**

Run: `cd src-tauri && cargo clippy -p raki-storage -- -D warnings`
```bash
git add src-tauri/crates/raki-storage/src/vectors.rs src-tauri/crates/raki-storage/src/lib.rs
git commit -m "Add SqliteVectorIndex over sqlite-vec vec0"
```

---

## Task 6: `IndexingStore` port + `SqliteIndexingStore` (staleness + compare-and-stamp)

**Files:**
- Modify: `src-tauri/crates/raki-domain/src/ports.rs`, `src-tauri/crates/raki-domain/src/lib.rs`
- Create: `src-tauri/crates/raki-storage/src/indexing.rs`
- Modify: `src-tauri/crates/raki-storage/src/lib.rs`

- [ ] **Step 1: Add the port to the domain**

In `src-tauri/crates/raki-domain/src/ports.rs`, append (after the `KeywordIndex` block):

```rust
/// A note awaiting (re)embedding: its id, the text to embed, and the content hash
/// that text corresponds to (used for the compare-and-stamp guard).
pub struct PendingNote {
    pub id: NoteId,
    pub text: String,
    pub content_hash: String,
}

#[async_trait]
pub trait IndexingStore: Send + Sync {
    /// One-time: populate `content_hash` for any rows missing it (pre-V3 notes).
    /// Idempotent; a no-op once every live note has a hash.
    async fn backfill_content_hashes(&self) -> Result<(), DomainError>;

    /// Live notes whose embedding is missing or stale for `model_id`, at most `limit`.
    async fn list_pending(&self, model_id: &str, limit: usize) -> Result<Vec<PendingNote>, DomainError>;

    /// Compare-and-stamp: mark `id` embedded for (`content_hash`, `model_id`) ONLY if
    /// the note's CURRENT content_hash still equals `content_hash`. Returns `true` if
    /// it stamped, `false` if the content changed since `content_hash` was computed
    /// (the note stays stale and re-embeds next pass — never stamp stale as current).
    async fn mark_embedded(&self, id: &NoteId, content_hash: &str, model_id: &str) -> Result<bool, DomainError>;
}
```

In `src-tauri/crates/raki-domain/src/lib.rs`, extend the `ports` re-export to include the new items:

```rust
pub use ports::{
    Completion, CompletionRequest, Embedding, EmbeddingProvider, IndexingStore, KeywordHit,
    KeywordIndex, LlmProvider, Locality, NoteRepository, PendingNote, VectorHit, VectorIndex,
};
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/crates/raki-storage/src/indexing.rs`:

```rust
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
        assert!(!stamped, "must not stamp an embedding for superseded content");
        assert_eq!(store.list_pending(MODEL, 10).await.unwrap().len(), 1, "still pending");
    }

    #[tokio::test]
    async fn changing_model_makes_notes_pending_again() {
        let db = Database::open_in_memory().unwrap();
        let id = seed(&db, "Hello").await;
        let store = SqliteIndexingStore::new(db.clone());

        let hash = store.list_pending(MODEL, 10).await.unwrap()[0].content_hash.clone();
        store.mark_embedded(&id, &hash, MODEL).await.unwrap();
        assert!(store.list_pending(MODEL, 10).await.unwrap().is_empty());

        // A different model id ⇒ everything is stale again.
        assert_eq!(store.list_pending("other-model", 10).await.unwrap().len(), 1);
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
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-storage indexing`
Expected: FAIL — `SqliteIndexingStore` not found.

- [ ] **Step 4: Implement the adapter (prepend above the test module)**

```rust
//! The note-side of the embedding pipeline: which notes need (re)embedding, and the
//! compare-and-stamp that records an embedding without clobbering newer content.

use async_trait::async_trait;
use rusqlite::params;

use raki_domain::{DomainError, IndexingStore, NoteId, PendingNote};

use crate::db::Database;
use crate::hash::content_hash;

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

    async fn list_pending(&self, model_id: &str, limit: usize) -> Result<Vec<PendingNote>, DomainError> {
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
                        let id = NoteId::parse(&id)
                            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                        Ok(PendingNote {
                            id,
                            text: format!("{title}\n\n{body}"),
                            content_hash,
                        })
                    })
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .await
    }

    async fn mark_embedded(&self, id: &NoteId, content_hash: &str, model_id: &str) -> Result<bool, DomainError> {
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
```

- [ ] **Step 5: Export it**

In `src-tauri/crates/raki-storage/src/lib.rs`, add the `indexing` module + export (final module list):

```rust
mod indexing;
pub use indexing::SqliteIndexingStore;
```

- [ ] **Step 6: Run it to verify it passes**

Run: `cd src-tauri && cargo test -p raki-storage indexing && cargo test -p raki-domain`
Expected: PASS — 4 indexing tests + domain tests still green.

- [ ] **Step 7: Lint + commit**

Run: `cd src-tauri && cargo clippy -p raki-storage -p raki-domain -- -D warnings`
```bash
git add src-tauri/crates/raki-domain/src/ports.rs src-tauri/crates/raki-domain/src/lib.rs src-tauri/crates/raki-storage/src/indexing.rs src-tauri/crates/raki-storage/src/lib.rs
git commit -m "Add IndexingStore port and SqliteIndexingStore with compare-and-stamp"
```

---

## Task 7: `FastEmbedProvider` — real embeddings + `model_id` on the port

**Files:**
- Modify: `src-tauri/crates/raki-ai/Cargo.toml`
- Modify: `src-tauri/crates/raki-domain/src/ports.rs`
- Modify: `src-tauri/crates/raki-ai/src/fake.rs`
- Create: `src-tauri/crates/raki-ai/src/fastembed.rs`
- Modify: `src-tauri/crates/raki-ai/src/lib.rs`

- [ ] **Step 1: Add `model_id` to the `EmbeddingProvider` port**

In `src-tauri/crates/raki-domain/src/ports.rs`, add a method to the `EmbeddingProvider` trait:

```rust
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn dimension(&self) -> usize;
    fn locality(&self) -> Locality;
    /// Stable identifier of the model+version. Drives embedding staleness: changing
    /// it re-embeds the whole corpus.
    fn model_id(&self) -> String;
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, DomainError>;
}
```

- [ ] **Step 2: Update `FakeEmbeddingProvider` to satisfy the port**

In `src-tauri/crates/raki-ai/src/fake.rs`, add the method inside `impl EmbeddingProvider for FakeEmbeddingProvider` (above `embed`):

```rust
    fn model_id(&self) -> String {
        format!("fake-{}", self.dim)
    }
```

- [ ] **Step 3: Verify the fake still builds/tests (port change is satisfied)**

Run: `cd src-tauri && cargo test -p raki-ai`
Expected: PASS — `fake_embeddings_are_deterministic_and_sized` still green.

- [ ] **Step 4: Add fastembed dependencies**

In `src-tauri/crates/raki-ai/Cargo.toml`, add to `[dependencies]`:

```toml
fastembed = "5.15"
tokio = { workspace = true }
```

(`tokio` moves from dev to a normal dep — `embed` runs the CPU-bound model on `spawn_blocking`.)

- [ ] **Step 5: Write the failing test (ignored: it downloads the model)**

Create `src-tauri/crates/raki-ai/src/fastembed.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{EmbeddingProvider, Locality};

    #[tokio::test]
    #[ignore = "downloads the bge-small model on first run; run explicitly with --ignored"]
    async fn fastembed_smoke_produces_384_dim_distinct_vectors() {
        let p = FastEmbedProvider::try_new().expect("model init");
        assert_eq!(p.dimension(), 384);
        assert_eq!(p.locality(), Locality::Local);
        assert_eq!(p.model_id(), "bge-small-en-v1.5");

        let out = p
            .embed(&["apples are red".to_string(), "the stock market fell".to_string()])
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0.len(), 384);
        assert_ne!(out[0].0, out[1].0, "different text → different vectors");
    }
}
```

- [ ] **Step 6: Run it to verify it fails to compile**

Run: `cd src-tauri && cargo test -p raki-ai fastembed`
Expected: FAIL — `FastEmbedProvider` not found.

- [ ] **Step 7: Implement the provider (prepend above the test module)**

```rust
//! The fastembed-backed EmbeddingProvider: in-process ONNX, model `bge-small-en-v1.5`
//! (384-dim). The model downloads once on first construction and is cached on disk.

use std::sync::Arc;

use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use raki_domain::{DomainError, Embedding, EmbeddingProvider, Locality};

/// Stable model identifier stored alongside embeddings (drives staleness).
pub const MODEL_ID: &str = "bge-small-en-v1.5";
/// bge models want a query instruction prefix on the QUERY side only. Document
/// embeddings (the pipeline's path) are embedded as-is; the query-issuing layer
/// (retrieval/eval, later slices) applies this prefix. Exposed here for reuse.
pub const BGE_QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

pub struct FastEmbedProvider {
    model: Arc<TextEmbedding>,
}

impl FastEmbedProvider {
    pub fn try_new() -> Result<Self, DomainError> {
        let model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::BGESmallENV15))
            .map_err(|e| DomainError::Provider(format!("fastembed init: {e}")))?;
        Ok(Self {
            model: Arc::new(model),
        })
    }
}

#[async_trait]
impl EmbeddingProvider for FastEmbedProvider {
    fn dimension(&self) -> usize {
        384
    }

    fn locality(&self) -> Locality {
        Locality::Local
    }

    fn model_id(&self) -> String {
        MODEL_ID.to_string()
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, DomainError> {
        let model = self.model.clone();
        let owned: Vec<String> = inputs.to_vec();
        let vectors = tokio::task::spawn_blocking(move || model.embed(owned, None))
            .await
            .map_err(|e| DomainError::Provider(format!("embed join: {e}")))?
            .map_err(|e| DomainError::Provider(format!("embed: {e}")))?;
        Ok(vectors.into_iter().map(Embedding).collect())
    }
}
```

> If the build fails because `Arc<TextEmbedding>` isn't `Send`/`Sync`, wrap the model
> in `std::sync::Mutex<TextEmbedding>` and lock inside the `spawn_blocking` closure.
> Report which path you took.

- [ ] **Step 8: Export it**

In `src-tauri/crates/raki-ai/src/lib.rs`:

```rust
//! AI provider adapters (local + cloud) and the egress/consent policy.

mod egress;
mod fake;
mod fastembed;

pub use egress::EgressPolicy;
pub use fake::FakeEmbeddingProvider;
pub use fastembed::FastEmbedProvider;
```

- [ ] **Step 9: Verify it compiles, fast suite passes, real smoke passes**

Run: `cd src-tauri && cargo test -p raki-ai` (fast: the real test is `#[ignore]`d)
Expected: PASS — fake test runs; fastembed smoke is listed as ignored.
Run: `cd src-tauri && cargo test -p raki-ai --ignored fastembed_smoke_produces_384_dim_distinct_vectors`
Expected: PASS (downloads the model once; may take a minute on first run).

> If the machine is offline and the ignored test can't download, report it as
> deferred rather than as a failure — it's gated for exactly this reason.

- [ ] **Step 10: Lint + commit**

Run: `cd src-tauri && cargo clippy -p raki-ai -p raki-domain -- -D warnings`
```bash
git add src-tauri/crates/raki-ai/Cargo.toml src-tauri/crates/raki-ai/src/fastembed.rs src-tauri/crates/raki-ai/src/lib.rs src-tauri/crates/raki-ai/src/fake.rs src-tauri/crates/raki-domain/src/ports.rs src-tauri/Cargo.lock
git commit -m "Add FastEmbedProvider (bge-small-en-v1.5) and model_id on the port"
```

---

## Task 8: `embed_pending` service + `IndexingService` (orchestration)

**Files:**
- Create: `src-tauri/src/indexing.rs`
- Modify: `src-tauri/src/lib.rs` (just `mod indexing;` — full wiring is Task 9)

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/indexing.rs`:

```rust
//! The embedding pipeline orchestration: drain stale notes through embed → upsert
//! vector → compare-and-stamp, with per-note failure isolation. Pure of Tauri so it
//! is unit-testable; `IndexingService` adds dependency injection + single-flight.

use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::Mutex;

use raki_domain::{DomainError, EmbeddingProvider, IndexingStore, NoteId, PendingNote, VectorIndex};

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
        let todo: Vec<PendingNote> = pending.into_iter().filter(|p| !failed.contains(&p.id)).collect();
        if todo.is_empty() {
            break;
        }
        for note in todo {
            match embed_one(store, embedder, vectors, &note, &model_id).await {
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
    model_id: &str,
) -> Result<bool, DomainError> {
    let mut out = embedder.embed(&[note.text.clone()]).await?;
    let emb = out
        .pop()
        .ok_or_else(|| DomainError::Provider("embedder returned no vector".to_string()))?;
    vectors.upsert(&note.id.to_string(), &emb).await?;
    // Stamp last: if we crash before this, the note stays stale and re-embeds.
    store.mark_embedded(&note.id, &note.content_hash, model_id).await
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
        embed_pending(self.store.as_ref(), self.embedder.as_ref(), self.vectors.as_ref(), self.batch).await
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

    fn wiring(db: &Database) -> (Arc<dyn IndexingStore>, Arc<dyn EmbeddingProvider>, Arc<dyn VectorIndex>) {
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

        let first = embed_pending(store.as_ref(), embedder.as_ref(), vectors.as_ref(), 32).await.unwrap();
        assert_eq!(first.embedded, 3);
        assert_eq!(vector_count(&db).await, 3);

        // Nothing stale now → a second pass embeds nothing.
        let second = embed_pending(store.as_ref(), embedder.as_ref(), vectors.as_ref(), 32).await.unwrap();
        assert_eq!(second, EmbedStats { embedded: 0, failed: 0 });
    }

    #[tokio::test]
    async fn re_embeds_only_the_edited_note() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        repo.upsert(&Note::new("Keep".to_string(), "body".to_string(), 1000)).await.unwrap();
        repo.upsert(&Note::new("Edit".to_string(), "body".to_string(), 1000)).await.unwrap();
        let (store, embedder, vectors) = wiring(&db);
        embed_pending(store.as_ref(), embedder.as_ref(), vectors.as_ref(), 32).await.unwrap();

        // Edit one note → exactly one becomes stale.
        let mut edit = repo.list().await.unwrap().into_iter().find(|n| n.title == "Edit").unwrap();
        edit.body = "rewritten".to_string();
        repo.upsert(&edit).await.unwrap();

        let again = embed_pending(store.as_ref(), embedder.as_ref(), vectors.as_ref(), 32).await.unwrap();
        assert_eq!(again.embedded, 1);
    }
}
```

- [ ] **Step 2: Register the module**

In `src-tauri/src/lib.rs`, add `mod indexing;` to the module list near the top:

```rust
mod commands;
mod dto;
mod error;
mod indexing;
mod state;
```

- [ ] **Step 3: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki indexing`
Expected: FAIL — `indexing` module / `embed_pending` referenced before it compiles; once Step 1+2 are in, it builds and the tests run.

- [ ] **Step 4: Run it to verify it passes**

Run: `cd src-tauri && cargo test -p raki indexing`
Expected: PASS — `embeds_all_pending_then_is_idempotent`, `re_embeds_only_the_edited_note`.

> The compare-and-stamp race itself is unit-tested at the store level
> (`mark_embedded_is_a_noop_when_content_changed`, Task 6); here we verify the
> drain/idempotency/selective-re-embed behavior end-to-end with real adapters.

- [ ] **Step 5: Lint + commit**

Run: `cd src-tauri && cargo clippy -p raki --lib --all-targets -- -D warnings`
```bash
git add src-tauri/src/indexing.rs src-tauri/src/lib.rs
git commit -m "Add embed_pending pipeline and single-flight IndexingService"
```

---

## Task 9: Wire the pipeline into the app (state, composition root, post-save trigger)

**Files:**
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands/notes.rs`

- [ ] **Step 1: Replace `AppState` (drop `embedder`, add `index`)**

In `src-tauri/src/state.rs`, replace the file:

```rust
//! Application state: the injected ports the command layer delegates to.

use std::sync::Arc;

use raki_ai::EgressPolicy;
use raki_domain::{Clock, KeywordIndex, NoteRepository};

use crate::indexing::IndexingService;

#[allow(dead_code)]
pub struct AppState {
    pub notes: Arc<dyn NoteRepository>,
    pub keyword: Arc<dyn KeywordIndex>,
    pub clock: Arc<dyn Clock>,
    pub egress: EgressPolicy,
    pub index: Arc<IndexingService>,
}
```

(The embedder + vector index now live inside `IndexingService`; `AppState` exposes the service.)

- [ ] **Step 2: Construct the adapters + service in the composition root**

In `src-tauri/src/lib.rs`, update the imports:

```rust
use raki_ai::{EgressPolicy, FakeEmbeddingProvider, FastEmbedProvider};
use raki_domain::{Clock, EmbeddingProvider, IndexingStore, VectorIndex};
use raki_storage::{
    Database, SqliteIndexingStore, SqliteKeywordIndex, SqliteNoteRepository, SqliteVectorIndex,
};

use crate::commands::notes::{create_note, get_note, list_notes, search_notes};
use crate::indexing::IndexingService;
use crate::state::AppState;
```

Replace the body of the `.setup(|app| { ... })` closure:

```rust
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db = Database::open(&dir.join("raki.sqlite"))?;

            let notes = Arc::new(SqliteNoteRepository::new(db.clone()));
            let keyword = Arc::new(SqliteKeywordIndex::new(db.clone()));
            let vectors: Arc<dyn VectorIndex> = Arc::new(SqliteVectorIndex::new(db.clone()));
            let store: Arc<dyn IndexingStore> = Arc::new(SqliteIndexingStore::new(db));

            // Real embeddings if the model is available; otherwise degrade to the fake
            // so the app still runs (keyword search is unaffected). The model-id
            // staleness check re-embeds with the real model once it's available.
            let embedder: Arc<dyn EmbeddingProvider> = match FastEmbedProvider::try_new() {
                Ok(p) => Arc::new(p),
                Err(e) => {
                    eprintln!("fastembed unavailable ({e}); using fake embeddings this session");
                    Arc::new(FakeEmbeddingProvider::new(384))
                }
            };

            let index = Arc::new(IndexingService::new(store, embedder, vectors));
            index.trigger(); // startup catch-up pass (backfill + drain), single-flight

            app.manage(AppState {
                notes,
                keyword,
                clock: Arc::new(SystemClock),
                egress: EgressPolicy::LocalOnly,
                index,
            });
            Ok(())
        })
```

- [ ] **Step 3: Trigger an indexing pass after a note is created**

In `src-tauri/src/commands/notes.rs`, update `create_note` to kick the pipeline after the write:

```rust
#[tauri::command]
pub async fn create_note(
    state: State<'_, AppState>,
    input: CreateNoteInput,
) -> Result<NoteDto, AppError> {
    let note = Note::new(input.title, input.body, state.clock.now_ms());
    state.notes.upsert(&note).await?;
    state.index.trigger(); // embed the new note in the background (single-flight)
    Ok(NoteDto::from(note))
}
```

- [ ] **Step 4: Build, test, lint, fmt the whole workspace**

Run: `cd src-tauri && cargo test --workspace`
Expected: PASS — all crate tests (the fastembed real test stays ignored).
Run: `cd src-tauri && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/state.rs src-tauri/src/lib.rs src-tauri/src/commands/notes.rs
git commit -m "Wire embedding pipeline into the app with startup and post-save triggers"
```

---

## Task 10: Slice 1a verification + Definition of Done

- [ ] **Step 1: Full workspace sweep**

Run: `cd src-tauri && cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all tests pass, fmt clean, no clippy warnings.

- [ ] **Step 2: Real-model integration check (network required)**

Run: `cd src-tauri && cargo test -p raki-ai --ignored`
Expected: `fastembed_smoke_produces_384_dim_distinct_vectors` passes (downloads bge-small once). If offline, report as deferred (`superpowers:verification-before-completion`) — do not claim success.

- [ ] **Step 3: Frontend untouched — confirm still green**

Run (repo root): `bun run typecheck && bun run test && bun run build`
Expected: all green (this slice changed no frontend code).

- [ ] **Step 4: Manual end-to-end smoke (user-performed)**

Run: `bun run tauri dev`
Expected: app launches; create a couple of notes ("Apples", "Oranges"); confirm no errors in the console and that the app data dir has a `raki.sqlite` whose `note_vectors` table is populated after a moment (the background pass). Vector search is not yet a UI command (arrives with fusion #2), so this confirms the *pipeline runs*, not search relevance.

> If `tauri dev` can't run here, this step is the user's; report it as deferred, not as success.

- [ ] **Step 5: Final commit (only if Step 1 required a fmt pass)**

```bash
git add -A
git commit -m "Slice 1a (vector mechanism): final verification"
```

---

## Self-Review

**Spec coverage (1a items from the design doc):**
- `FastEmbedProvider` (bge-small-en-v1.5, 384) → Task 7.
- `SqliteVectorIndex` (sqlite-vec vec0) → Task 5; extension registration → Task 1.
- Migration V3 (note_vectors + content_hash/embedded_hash/embedded_model) → Task 2.
- Content hash pinned (NFC, whitespace, volatile-field exclusion) → Task 3.
- Decoupled pipeline, content-hash keyed → Tasks 3/6/8.
- **Compare-and-stamp** race protection → Task 6 (`mark_embedded`) + test; pipeline use → Task 8.
- **Deletion** (no orphan vectors) → Task 4.
- Operational semantics: startup pass → Task 9 (`index.trigger()` at setup); single-flight → Task 8 (`running` mutex); per-note failure isolation → Task 8 (`failed` set); crash-recovery → inherent (stale ⇒ re-embed, noted in Task 8).
- Backfill for pre-V3 notes → Task 6 (`backfill_content_hashes`) + Task 8 (`run_once`).
- Graceful degradation to keyword-only if the model is unavailable → Task 9.
- App wiring → Task 9.

**Out of scope confirmed deferred:** eval harness/metrics/gate/report (Slice 1b); RRF fusion + wiring vector search into `search_notes` (#2); chunking (falsifiable deferral, 1b taxonomy); query-prefix application (constant exposed in Task 7, applied by the query layer in 1b/#2).

**Placeholder scan:** none — every code step has complete code; the only forward
reference (`BGE_QUERY_PREFIX` used later) is a deliberately-named deferral with the
constant defined now.

**Type consistency:**
- `EmbeddingProvider::model_id(&self) -> String` defined Task 7, implemented for Fake (Task 7) and FastEmbed (Task 7), consumed in `embed_pending` (Task 8).
- `IndexingStore { backfill_content_hashes, list_pending, mark_embedded }` + `PendingNote { id, text, content_hash }` defined Task 6, implemented Task 6, consumed Task 8.
- `VectorIndex { upsert, query }` (pre-existing port) implemented Task 5, consumed Task 8.
- `content_hash(&str, &str) -> String` defined Task 3, used by `upsert` (Task 3) and `SqliteIndexingStore` (Task 6).
- `IndexingService::new(store, embedder, vectors)` defined Task 8, constructed Task 9; `trigger(self: &Arc<Self>)` defined Task 8, called Task 9 (setup + create_note).
- `EmbedStats { embedded, failed }` defined Task 8, asserted in Task 8 tests.

---

## Execution Handoff

(Presented to the user after saving.)
