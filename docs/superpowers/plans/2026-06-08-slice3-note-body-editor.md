# Slice 3 — Note Body Editor (Draft) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a user write, read, and edit a note's body as plain text — stored as canonical ProseMirror JSON — so real content finally flows through embedding → retrieval → grounded-QA instead of empty `"{}"` bodies.

**Architecture:** `raki-domain` owns the body format (`body_to_text`/`text_to_body`, total + defensive) and the edit rule (`Note::edit`). Storage indexes *flattened* prose at both seams (FTS + embedding text) and gains an atomic, soft-delete-safe `update`. A V6 migration re-embeds the corpus under the new flattener. The app exposes a thin `update_note` command (plain-text IPC, validated); the SolidJS notes module renders a master-detail editor.

**Tech Stack:** Rust, `tauri` v2, `rusqlite`/FTS5/sqlite-vec, `ts-rs` 12, SolidJS, `@tanstack/solid-query`, `vitest`, `@solidjs/testing-library`.

**Spec:** `docs/superpowers/specs/2026-06-08-slice3-note-body-editor-design.md` (central decision + D1–D6). Spec review (CONDITIONAL GO) applied: C1 (`"{}"`→`""`), C2 (V6 re-embed), C3 (guarded `update`), M1 (validation + `(Untitled)`), M2 (total flattener + tests), M3 (`serde_json` dep).

---

## Verified facts (read before starting)

- **Note entity** (`raki-domain/src/note.rs`): `Note { id: NoteId, title, body, created_at, updated_at, deleted_at: Option<i64>, version: i64 }`, derives `Clone`. `Note::new(title, body, now_ms)` is infallible (fresh id, version 1). No edit method yet.
- **`NoteRepository`** (`raki-domain/src/ports.rs:21`): `upsert`, `get`, `list`, `soft_delete`. **Three impls total:** `SqliteNoteRepository` (`raki-storage/src/notes.rs:39`), and test fakes `OneNote` + `EmptyRepo` (`raki-generate/src/lib.rs:263,280`). Adding a trait method breaks all three.
- **`get`/`list`** already filter `WHERE deleted_at IS NULL`. **`upsert`** (`notes.rs:40`) is `INSERT … ON CONFLICT(id) DO UPDATE …` and *intentionally* resurrects (FTS comment) — leave it as the create/resurrect primitive.
- **FTS seam** (`notes.rs:61`): `INSERT INTO notes_fts (note_id, title, body) VALUES (?1, ?2, ?3)` with **raw** `n.body`. **Embed seam** (`raki-storage/src/indexing.rs:84`): `text: format!("{title}\n\n{body}")` with **raw** `body`. `content_hash(&n.title, &n.body)` (`hash.rs`) is over raw — leave it.
- **Migrations** (`raki-storage/src/migrations.rs:6`): `const MIGRATIONS: &[&str]`, V1–V5 present, applied by index+1 vs `PRAGMA user_version`. V6 is the next element. Populated-fixture test pattern: `v5_grounded_column_applies_to_a_populated_egress_log` (apply `MIGRATIONS[0..5]` is wrong there — it uses `[0..4]` then `migrate`; for V6 apply `[0..5]`, stamp `user_version=5`, seed, then `migrate`). `register_sqlite_vec()` is needed because V3 builds a `vec0` table.
- **Flattener today** (`raki-generate/src/lib.rs:64` `note_body_to_text`): private; space-joins; used at `lib.rs:124` in `assemble_for`; has two unit tests (`prosemirror_body_is_flattened_to_text` @460, `plain_text_body_passes_through_unchanged` @471). `OneNote.get` returns a **plain-text** body (`"Pay cash at the ryokan."`), so no QA *flow* test asserts flattened output — only those two unit tests reference the private fn.
- **DTO/IPC** (`src-tauri/src/dto.rs`): `NoteDto { id, title, body, created_at:#[ts(type="number")] i64, updated_at:… }`, `impl From<Note>`; `CreateNoteInput { title, body }`. Export attr: `#[ts(export, export_to = "../../src/shared/ipc/bindings/")]`. Bindings regenerate on `cargo test -p raki`.
- **Errors** (`src-tauri/src/error.rs`): `AppError { kind: String, message: String }`; `DomainError` variants `NotFound | Invalid | Storage | Provider`; `NoteId::parse(&str) -> Result<_, DomainError>` (so `?` maps via `From<DomainError>`).
- **Commands** (`src-tauri/src/commands/notes.rs`): `#[tauri::command] pub async fn name(state: State<'_, AppState>, …) -> Result<_, AppError>`; `state.notes.upsert`, `state.index.trigger()`, `state.clock.now_ms()`. Handlers registered in `src-tauri/src/lib.rs` `tauri::generate_handler![…]`.
- **Frontend**: `src/shared/ipc/index.ts` (typed `commands`), `src/modules/notes/api.ts` (`notesApi` + `notesKeys`), `NotesView.tsx` (create form hardcodes `body: "{}"`; list renders `{n.title}` only). `App.tsx` already mounts `<NotesView/>` — **no shell change needed** (the editor lives inside the notes module). Scripts: `bun run typecheck | test | build`.
- **Cargo:** `raki-domain/Cargo.toml` has `serde` but **not** `serde_json` (must add). `serde_json` IS in `[workspace.dependencies]` (used by `raki-generate`).

