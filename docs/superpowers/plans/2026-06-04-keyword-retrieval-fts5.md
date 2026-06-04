# Keyword Retrieval (FTS5) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the naive substring `search_notes` with real SQLite **FTS5** full-text search, with the note row and its search index kept consistent in one transaction.

**Architecture:** A standalone FTS5 virtual table (`notes_fts`) is written **in the same transaction** as every `notes` write (`AGENT.md §5`, one transactional store) — so the repository owns keeping its derived index consistent. A read-only `KeywordIndex` port (trimmed to `query`) is implemented by `SqliteKeywordIndex` and consumed through a `raki-retrieval::search` seam (a keyword passthrough today, ready to fuse with vectors via RRF later). `sqlite-vec`, embeddings, and true hybrid search are explicitly the **next** plan.

**Tech Stack:** rusqlite `0.35` (bundled SQLite, FTS5 — no new deps) · SolidJS + @tanstack/solid-query · vitest.

---

## Scope & Non-Goals

**In scope:** FTS5 table + backfill migration; atomic note+FTS writes; `SqliteKeywordIndex` (bm25-ranked, injection-safe query); `raki-retrieval::search`; rewired `search_notes` command; a search box in the notes UI; tests at each layer.

**Deferred (next plan, still stubbed behind ports):** `sqlite-vec` vector index, the embedding pipeline (fastembed), true hybrid fusion of keyword+vector, prefix/fuzzy matching, ranking tuning.

## Design decision (surfaced for review)

The `KeywordIndex` port defined in the foundation had a speculative `upsert` method. Because the FTS row must commit atomically with the `notes` row, **writes go through `SqliteNoteRepository` inside one transaction**, not through a separate index-write port. So this plan **trims `KeywordIndex` to `query` only** (read path). The repository — which already owns `notes` SQL — also owns keeping `notes_fts` in sync. Both tables live entirely inside `raki-storage`, so "the only SQL is in storage" still holds.

## File Structure

```
src-tauri/crates/raki-domain/src/ports.rs   MODIFY → trim KeywordIndex to query-only
src-tauri/crates/raki-storage/src/migrations.rs MODIFY → add V2 (notes_fts + backfill)
src-tauri/crates/raki-storage/src/notes.rs   MODIFY → upsert/soft_delete write notes_fts in one tx
src-tauri/crates/raki-storage/src/search.rs  CREATE → SqliteKeywordIndex + fts_query sanitizer
src-tauri/crates/raki-storage/src/lib.rs     MODIFY → export SqliteKeywordIndex
src-tauri/crates/raki-retrieval/src/search.rs CREATE → search() seam over KeywordIndex
src-tauri/crates/raki-retrieval/src/lib.rs   MODIFY → export search
src-tauri/crates/raki-retrieval/Cargo.toml   MODIFY → dev-deps tokio + async-trait
src-tauri/src/state.rs                        MODIFY → AppState.keyword
src-tauri/src/lib.rs                          MODIFY → wire SqliteKeywordIndex
src-tauri/src/commands/notes.rs               MODIFY → search_notes via retrieval + hydrate
src/modules/notes/api.ts                      MODIFY → notesApi.search
src/modules/notes/NotesView.tsx               MODIFY → search box
src/modules/notes/api.test.ts                 CREATE → notesApi.search calls the command
```

---

## Task 1: Trim the `KeywordIndex` port to read-only

**Files:**
- Modify: `src-tauri/crates/raki-domain/src/ports.rs`

- [ ] **Step 1: Replace the `KeywordIndex` trait (and keep `KeywordHit`)**

In `src-tauri/crates/raki-domain/src/ports.rs`, replace the existing `KeywordHit` struct and `KeywordIndex` trait block:

```rust
pub struct KeywordHit {
    pub source_id: String,
    pub score: f32,
}

#[async_trait]
pub trait KeywordIndex: Send + Sync {
    async fn upsert(&self, source_id: &str, text: &str) -> Result<(), DomainError>;
    async fn query(&self, query: &str, k: usize) -> Result<Vec<KeywordHit>, DomainError>;
}
```

with (drop the `upsert` method — writes go through the repository transactionally):