---

## File Structure

```
raki-domain/Cargo.toml                  MODIFY  add serde_json
raki-domain/src/body.rs                 CREATE  body_to_text + text_to_body (+ tests)
raki-domain/src/note.rs                 MODIFY  Note::edit
raki-domain/src/lib.rs                  MODIFY  pub mod body; re-export converters
raki-domain/src/ports.rs                MODIFY  NoteRepository::update
raki-generate/src/lib.rs                MODIFY  use domain body_to_text; delete private fn + 2 tests; stub update on fakes
raki-storage/src/notes.rs               MODIFY  flatten FTS body; impl guarded update (+ tests)
raki-storage/src/indexing.rs            MODIFY  flatten embed text
raki-storage/src/migrations.rs          MODIFY  V6 re-embed migration (+ test)
src-tauri/src/dto.rs                     MODIFY  UpdateNoteInput; NoteDto/From flatten
src-tauri/src/commands/notes.rs          MODIFY  validate; update_note; create_note wrap
src-tauri/src/lib.rs                     MODIFY  register update_note
src/shared/ipc/index.ts                  MODIFY  updateNote
src/modules/notes/api.ts                 MODIFY  notesApi.update
src/modules/notes/api.test.ts            MODIFY  update delegation test
src/modules/notes/NotesView.tsx          MODIFY  master-detail editor
src/modules/notes/NotesView.test.tsx     CREATE  editor component tests
src/shared/ipc/bindings/UpdateNoteInput.ts  GENERATED by ts-rs
```

**Rollback:** Tasks are independent commits. Task 1 is pure kernel (no consumers break). If Task 4's trait change misbehaves, `git revert` it plus Task 5 (the only caller). Frontend (6–7) reverts without touching Rust.

---

## Task 1: Domain — body converters + `Note::edit`

**Files:** Modify `raki-domain/Cargo.toml`, `raki-domain/src/lib.rs`, `raki-domain/src/note.rs`; Create `raki-domain/src/body.rs`.

- [ ] **Step 1: Add the `serde_json` dependency (review M3)**

In `src-tauri/crates/raki-domain/Cargo.toml`, under `[dependencies]` after `serde = { workspace = true }`:

```toml
serde_json = { workspace = true }
```

- [ ] **Step 2: Write the failing converter tests**

Create `src-tauri/crates/raki-domain/src/body.rs` with ONLY the tests first (the `use super::*` will fail to resolve the fns — that is the red):

```rust
//! Conversion between the canonical ProseMirror-JSON note body (ADR-0004) and the plain
//! text the editor works in. These are the single, total definitions shared by storage
//! indexing, QA context assembly, and the command layer, so the format rule cannot drift.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flattens_a_doc_blocks_with_newlines_text_nodes_directly() {
        let doc = r#"{"type":"doc","content":[
            {"type":"paragraph","content":[{"type":"text","text":"Pay cash"},{"type":"text","text":" at the ryokan."}]},
            {"type":"paragraph","content":[{"type":"text","text":"Checkout is 10am."}]}
        ]}"#;
        assert_eq!(body_to_text(doc), "Pay cash at the ryokan.\nCheckout is 10am.");
    }

    #[test]
    fn empty_marker_and_empty_doc_are_blank_not_raw() {
        // review C1: legacy "{}" must NOT surface as literal text.
        assert_eq!(body_to_text("{}"), "");
        assert_eq!(body_to_text(r#"{"type":"doc","content":[]}"#), "");
    }

    #[test]
    fn doc_without_content_is_blank_and_nested_nodes_are_walked_without_panic() {
        // review M2: total/defensive on odd-but-valid doc JSON.
        assert_eq!(body_to_text(r#"{"type":"doc"}"#), "");
        let nested = r#"{"type":"doc","content":[
            {"type":"bulletList","content":[
                {"type":"listItem","content":[
                    {"type":"paragraph","content":[{"type":"text","text":"hi"}]}]}]}]}"#;
        assert_eq!(body_to_text(nested), "hi");
    }

    #[test]
    fn non_json_falls_back_to_raw() {
        assert_eq!(body_to_text("just plain text"), "just plain text");
    }

    #[test]
    fn text_to_body_round_trips_line_separated_text() {
        for t in ["", "one line", "a\nb", "a\n\nb"] {
            assert_eq!(body_to_text(&text_to_body(t)), t, "round-trip {t:?}");
        }
    }

    #[test]
    fn text_to_body_emits_a_canonical_doc() {
        assert_eq!(text_to_body(""), r#"{"content":[],"type":"doc"}"#);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cd src-tauri && cargo test -p raki-domain --lib body`
Expected: FAIL — `cannot find function body_to_text` / `text_to_body`.

- [ ] **Step 4: Implement the converters (prepend above the test module in `body.rs`)**

```rust
use serde_json::{json, Value};

/// Flatten a canonical ProseMirror `doc` to plain text: each top-level block's text joined
/// with `\n` between blocks; text nodes within a block concatenated directly (their own
/// spacing is preserved). Total and defensive — never panics:
/// - the empty marker `"{}"`, an empty `doc`, or a contentless `doc` → `""` (review C1)
/// - structurally-odd but valid JSON → best-effort text, unknown nodes skipped (review M2)
/// - genuinely non-JSON input → returned verbatim (a legacy/plain body stays editable)
pub fn body_to_text(body: &str) -> String {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return body.to_string(); // not JSON at all → treat as raw text
    };
    if value.get("type").and_then(Value::as_str) != Some("doc") {
        return String::new(); // any non-doc JSON (incl. `{}`) carries no editor text
    }
    let mut blocks: Vec<String> = Vec::new();
    if let Some(content) = value.get("content").and_then(Value::as_array) {
        for block in content {
            let mut text = String::new();
            collect_text(block, &mut text);
            blocks.push(text);
        }
    }
    blocks.join("\n")
}

/// Depth-first collect every `text` node's string (no separators — block separation is the
/// caller's job). Skips any node without a text payload; recurses through `content`.
fn collect_text(node: &Value, out: &mut String) {
    if let Some(t) = node.get("text").and_then(Value::as_str) {
        out.push_str(t);
    }
    if let Some(content) = node.get("content").and_then(Value::as_array) {
        for child in content {
            collect_text(child, out);
        }
    }
}

/// Wrap plain editor text into a canonical ProseMirror `doc`: one `paragraph` per line,
/// each holding a single `text` node (empty lines → empty paragraphs). Empty input → an
/// empty `doc`. Inverse of `body_to_text` for line-separated plain text.
pub fn text_to_body(text: &str) -> String {
    let content: Vec<Value> = if text.is_empty() {
        Vec::new()
    } else {
        text.split('\n')
            .map(|line| {
                if line.is_empty() {
                    json!({ "type": "paragraph" })
                } else {
                    json!({ "type": "paragraph", "content": [{ "type": "text", "text": line }] })
                }
            })
            .collect()
    };
    json!({ "type": "doc", "content": content }).to_string()
}
```

- [ ] **Step 5: Wire the module + re-export**

In `raki-domain/src/lib.rs`, add `pub mod body;` (after `pub mod ports;`) and extend the re-exports:

```rust
pub mod body;
pub use body::{body_to_text, text_to_body};
```

- [ ] **Step 6: Run the converter tests to verify they pass**

Run: `cd src-tauri && cargo test -p raki-domain --lib body`
Expected: PASS — 6 tests.

- [ ] **Step 7: Write the failing `Note::edit` test**

In `raki-domain/src/note.rs`, append a test module:

```rust
#[cfg(test)]
mod edit_tests {
    use super::*;

    #[test]
    fn edit_preserves_identity_and_bumps_version() {
        let original = Note::new("Trip".into(), "old".into(), 1000);
        let edited = original.edit("Trip v2".into(), "new".into(), 2000);
        assert_eq!(edited.id, original.id, "id preserved");
        assert_eq!(edited.created_at, 1000, "created_at preserved");
        assert_eq!(edited.title, "Trip v2");
        assert_eq!(edited.body, "new");
        assert_eq!(edited.updated_at, 2000);
        assert_eq!(edited.version, 2, "version bumped");
        assert_eq!(edited.deleted_at, None, "liveness preserved");
    }
}
```

- [ ] **Step 8: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-domain --lib edit_tests`
Expected: FAIL — `no method named edit`.

- [ ] **Step 9: Implement `Note::edit`**

In `raki-domain/src/note.rs`, inside `impl Note` (after `new`):

```rust
    /// Apply an edit: new `title`/`body`, `updated_at = now_ms`, `version` bumped. Preserves
    /// `id`, `created_at`, and `deleted_at`. The "what an edit is" rule lives here, not in a
    /// command adapter.
    pub fn edit(&self, title: String, body: String, now_ms: i64) -> Note {
        Note {
            id: self.id.clone(),
            title,
            body,
            created_at: self.created_at,
            updated_at: now_ms,
            deleted_at: self.deleted_at,
            version: self.version + 1,
        }
    }