```rust
pub struct KeywordHit {
    pub source_id: String,
    /// FTS5 bm25 value; lower is a better match. Used by retrieval for rank ordering.
    pub score: f32,
}

#[async_trait]
pub trait KeywordIndex: Send + Sync {
    /// Best-first keyword hits for `query`, at most `k`. Read-only — index writes
    /// happen transactionally inside the repository, not here.
    async fn query(&self, query: &str, k: usize) -> Result<Vec<KeywordHit>, DomainError>;
}
```

- [ ] **Step 2: Verify the workspace still builds**

Run: `cd src-tauri && cargo build`
Expected: builds clean (`KeywordIndex` has no implementors yet, so nothing breaks).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/raki-domain/src/ports.rs
git commit -m "Trim KeywordIndex port to read-only query"
```

---

## Task 2: Migration V2 — `notes_fts` table + backfill

**Files:**
- Modify: `src-tauri/crates/raki-storage/src/migrations.rs`

- [ ] **Step 1: Write the failing test**

Append to `src-tauri/crates/raki-storage/src/migrations.rs`:

```rust
#[cfg(test)]
mod tests {
    use crate::db::Database;

    #[test]
    fn migration_creates_fts_table() {
        let db = Database::open_in_memory().unwrap();
        // open_in_memory runs migrate(); notes_fts must exist and be queryable.
        let count: i64 = futures_lite_block(&db);
        assert_eq!(count, 0);
    }

    // Tiny synchronous helper: run a count against notes_fts via the blocking pool.
    fn futures_lite_block(db: &Database) -> i64 {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            db.call(|c| c.query_row("SELECT count(*) FROM notes_fts", [], |r| r.get(0)))
                .await
                .unwrap()
        })
    }
}
```

Add `tokio` to `raki-storage` dev-deps if not present. Check `src-tauri/crates/raki-storage/Cargo.toml` — it already lists `tokio` under `[dependencies]`, so the test can use it. No change needed.

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p raki-storage migration_creates_fts_table`
Expected: FAIL — `no such table: notes_fts`.

- [ ] **Step 3: Add the V2 migration**

In `src-tauri/crates/raki-storage/src/migrations.rs`, replace the `MIGRATIONS` constant:

```rust
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
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test -p raki-storage migration_creates_fts_table`
Expected: PASS.

- [ ] **Step 5: Run the full storage suite + lint**

Run: `cd src-tauri && cargo test -p raki-storage && cargo clippy -p raki-storage -- -D warnings`
Expected: all pass, no warnings.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-storage/src/migrations.rs
git commit -m "Add FTS5 notes_fts table migration with backfill"
```

---

## Task 3: Keep `notes_fts` in sync on every note write

**Files:**
- Modify: `src-tauri/crates/raki-storage/src/notes.rs`

- [ ] **Step 1: Write the failing tests (append to the existing `tests` module in `notes.rs`)**

Add these two tests inside the existing `#[cfg(test)] mod tests { ... }` block, after the current tests:

```rust
    async fn fts_count(db: &Database, note_id: &str) -> i64 {
        let id = note_id.to_string();
        db.call(move |c| {
            c.query_row(
                "SELECT count(*) FROM notes_fts WHERE note_id = ?1",
                rusqlite::params![id],
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p raki-storage upsert_indexes_into_fts soft_delete_removes_from_fts`
Expected: FAIL — `no such table: notes_fts` is gone (migration exists), but `fts_count` returns 0 after upsert because writes don't touch FTS yet.

- [ ] **Step 3: Make `upsert` write the note row and FTS row in one transaction**

In `src-tauri/crates/raki-storage/src/notes.rs`, replace the `upsert` method body:

```rust
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
```

- [ ] **Step 4: Make `soft_delete` also drop the FTS row in one transaction**