```

- [ ] **Step 10: Verify + commit**

Run: `cd src-tauri && cargo test -p raki-domain && cargo clippy -p raki-domain -- -D warnings`
Expected: PASS, no warnings.

```bash
git add src-tauri/crates/raki-domain
git commit -m "raki-domain: body_to_text/text_to_body converters + Note::edit"
```

---

## Task 2: Generate — consolidate the flattener onto the kernel

**Files:** Modify `raki-generate/src/lib.rs`.

- [ ] **Step 1: Swap the call site to the kernel function**

In `raki-generate/src/lib.rs`, add `body_to_text` to the existing `use raki_domain::{…}` import. Then at the `assemble_for` call site (`lib.rs:124`) replace `note_body_to_text(&note.body)` with `body_to_text(&note.body)`:

```rust
            text: format!("{}\n{}", note.title, body_to_text(&note.body)),
```

- [ ] **Step 2: Delete the private flattener + its two unit tests**

Remove the whole `fn note_body_to_text(body: &str) -> String { … }` (its `extract_text` inner fn and all). In the test module remove `prosemirror_body_is_flattened_to_text` and `plain_text_body_passes_through_unchanged` (now owned by `raki-domain`'s `body` tests). Leave every other test.

- [ ] **Step 3: Verify the QA flow is unchanged**

Run: `cd src-tauri && cargo test -p raki-generate`
Expected: PASS — flow/groundedness/preview tests green. (`OneNote`'s body is plain text, so `body_to_text` returns it verbatim; assembled context is identical.)

- [ ] **Step 4: Lint + commit**

Run: `cd src-tauri && cargo clippy -p raki-generate -- -D warnings`
Expected: no warnings (no dead `note_body_to_text`).

```bash
git add src-tauri/crates/raki-generate/src/lib.rs
git commit -m "raki-generate: use raki_domain::body_to_text; drop the duplicate flattener"
```

---

## Task 3: Storage — flatten both index seams + V6 re-embed migration

**Files:** Modify `raki-storage/src/notes.rs`, `raki-storage/src/indexing.rs`, `raki-storage/src/migrations.rs`.

- [ ] **Step 1: Write the failing "FTS indexes prose, not JSON" test**

In `raki-storage/src/notes.rs` test module, add (it asserts the *stored FTS body* is flattened):

```rust
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
        assert!(!fts_body.contains("paragraph"), "no structural keys in the index");
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-storage --lib fts_body_is_flattened`
Expected: FAIL — `fts_body` is the raw JSON (`{"content":[…`), not `"pay cash…"`.

- [ ] **Step 3: Flatten the FTS seam**

In `raki-storage/src/notes.rs`: add `body_to_text` to the `use raki_domain::{…}` import. In `upsert`, change the FTS insert (currently `params![id, n.title, n.body]`) to flatten the body:

```rust
                    tx.execute(
                        "INSERT INTO notes_fts (note_id, title, body) VALUES (?1, ?2, ?3)",
                        params![id, n.title, body_to_text(&n.body)],
                    )?;
```

- [ ] **Step 4: Flatten the embedding seam**

In `raki-storage/src/indexing.rs`: add `use raki_domain::body_to_text;` (or extend the existing `raki_domain` import). In `list_pending`'s row map, change the `text` field:

```rust
                            text: format!("{title}\n\n{}", body_to_text(&body)),
```

- [ ] **Step 5: Run to verify the seam test passes (and nothing regressed)**

Run: `cd src-tauri && cargo test -p raki-storage`
Expected: PASS — new test green; existing `upsert_indexes_into_fts`, indexing tests still green (their plain-text/`"body"` bodies flatten to themselves).

- [ ] **Step 6: Write the failing V6 migration test (review C2)**

In `raki-storage/src/migrations.rs` test module, add:

```rust
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
            .query_row("SELECT embedded_hash FROM notes WHERE id = 'n1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(embedded_hash, None, "V6 clears the staleness stamp → note re-lists as pending");
    }
```

- [ ] **Step 7: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-storage --lib v6_re_embed`
Expected: FAIL — `embedded_hash` is still `'h'` (no V6 yet).

- [ ] **Step 8: Add the V6 migration**

In `raki-storage/src/migrations.rs`, append to the `MIGRATIONS` array (after V5, before the closing `];`):

```rust
    // V6: the body flattener changed (space-join → newline-join, and "{}" → empty), so the
    // text we embed changed for every note while content_hash (over raw body) did not.
    // Clear the staleness stamp to force a clean re-embed of the whole corpus on next start.
    "UPDATE notes SET embedded_hash = NULL;",
```

- [ ] **Step 9: Verify + commit**

Run: `cd src-tauri && cargo test -p raki-storage && cargo clippy -p raki-storage -- -D warnings`
Expected: PASS — V6 + seam tests green, no warnings.

```bash
git add src-tauri/crates/raki-storage/src
git commit -m "raki-storage: index flattened body (FTS + embeddings); V6 re-embed migration"
```

---

## Task 4: Repository `update` — atomic, soft-delete-safe (review C3)

**Files:** Modify `raki-domain/src/ports.rs`, `raki-storage/src/notes.rs`, `raki-generate/src/lib.rs`.

- [ ] **Step 1: Add the trait method (breaks all impls — that's the red)**

In `raki-domain/src/ports.rs`, in `trait NoteRepository`, after `upsert`:

```rust
    /// Update an existing **live** note in place. Returns `false` when no live row matched
    /// (missing or soft-deleted) — the caller treats that as not-found and never resurrects.
    /// Distinct from `upsert`, which deliberately creates/resurrects.
    async fn update(&self, note: &Note) -> Result<bool, DomainError>;
```

- [ ] **Step 2: Stub it on the two test fakes**

In `raki-generate/src/lib.rs`, in `impl NoteRepository for OneNote` and `impl NoteRepository for EmptyRepo`, add (after their `upsert`):

```rust
        async fn update(&self, _: &Note) -> Result<bool, DomainError> {
            Ok(true)
        }
```

- [ ] **Step 3: Write the failing Sqlite `update` tests**

In `raki-storage/src/notes.rs` test module:

```rust
    #[tokio::test]
    async fn update_changes_a_live_note_and_refreshes_fts() {
        use raki_domain::text_to_body;
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let note = Note::new("Trip".into(), text_to_body("old"), 1000);
        repo.upsert(&note).await.unwrap();

        let edited = note.edit("Trip".into(), text_to_body("new plan cash"), 2000);
        assert!(repo.update(&edited).await.unwrap(), "live row updated");

        let got = repo.get(&note.id).await.unwrap().unwrap();
        assert_eq!(got.body, text_to_body("new plan cash"));
        assert_eq!(got.version, 2);
        assert_eq!(fts_count(&db, &note.id.to_string()).await, 1, "still indexed");
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
        assert_eq!(fts_count(&db, &note.id.to_string()).await, 0, "not re-indexed");
    }
```

- [ ] **Step 4: Run them to verify they fail**

Run: `cd src-tauri && cargo test -p raki-storage --lib update_`
Expected: FAIL — `SqliteNoteRepository` has no `update` (only the trait + stubs exist).

- [ ] **Step 5: Implement the guarded `update` on `SqliteNoteRepository`**

In `raki-storage/src/notes.rs`, in `impl NoteRepository for SqliteNoteRepository`, after `upsert`:

```rust
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
```

- [ ] **Step 6: Verify the whole backend compiles + passes (all three crates)**

Run: `cd src-tauri && cargo test -p raki-domain -p raki-storage -p raki-generate && cargo clippy -p raki-storage -p raki-generate -- -D warnings`
Expected: PASS — `update_` tests green; generate fakes satisfy the trait.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/crates/raki-domain/src/ports.rs src-tauri/crates/raki-storage/src/notes.rs src-tauri/crates/raki-generate/src/lib.rs
git commit -m "NoteRepository::update — atomic, soft-delete-safe (no resurrection)"
```

---

## Task 5: App — `UpdateNoteInput` DTO, validation, `update_note`, plain-text IPC

**Files:** Modify `src-tauri/src/dto.rs`, `src-tauri/src/commands/notes.rs`, `src-tauri/src/lib.rs`.

- [ ] **Step 1: Add the `UpdateNoteInput` DTO + flatten `NoteDto`**

In `src-tauri/src/dto.rs`, add the input type (after `CreateNoteInput`):

```rust
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct UpdateNoteInput {
    pub id: String,
    pub title: String,
    pub body: String,
}
```

And change `impl From<Note> for NoteDto` so every read returns editable plain text:

```rust
impl From<Note> for NoteDto {
    fn from(n: Note) -> Self {
        NoteDto {
            id: n.id.to_string(),
            title: n.title,
            body: raki_domain::body_to_text(&n.body),
            created_at: n.created_at,
            updated_at: n.updated_at,
        }
    }
}
```

- [ ] **Step 2: Add validation + `update_note`; wrap `create_note` (review M1, C1, C3)**

In `src-tauri/src/commands/notes.rs`: extend imports and add the helper + command, and wrap create. Replace the import line `use raki_domain::{Note, NoteId};` with:

```rust
use raki_domain::{text_to_body, Note, NoteId};

use crate::dto::{CreateNoteInput, NoteDto, UpdateNoteInput};
```

(Keep the existing `use crate::dto::{CreateNoteInput, NoteDto};` replaced by the line above — do not duplicate.) Add the shared validator near the top:

```rust
const MAX_TITLE_CHARS: usize = 512;
const MAX_BODY_BYTES: usize = 256 * 1024;

/// Boundary validation shared by create + update (review M1). Trims the title for the
/// emptiness check; the caller stores the trimmed title.
fn validate(title: &str, body: &str) -> Result<(), AppError> {
    let t = title.trim();
    if t.is_empty() {
        return Err(AppError { kind: "validation_error".into(), message: "title must not be empty".into() });
    }
    if t.chars().count() > MAX_TITLE_CHARS {
        return Err(AppError { kind: "validation_error".into(), message: "title too long".into() });
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(AppError { kind: "validation_error".into(), message: "body too long".into() });
    }
    Ok(())
}
```

Replace the body of `create_note` to validate + wrap plain text to canonical JSON:

```rust
#[tauri::command]
pub async fn create_note(
    state: State<'_, AppState>,
    input: CreateNoteInput,
) -> Result<NoteDto, AppError> {
    validate(&input.title, &input.body)?;
    let note = Note::new(
        input.title.trim().to_string(),
        text_to_body(&input.body),
        state.clock.now_ms(),
    );
    state.notes.upsert(&note).await?;
    state.index.trigger();
    Ok(NoteDto::from(note))
}
```

Add the new command (after `get_note`):

```rust
#[tauri::command]
pub async fn update_note(
    state: State<'_, AppState>,
    input: UpdateNoteInput,
) -> Result<NoteDto, AppError> {
    validate(&input.title, &input.body)?;
    let nid = NoteId::parse(&input.id)?;
    let existing = state
        .notes
        .get(&nid)
        .await?
        .ok_or_else(|| AppError { kind: "not_found".into(), message: "note not found".into() })?;
    let edited = existing.edit(
        input.title.trim().to_string(),
        text_to_body(&input.body),
        state.clock.now_ms(),
    );
    // Atomic guarded update: false ⇒ the row was deleted between read and write — do not resurrect.
    if !state.notes.update(&edited).await? {
        return Err(AppError { kind: "not_found".into(), message: "note not found".into() });
    }
    state.index.trigger();
    Ok(NoteDto::from(edited))
}
```

- [ ] **Step 3: Register the handler**

In `src-tauri/src/lib.rs`, add `update_note` to the notes import and the `generate_handler!`:

```rust
use crate::commands::notes::{create_note, get_note, list_notes, search_notes, update_note};
```
```rust
        .invoke_handler(tauri::generate_handler![
            create_note, list_notes, get_note, search_notes, update_note,
            answer_question, grant_cloud_consent, revoke_cloud_consent
        ])
```

(Match the existing import/handler lines exactly; add `update_note` to each.)

- [ ] **Step 4: Build, generate bindings, inspect**

Run: `cd src-tauri && cargo build -p raki && cargo test -p raki 2>&1 | tail -5 && cargo clippy -p raki -- -D warnings`
Then confirm the binding emitted (review-style check):
Run (repo root): `cat src/shared/ipc/bindings/UpdateNoteInput.ts`
Expected: `export type UpdateNoteInput = { id: string, title: string, body: string, };`

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src src/shared/ipc/bindings
git commit -m "App: update_note command + plain-text IPC (validated); NoteDto returns flat text"
```

---

## Task 6: Frontend — typed `updateNote` + `notesApi.update`

**Files:** Modify `src/shared/ipc/index.ts`, `src/modules/notes/api.ts`, `src/modules/notes/api.test.ts`.

- [ ] **Step 1: Extend the typed command surface**

In `src/shared/ipc/index.ts`, import the new binding and add the command:

```ts
import type { UpdateNoteInput } from "./bindings/UpdateNoteInput";
```
Add `UpdateNoteInput` to the `export type { … }` line, and inside `commands`:

```ts
  updateNote: (input: UpdateNoteInput) => invoke<NoteDto>("update_note", { input }),
```

- [ ] **Step 2: Add `notesApi.update`**

In `src/modules/notes/api.ts`, extend the import and the object:

```ts
import { commands, type CreateNoteInput, type UpdateNoteInput } from "~/shared/ipc";
```
```ts
  update: (input: UpdateNoteInput) => commands.updateNote(input),
```

- [ ] **Step 3: Write the failing delegation test**

In `src/modules/notes/api.test.ts`, add `updateNote: vi.fn()` to the `commands` mock object, then add:

```ts
  it("update delegates to the updateNote command with the input", async () => {
    mocked.updateNote.mockResolvedValue({ id: "n1", title: "t", body: "b", created_at: 0, updated_at: 1 });
    await notesApi.update({ id: "n1", title: "t", body: "b" });
    expect(mocked.updateNote).toHaveBeenCalledWith({ id: "n1", title: "t", body: "b" });
  });
```

- [ ] **Step 4: Verify + commit**

Run (repo root): `bun run test && bun run typecheck`
Expected: PASS — delegation test green, `tsc` resolves `UpdateNoteInput`.

```bash
git add src/shared/ipc/index.ts src/modules/notes/api.ts src/modules/notes/api.test.ts
git commit -m "Frontend: typed updateNote command + notesApi.update"
```

---

## Task 7: Frontend — master-detail editor in `NotesView`

**Files:** Modify `src/modules/notes/NotesView.tsx`; Create `src/modules/notes/NotesView.test.tsx`.

- [ ] **Step 1: Write the failing component tests**

Create `src/modules/notes/NotesView.test.tsx`:

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, screen, waitFor } from "@solidjs/testing-library";
import { QueryClient, QueryClientProvider } from "@tanstack/solid-query";

vi.mock("./api", () => ({
  notesKeys: { all: ["notes"], search: (q: string) => ["notes", "search", q] },
  notesApi: { list: vi.fn(), create: vi.fn(), search: vi.fn(), update: vi.fn() },
}));

import { notesApi } from "./api";
import { NotesView } from "./NotesView";

const mocked = vi.mocked(notesApi);

function renderView() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(() => (
    <QueryClientProvider client={client}>
      <NotesView />
    </QueryClientProvider>
  ));
}

describe("NotesView editor", () => {
  beforeEach(() => vi.clearAllMocks());

  it("selecting a note populates the editor and Save delegates to update", async () => {
    mocked.list.mockResolvedValue([
      { id: "n1", title: "Trip", body: "Pay cash", created_at: 0, updated_at: 0 },
    ]);
    mocked.update.mockResolvedValue({ id: "n1", title: "Trip", body: "Pay card", created_at: 0, updated_at: 1 });
    renderView();

    fireEvent.click(await screen.findByRole("button", { name: "Trip" }));
    const body = (await screen.findByLabelText("Body")) as HTMLTextAreaElement;
    expect(body.value).toBe("Pay cash");

    fireEvent.input(body, { target: { value: "Pay card" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() =>
      expect(mocked.update).toHaveBeenCalledWith({ id: "n1", title: "Trip", body: "Pay card" }),
    );
  });

  it("renders (Untitled) for a blank-title note", async () => {
    mocked.list.mockResolvedValue([{ id: "n2", title: "  ", body: "", created_at: 0, updated_at: 0 }]);
    renderView();
    expect(await screen.findByRole("button", { name: "(Untitled)" })).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: Run them to verify they fail**

Run (repo root): `bun run test -- NotesView`
Expected: FAIL — no list buttons / no `Body` field (current `NotesView` renders `{n.title}` as plain text, no editor).

- [ ] **Step 3: Rewrite `NotesView.tsx` with the master-detail editor**

Replace `src/modules/notes/NotesView.tsx` with:

```tsx
import { createSignal, createEffect, For, Show } from "solid-js";
import { createQuery, createMutation, useQueryClient } from "@tanstack/solid-query";
import { notesApi, notesKeys } from "./api";
import type { NoteDto } from "~/shared/ipc";

export function NotesView() {
  const queryClient = useQueryClient();
  const [title, setTitle] = createSignal("");
  const [search, setSearch] = createSignal("");
  const [debouncedSearch, setDebouncedSearch] = createSignal("");
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [editTitle, setEditTitle] = createSignal("");
  const [editBody, setEditBody] = createSignal("");

  createEffect(() => {
    const q = search();
    const timer = setTimeout(() => setDebouncedSearch(q.trim()), 150);
    return () => clearTimeout(timer);
  });

  const notes = createQuery(() => {
    const q = debouncedSearch();
    return {
      queryKey: q ? notesKeys.search(q) : notesKeys.all,
      queryFn: () => (q ? notesApi.search(q) : notesApi.list()),
    };
  });

  const selected = (): NoteDto | undefined =>
    (notes.data ?? []).find((n) => n.id === selectedId());

  // Seed the editor fields whenever the selected note (or its server copy) changes.
  createEffect(() => {
    const n = selected();
    if (n) {
      setEditTitle(n.title);
      setEditBody(n.body);
    }
  });

  const createNote = createMutation(() => ({
    mutationFn: () => notesApi.create({ title: title(), body: "" }),
    onSuccess: () => {
      setTitle("");
      queryClient.invalidateQueries({ queryKey: notesKeys.all });
    },
  }));

  const saveNote = createMutation(() => ({
    mutationFn: () =>
      notesApi.update({ id: selectedId()!, title: editTitle(), body: editBody() }),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: notesKeys.all }),
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

      <div class="notes-layout">
        <Show when={!notes.isLoading} fallback={<p>Loading…</p>}>
          <ul>
            <For each={notes.data ?? []}>
              {(n) => (
                <li>
                  <button type="button" onClick={() => setSelectedId(n.id)}>
                    {n.title.trim() || "(Untitled)"}
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>

        <Show when={selected()}>
          <form
            class="note-editor"
            onSubmit={(e) => {
              e.preventDefault();
              if (editTitle().trim()) saveNote.mutate();
            }}
          >
            <input
              aria-label="Title"
              value={editTitle()}
              onInput={(e) => setEditTitle(e.currentTarget.value)}
            />
            <textarea
              aria-label="Body"
              value={editBody()}
              onInput={(e) => setEditBody(e.currentTarget.value)}
            />
            <button type="submit" disabled={saveNote.isPending || !editTitle().trim()}>
              Save
            </button>
          </form>
        </Show>
      </div>
    </section>
  );
}
```

- [ ] **Step 4: Run the component tests to verify they pass**

Run (repo root): `bun run test -- NotesView`
Expected: PASS — both editor tests green.

- [ ] **Step 5: Full frontend gate + commit**

Run (repo root): `bun run typecheck && bun run test && bun run build`
Expected: all green.

```bash
git add src/modules/notes/NotesView.tsx src/modules/notes/NotesView.test.tsx
git commit -m "Frontend: master-detail note editor (select → edit body → save)"
```

---

## Task 8: Verification + Definition of Done

- [ ] **Step 1: Full workspace + app build clean**

Run: `cd src-tauri && cargo test --workspace --exclude raki && cargo build -p raki && cargo clippy --workspace -- -D warnings && cargo fmt --check`
Expected: all pass.

- [ ] **Step 2: Frontend checks**

Run (repo root): `bun run typecheck && bun run test && bun run build`
Expected: all green.

- [ ] **Step 3: Binding present**

Run (repo root): `ls src/shared/ipc/bindings/UpdateNoteInput.ts`
Expected: file exists.

- [ ] **Step 4: MANUAL `tauri dev` walkthrough (REQUIRED — not claimable from a test run)**

Per `verification-before-completion`: this slice touches the CI-excluded app + frontend, so completion is **not** claimed until the user confirms it in the running app. `bun run tauri dev`, then:

*Editor + clean indexing:*
  1. Create a note (title only) → it appears in the list.
  2. Click it → the editor pane opens; type a multi-line body → **Save**.
  3. Click another note then back → the body persists and shows line breaks intact.
  4. **Search for a word that appears only in the body** → the note is found (proves flattened FTS indexing, not JSON noise).
  5. (If cloud QA is enabled) ask a question the body answers → grounded answer cites the note (proves the body reached embeddings).

*Legacy + regression (review C1/C2/M1):*
  6. Open a **pre-existing** note (created before this slice) → it shows **empty** (not literal `{}`) and is still findable by title.
  7. A blank-title note shows as **(Untitled)** and is still clickable.
  8. Existing list / search / create still work.

**STOP. Do not mark this slice complete until the user reports the walkthrough passed.**

- [ ] **Step 5: DoD against the spec**

Central decision (domain owns format; plain-text IPC; dumb textarea) ✓ T1,T5,T7 · D1 (converters + `Note::edit`; serde_json M3) ✓ T1 · D2 (flatten both seams; V6 re-embed C2) ✓ T3 · D3 (`update_note`, validation M1, guarded `update` C3, plain-text IPC) ✓ T4,T5 · D4 (master-detail, `(Untitled)`, Save guard) ✓ T7 · D5 (not_found, legacy `{}`→empty C1, total flattener M2) ✓ T1,T5 · D6 (all test classes) ✓ T1–T7.

---

## Self-Review

**Spec coverage:** central decision → T1/T5/T7; D1 → T1; D2 → T3; D3 → T4+T5; D4 → T7; D5 → T1+T5; D6 → spread. All six review conditions (C1 `{}`→`""` T1; C2 V6 T3; C3 guarded `update` T4; M1 validation+`(Untitled)` T5+T7; M2 total flattener+tests T1; M3 serde_json T1) have tasks.

**Placeholder scan:** none — every code step has complete code; every run step has an exact command + expected output. The "match the existing import/handler lines exactly" notes in T5 are precision instructions, not deferred work.

**Type/consistency:** `body_to_text(&str)->String`, `text_to_body(&str)->String`, `Note::edit(&self,String,String,i64)->Note`, `NoteRepository::update(&Note)->Result<bool,DomainError>` used identically across T1/T3/T4/T5. `UpdateNoteInput{id,title,body}` ↔ frontend `notesApi.update({id,title,body})` ↔ `commands.updateNote(input)` → `invoke("update_note",{input})`. `AppError{kind,message}` literals match the existing struct. `NoteDto.body` is now flattened text everywhere it's read. V6 is `MIGRATIONS[5]`; the test applies `[0..5]` then stamps `user_version=5` (consistent with the V5 test's resume pattern). `OneNote`/`EmptyRepo` gain the `update` stub so `raki-generate` still compiles after the trait grows.

**Known confirmations (read-and-match at implementation time):** the exact existing `use`/`generate_handler!` lines in `commands/notes.rs` + `lib.rs` (add `update_note`, don't duplicate); that no other QA flow test asserts a flattened body string (`OneNote`'s body is plain text — verified); `@solidjs/testing-library` exports `render/fireEvent/screen/waitFor/findByRole/findByLabelText` (standard API, as used by the existing `AskBox.test.tsx`).

---

## Execution Handoff

(Presented to the user after saving.)