Replace the `soft_delete` method body:

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
                tx.commit()?;
                Ok(())
            })
            .await
    }
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p raki-storage`
Expected: PASS — all storage tests (existing 2 + migration + new 2) green.

- [ ] **Step 6: Lint + commit**

Run: `cd src-tauri && cargo clippy -p raki-storage -- -D warnings`
Expected: no warnings.
```bash
git add src-tauri/crates/raki-storage/src/notes.rs
git commit -m "Keep notes_fts in sync transactionally on note writes"
```

---

## Task 4: `SqliteKeywordIndex` — bm25 query with injection-safe input

**Files:**
- Create: `src-tauri/crates/raki-storage/src/search.rs`
- Modify: `src-tauri/crates/raki-storage/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/crates/raki-storage/src/search.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{KeywordIndex, Note, NoteRepository};

    use crate::db::Database;
    use crate::notes::SqliteNoteRepository;

    fn note(title: &str, body: &str) -> Note {
        Note::new(title.to_string(), body.to_string(), 1000)
    }

    #[test]
    fn fts_query_quotes_each_term_and_escapes_quotes() {
        assert_eq!(fts_query("hello world"), "\"hello\" \"world\"");
        assert_eq!(fts_query("  spaced  "), "\"spaced\"");
        assert_eq!(fts_query(""), "");
        // a stray double-quote must not break the FTS5 grammar
        assert_eq!(fts_query("a\"b"), "\"a\"\"b\"");
    }

    #[tokio::test]
    async fn query_finds_matching_note_and_skips_others() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let index = SqliteKeywordIndex::new(db);

        let apples = note("Apples", "crisp and red");
        let oranges = note("Oranges", "citrus");
        repo.upsert(&apples).await.unwrap();
        repo.upsert(&oranges).await.unwrap();

        let hits = index.query("apples", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].source_id, apples.id.to_string());
    }

    #[tokio::test]
    async fn empty_query_returns_no_hits() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteKeywordIndex::new(db);
        assert!(index.query("   ", 10).await.unwrap().is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test -p raki-storage search`
Expected: FAIL — `SqliteKeywordIndex` / `fts_query` not found.

- [ ] **Step 3: Implement the adapter (prepend above the test module)**

```rust
//! The FTS5-backed KeywordIndex (read path). Writes are kept in sync by the repository.

use async_trait::async_trait;
use rusqlite::params;

use raki_domain::{DomainError, KeywordHit, KeywordIndex};

use crate::db::Database;

pub struct SqliteKeywordIndex {
    db: Database,
}

impl SqliteKeywordIndex {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

/// Turn free user text into a safe FTS5 MATCH expression: quote each whitespace-
/// separated term (doubling embedded quotes) so punctuation can't break the grammar.
/// Empty input yields an empty string, which the caller treats as "no results".
fn fts_query(input: &str) -> String {
    input
        .split_whitespace()
        .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

#[async_trait]
impl KeywordIndex for SqliteKeywordIndex {
    async fn query(&self, query: &str, k: usize) -> Result<Vec<KeywordHit>, DomainError> {
        let match_expr = fts_query(query);
        if match_expr.is_empty() {
            return Ok(Vec::new());
        }
        self.db
            .call(move |c| {
                let mut stmt = c.prepare_cached(
                    "SELECT note_id, bm25(notes_fts) AS score
                     FROM notes_fts
                     WHERE notes_fts MATCH ?1
                     ORDER BY score
                     LIMIT ?2",
                )?;
                let hits = stmt
                    .query_map(params![match_expr, k as i64], |row| {
                        Ok(KeywordHit {
                            source_id: row.get(0)?,
                            score: row.get::<_, f64>(1)? as f32,
                        })
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(hits)
            })
            .await
    }
}
```

- [ ] **Step 4: Export it from the crate**

In `src-tauri/crates/raki-storage/src/lib.rs`, add the module and export:

```rust
//! SQLite-backed adapters implementing `raki-domain` ports. The only place SQL lives.

mod db;
mod migrations;
mod notes;
mod search;

pub use db::Database;
pub use notes::SqliteNoteRepository;
pub use search::SqliteKeywordIndex;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p raki-storage`
Expected: PASS — all storage tests, including the 4 new `search` tests.

- [ ] **Step 6: Lint + commit**

Run: `cd src-tauri && cargo clippy -p raki-storage -- -D warnings`
Expected: no warnings.
```bash
git add src-tauri/crates/raki-storage/src/search.rs src-tauri/crates/raki-storage/src/lib.rs
git commit -m "Add FTS5 SqliteKeywordIndex with injection-safe query"
```

---

## Task 5: `raki-retrieval::search` — the ranking seam

**Files:**
- Create: `src-tauri/crates/raki-retrieval/src/search.rs`
- Modify: `src-tauri/crates/raki-retrieval/src/lib.rs`, `src-tauri/crates/raki-retrieval/Cargo.toml`

- [ ] **Step 1: Add async dev-deps to `raki-retrieval`**

In `src-tauri/crates/raki-retrieval/Cargo.toml`, add a `[dev-dependencies]` section (the test needs an async runtime and a fake `KeywordIndex`):

```toml
[dev-dependencies]
tokio = { workspace = true }
async-trait = { workspace = true }
```

- [ ] **Step 2: Write the failing test**

Create `src-tauri/crates/raki-retrieval/src/search.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use raki_domain::{DomainError, KeywordHit, KeywordIndex};

    struct FakeKeyword(Vec<&'static str>);

    #[async_trait]
    impl KeywordIndex for FakeKeyword {
        async fn query(&self, _q: &str, _k: usize) -> Result<Vec<KeywordHit>, DomainError> {
            Ok(self
                .0
                .iter()
                .enumerate()
                .map(|(i, id)| KeywordHit {
                    source_id: id.to_string(),
                    score: i as f32,
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn search_returns_ids_in_index_order() {
        let index = FakeKeyword(vec!["b", "a", "c"]);
        let ids = search(&index, "anything", 10).await.unwrap();
        assert_eq!(ids, vec!["b".to_string(), "a".to_string(), "c".to_string()]);
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p raki-retrieval search`
Expected: FAIL — `search` not found.

- [ ] **Step 4: Implement the seam (prepend above the test module)**

```rust
//! The query-time ranking seam. Today it returns keyword hits in order; when a
//! VectorIndex lands, fuse keyword + vector rankings here via `reciprocal_rank_fusion`.

use raki_domain::{DomainError, KeywordIndex};

/// Return up to `k` source ids best-matching `query`, best-first.
pub async fn search(
    keyword: &dyn KeywordIndex,
    query: &str,
    k: usize,
) -> Result<Vec<String>, DomainError> {
    let hits = keyword.query(query, k).await?;
    Ok(hits.into_iter().map(|h| h.source_id).collect())
}
```

- [ ] **Step 5: Export it**

In `src-tauri/crates/raki-retrieval/src/lib.rs`, add the module and export:

```rust
//! Hybrid retrieval: rank fusion and ranking over the domain index ports.

mod fusion;
mod search;

pub use fusion::{reciprocal_rank_fusion, DEFAULT_RRF_K};
pub use search::search;
```

- [ ] **Step 6: Run tests + lint, commit**

Run: `cd src-tauri && cargo test -p raki-retrieval && cargo clippy -p raki-retrieval --all-targets -- -D warnings`
Expected: PASS, no warnings.
```bash
git add src-tauri/crates/raki-retrieval
git commit -m "Add retrieval search seam over KeywordIndex"
```

---

## Task 6: Wire `search_notes` through retrieval

**Files:**
- Modify: `src-tauri/src/state.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/commands/notes.rs`

- [ ] **Step 1: Add the keyword index to `AppState`**

In `src-tauri/src/state.rs`, replace the file with:

```rust
//! Application state: the injected ports the command layer delegates to.

use std::sync::Arc;

use raki_ai::EgressPolicy;
use raki_domain::{Clock, EmbeddingProvider, KeywordIndex, NoteRepository};

pub struct AppState {
    pub notes: Arc<dyn NoteRepository>,
    pub keyword: Arc<dyn KeywordIndex>,
    pub clock: Arc<dyn Clock>,
    pub embedder: Arc<dyn EmbeddingProvider>,
    pub egress: EgressPolicy,
}
```

- [ ] **Step 2: Construct it in the composition root**

In `src-tauri/src/lib.rs`, update the imports and the `setup` closure. Change the storage import line:

```rust
use raki_storage::{Database, SqliteKeywordIndex, SqliteNoteRepository};
```

and replace the `db`/`notes`/`app.manage(...)` block inside `setup`:

```rust
            let db = Database::open(&dir.join("raki.sqlite"))?;
            let notes = Arc::new(SqliteNoteRepository::new(db.clone()));
            let keyword = Arc::new(SqliteKeywordIndex::new(db));

            app.manage(AppState {
                notes,
                keyword,
                clock: Arc::new(SystemClock),
                embedder: Arc::new(FakeEmbeddingProvider::new(384)),
                egress: EgressPolicy::LocalOnly,
            });
            Ok(())
```

- [ ] **Step 3: Rewrite `search_notes` to use retrieval + hydrate**

In `src-tauri/src/commands/notes.rs`, replace the whole `search_notes` function:

```rust
/// Keyword search via FTS5: retrieval ranks ids, then we hydrate them to DTOs.
/// (Hydration is one `get` per hit; fine at k = 20, personal scale.)
#[tauri::command]
pub async fn search_notes(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<NoteDto>, AppError> {
    let ids = raki_retrieval::search(state.keyword.as_ref(), &query, 20).await?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let note_id = NoteId::parse(&id)?;
        if let Some(note) = state.notes.get(&note_id).await? {
            out.push(NoteDto::from(note));
        }
    }
    Ok(out)
}
```

Note: `raki_retrieval::search` returns `Result<_, DomainError>`, and `AppError: From<DomainError>` already exists, so `?` maps it. `NoteId` is already imported in this file (used by `get_note`).

- [ ] **Step 4: Verify the backend compiles, tests pass, lints clean**

Run: `cd src-tauri && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: all green (no new tests here; this is wiring covered end-to-end in Task 7's manual check).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src
git commit -m "Wire search_notes through FTS5 keyword retrieval"
```

---

## Task 7: Search box in the notes UI

**Files:**
- Modify: `src/modules/notes/api.ts`, `src/modules/notes/NotesView.tsx`
- Create: `src/modules/notes/api.test.ts`

- [ ] **Step 1: Write the failing test**

Create `src/modules/notes/api.test.ts`:

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

const searchNotes = vi.fn();
const listNotes = vi.fn();
const createNote = vi.fn();
vi.mock("~/shared/ipc", () => ({
  commands: { searchNotes, listNotes, createNote },
}));

import { notesApi } from "./api";

describe("notesApi", () => {
  beforeEach(() => {
    searchNotes.mockReset();
    listNotes.mockReset();
  });

  it("search delegates to the searchNotes command with the query", async () => {
    searchNotes.mockResolvedValue([]);
    await notesApi.search("apples");
    expect(searchNotes).toHaveBeenCalledWith("apples");
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bun run test`
Expected: FAIL — `notesApi.search` is not a function.

- [ ] **Step 3: Add `search` to the notes api**

In `src/modules/notes/api.ts`, replace the file with:

```ts
import { commands, type CreateNoteInput } from "~/shared/ipc";

export const notesKeys = {
  all: ["notes"] as const,
  search: (q: string) => ["notes", "search", q] as const,
};

export const notesApi = {
  list: () => commands.listNotes(),
  create: (input: CreateNoteInput) => commands.createNote(input),
  search: (query: string) => commands.searchNotes(query),
};
```

- [ ] **Step 4: Run test to verify it passes**

Run: `bun run test`
Expected: PASS — both the existing ipc test and the new api test pass.

- [ ] **Step 5: Add the search box to `NotesView`**

In `src/modules/notes/NotesView.tsx`, replace the file with:

```tsx
import { createSignal, For, Show } from "solid-js";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { notesApi, notesKeys } from "./api";

export function NotesView() {
  const queryClient = useQueryClient();
  const [title, setTitle] = createSignal("");
  const [search, setSearch] = createSignal("");

  const notes = createQuery(() => {
    const q = search().trim();
    return {
      queryKey: q ? notesKeys.search(q) : notesKeys.all,
      queryFn: () => (q ? notesApi.search(q) : notesApi.list()),
    };
  });

  const createNote = createMutation(() => ({
    mutationFn: () => notesApi.create({ title: title(), body: "{}" }),
    onSuccess: () => {
      setTitle("");
      queryClient.invalidateQueries({ queryKey: notesKeys.all });
    },
  }));

  return (
    <section>
      <h1>Notes</h1>

      <input
        type="search"
        placeholder="Search notes…"
        value={search()}
        onInput={(e) => setSearch(e.currentTarget.value)}
      />

      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (title().trim()) createNote.mutate();
        }}
      >
        <input
          placeholder="New note title…"
          value={title()}
          onInput={(e) => setTitle(e.currentTarget.value)}
        />
        <button type="submit" disabled={createNote.isPending}>
          Add
        </button>
      </form>

      <Show when={!notes.isLoading} fallback={<p>Loading…</p>}>
        <ul>
          <For each={notes.data ?? []}>{(n) => <li>{n.title}</li>}</For>
        </ul>
      </Show>
    </section>
  );
}
```

- [ ] **Step 6: Verify frontend typecheck, tests, build**

Run: `bun run typecheck && bun run test && bun run build`
Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add src/modules/notes
git commit -m "Add keyword search box to the notes view"
```

---

## Task 8: End-to-end verification + Definition of Done

- [ ] **Step 1: Full workspace + frontend sweep**

Run: `cd src-tauri && cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all tests pass, fmt clean, no clippy warnings.
Run (repo root): `bun run typecheck && bun run test && bun run build`
Expected: all green.

- [ ] **Step 2: Manual end-to-end smoke test**

Run: `bun run tauri dev`
Expected: create a few notes with distinct titles (e.g. "Apples", "Oranges"). Type "apples" in the search box → only the Apples note shows. Clear the search → all notes show. Delete-by-soft-delete is not yet in the UI, so verify deletion via search isn't testable here — just confirm search filters live notes correctly.

> If `tauri dev` cannot run in this environment, this step is performed by the user; report it as deferred rather than claiming success (`superpowers:verification-before-completion`).

- [ ] **Step 3: Commit any final cleanup (if Step 1 required a fmt pass)**

```bash
git add -A
git commit -m "Keyword retrieval (FTS5): final verification"
```

---

## Self-Review

**Spec coverage:**
- FTS5 index + backfill → Task 2.
- Atomic note+FTS writes (one transaction, `AGENT.md §5`) → Task 3 (upsert + soft_delete).
- Read-only `KeywordIndex` port → Task 1; `SqliteKeywordIndex` bm25 query → Task 4.
- Injection-safe query input → Task 4 (`fts_query`, tested with a stray quote).
- Retrieval seam (RRF-ready) → Task 5.
- Rewired `search_notes` (replaces naive substring) → Task 6.
- Search UI → Task 7.
- Tests at every layer (migration, storage writes, adapter query, retrieval seam, frontend api) + DoD sweep → Tasks 2–8.

**Deferred (named, not gaps):** `sqlite-vec` vector index, embeddings, true hybrid fusion, prefix/fuzzy matching — the next plan. Hydration is intentionally one `get` per hit at k=20 (documented in Task 6).

**Placeholder scan:** none — every code step has complete code; the only forward-references ("when a VectorIndex lands, fuse here") are deliberate, named deferrals.

**Type consistency:**
- `KeywordIndex::query(&str, usize) -> Result<Vec<KeywordHit>, DomainError>` defined (Task 1), implemented (Task 4 `SqliteKeywordIndex`, retrieval-test fake Task 5), consumed (Task 5 `search`, Task 6 command).
- `KeywordHit { source_id: String, score: f32 }` consistent across Tasks 1, 4, 5.
- `raki_retrieval::search(&dyn KeywordIndex, &str, usize) -> Result<Vec<String>, DomainError>` defined Task 5, called Task 6.
- `AppState.keyword: Arc<dyn KeywordIndex>` defined Task 6 Step 1, constructed Task 6 Step 2, read Task 6 Step 3.
- `Note::new(...)` (from the prior simplify commit) reused in Task 4's test helper.
- Frontend `notesApi.search(query)` → `commands.searchNotes(query)` consistent Tasks 7 (api, test, view).

---

## Execution Handoff

(Presented to the user after saving.)
