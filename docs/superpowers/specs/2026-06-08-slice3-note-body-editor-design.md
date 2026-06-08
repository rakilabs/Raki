# Slice 3 — Note Body Editor (Draft) Design

**Goal:** Make notes *end-to-end* by letting a user write, read, and edit a note's
body as plain text — so real content finally flows through the embedding → retrieval →
grounded-QA substrate instead of empty `"{}"` bodies. This is the **draft / minimum
note-content** capability, not a notes-app feature push.

**Why now:** AGENTS.md Phase-1 discipline: "build the first vertical slice (notes)
*end-to-end*." Today `create_note` hardcodes `body: "{}"` (`NotesView.tsx:26`) and the
list renders titles only — every note is bodiless, so retrieval has nothing to retrieve
and QA nothing to ground against. The whole substrate below is built and tested; only
real content is missing.

**Tech Stack:** Rust, `tauri` v2, `rusqlite`/FTS5/sqlite-vec, `ts-rs`, SolidJS,
`@tanstack/solid-query`, `vitest`.

**Predecessors:** Slice 1 (egress substrate), Slice 2/2a/2b (grounded cloud QA) — all
committed. ADR-0002 (sync-ready data model), ADR-0004 (ProseMirror JSON canonical body).

---

## The central decision: the domain kernel owns the body format

The note body is canonical **ProseMirror JSON** (ADR-0004). The user edits **plain
text**. Something must convert between them, and storage must index *flattened* text,
not raw JSON. The conversion is a **domain fact**, so it lives in `raki-domain` — not in
the frontend, not duplicated per-crate.

- The **IPC carries plain text.** `create_note`/`update_note` accept plain text and wrap
  it to canonical JSON server-side; `NoteDto.body` returns flattened text. The SolidJS
  editor is a dumb `<textarea>` that never sees ProseMirror JSON.
- One format-owner (`raki-domain`), three consumers (storage indexing, `raki-generate`
  QA context, the command layer) — so the definition can't drift.

**Rejected alternative — frontend owns the conversion:** leaks ProseMirror knowledge
into the UI for zero benefit in a plain-text draft, *and* still requires the Rust
flattener for indexing. Two flatteners, no upside.

**Rejected alternative — store raw plain text, defer ADR-0004:** simplest today, but a
future rich editor would need a body migration (plain-text → JSON). Storing canonical
JSON now honors "rich editor later, no migration": a WYSIWYG editor writes richer-but-
still-valid PM JSON, so no stored body ever needs converting.

---

## D1 — Domain: body conversion + `Note::edit`

In `raki-domain` (the body format's home), two pure functions plus an entity method.
**Dependency:** add `serde_json = { workspace = true }` to `raki-domain/Cargo.toml` — it
currently has `serde` only, but the converters parse/serialize JSON (review M3).

- `body_to_text(body: &str) -> String` — flatten a PM `doc` to text: join each
  block node's text with **`\n` between blocks** (so paragraph breaks survive as line
  breaks — this is what makes the editor round-trip faithfully), text nodes *within* a
  block concatenated directly. **This consolidates the existing `note_body_to_text` in
  `raki-generate`**, which is deleted in favor of the kernel function (its space-join
  becomes a newline-join; the QA flatten test is updated to match — semantically
  equivalent for embedding/LLM input). The function is **total and defensive** (review
  M2): it never panics on a structurally-odd but valid `doc` — a missing/empty `content`
  → `""`, any node without a text-extraction path is skipped (not unwrapped), achieved
  with safe accessors (`.as_str()`/`.as_array()`/`.and_then`, no `.unwrap()`).
  - **Legacy-body fix (review C1):** the empty marker `"{}"` and any empty/contentless
    `doc` map to `""`, NOT the raw string. Every pre-slice note has `body: "{}"`; without
    this they would display literal `{}` in the editor and a save would persist the string
    `"{}"` as content. Mapping to `""` makes legacy notes open empty and self-heal to a
    canonical empty `doc` on first save. Only genuinely non-JSON garbage falls back to raw.
- `text_to_body(text: &str) -> String` — wrap plain text into a canonical PM `doc`:
  one `paragraph` per line, each holding a single `text` node; empty input → an empty
  `doc` (`{"type":"doc","content":[]}`). `body_to_text(text_to_body(t)) == t` for
  line-separated plain text (exact round-trip — line breaks preserved).
- `Note::edit(&self, title: String, body: String, now_ms: i64) -> Note` — mirrors
  `Note::new`: preserves `id` and `created_at`, sets `title`/`body`, `updated_at =
  now_ms`, `version += 1`, `deleted_at` unchanged. The "what an edit is" rule lives on
  the entity, not in a command adapter.

## D2 — Storage: index flattened text, not raw JSON

`raki-storage` must index prose, not structure. Apply `raki_domain::body_to_text` at the
two index seams:

- FTS5 insert (`notes.rs`): `INSERT INTO notes_fts (note_id, title, body)` uses
  `body_to_text(&n.body)` for the body column.
- Embedding text (`indexing.rs` `list_pending`): `format!("{title}\n\n{}",
  body_to_text(&body))`.

`content_hash` stays over **raw** `(title, body)` — it is change-detection for *future*
edits, not search input. Legacy FTS rows still hold `"{}"` as their body; that is inert
(the tokenizer yields no terms from punctuation) and is refreshed to flattened prose the
next time the note is edited — no FTS migration needed.

**Re-embed invalidation — migration V6 (review C2).** Because `content_hash` is over raw
body, the flattener change does NOT flag existing notes stale, so they would keep
embeddings built from the old text (`"Title\n\n{}"`) while new notes embed the new text
(`"Title"`) — a corrupted, mixed-semantics vector index. Migration **V6:
`UPDATE notes SET embedded_hash = NULL;`** clears the staleness stamp so `list_pending`
re-lists every live note and the indexer re-embeds them all with the new flattener on next
startup. Forward-only, idempotent, with the mandated populated-fixture test (apply V1–V5,
seed embedded notes, run V6, assert all rows re-list as pending).

## D3 — Commands: `update_note` + plain-text IPC

- New DTO `UpdateNoteInput { id: String, title: String, body: String }` (ts-rs exported).
- **Input validation (review M1), shared by `create_note` and `update_note`:** trim the
  title; reject empty (`AppError{kind:"validation_error"}`); cap title ≤ 512 chars and
  body ≤ 256 KB (reject over-cap with the same error). Keeps a blank, unclickable row out
  of the list and bounds embedding cost.
- **Atomic, soft-delete-safe update (review C3).** Add a repository method
  `update(&Note) -> Result<bool, DomainError>` distinct from `upsert`: a single guarded
  statement (`... ON CONFLICT(id) DO UPDATE SET … WHERE deleted_at IS NULL`) returning
  whether a *live* row was written. It refreshes FTS like `upsert` but never resurrects —
  `upsert` remains the general (resurrection-capable) primitive used by `create`.
- `update_note(state, input) -> Result<NoteDto, AppError>`: validate → `NoteId::parse` →
  `state.notes.get` → **`not_found` if missing** → `existing.edit(title,
  text_to_body(body), now)` → `state.notes.update(&edited)`; **if it returns `false`
  (row gone/deleted between read and write), return `not_found`** rather than resurrect →
  `index.trigger()` → `NoteDto::from`. (The get→update race is not reachable in this
  slice — `soft_delete` is exposed by no command — but the guard is correct and cheap.)
- `create_note` wraps `text_to_body(input.body)` before `Note::new` (the form sends
  plain text, not `"{}"`).
- `NoteDto::from(note)` returns `body: body_to_text(&note.body)` so every read
  (`get`/`list`/`search`) yields editable plain text.
- Register `update_note` in `invoke_handler`.

## D4 — Frontend: master-detail editor in `NotesView`

`src/modules/notes/` only (one module — no cross-module imports; AGENTS.md §5):

- A `selectedId` signal. List rows become clickable; clicking selects a note. A row with
  a blank title renders **`(Untitled)`** so it stays clickable (review M1 defense).
- An editor pane (beside/below the list) bound to the selected note: a title `<input>`,
  a body `<textarea>` (seeded from `NoteDto.body`, already plain text), and an explicit
  **Save** button → `notesApi.update({ id, title, body })`. Save is disabled when the
  title is blank (mirrors the create form's existing guard), so the backend
  `validation_error` is a backstop, not the primary UX.
- On save success, invalidate the notes query so list + pane reflect the new state.
  Switching selection before saving discards the in-progress edit (draft simplicity).
- Extend `commands` (`shared/ipc`) + `notesApi` with `update`.

## D5 — Error handling

- Editing a missing or soft-deleted note → `not_found` `AppError`, surfaced as an inline
  error in the pane (no crash, no silent no-op, no resurrection — see D3's guarded update).
- Legacy `"{}"` body → `body_to_text` returns `""`, so the note opens **empty** (not
  literal `{}`) and self-heals to a canonical empty `doc` on first save (review C1).
- Malformed/odd-but-valid stored body → `body_to_text` is total (review M2): structurally
  unexpected `doc` JSON yields best-effort text or `""`, never a panic; genuinely non-JSON
  garbage falls back to the raw string and re-wraps as canonical JSON on save (self-healing).
- Empty/over-long title or over-cap body → `validation_error` (D3), not a write.
- Indexing failures remain isolated and logged (existing `IndexingService` behavior);
  a save succeeds even if the background re-embed later fails.

## D6 — Testing

- **Domain:** `body_to_text` (doc flatten, multi-node, multi-paragraph→`\n`, non-JSON
  fallback) **plus the review-driven cases: `"{}"`→`""` (C1), `{"type":"doc"}` with no
  `content`→`""` and a nested non-paragraph node skipped without panic (M2)**;
  `text_to_body` (single/multi-line, empty→empty `doc`); exact line-preserving round-trip;
  `Note::edit` (id/created_at preserved, `updated_at`/`version` bumped, `deleted_at`
  untouched).
- **Storage:** on a populated fixture, a note whose body is real PM JSON indexes its
  **prose** into FTS and into `PendingNote.text` — assert the structural keys
  (`"paragraph"`, `"type"`) do **not** appear and the real words do. **V6 migration test
  (C2):** apply V1–V5, seed notes with non-null `embedded_hash`, run V6, assert every live
  note re-lists as pending. **Guarded `update` test (C3):** updating a soft-deleted note
  returns `false`/`not_found` and does not resurrect it (FTS row stays absent).
- **Generate:** the flatten test updates its expected output to the newline-join;
  all other QA tests stay green after swapping to `raki_domain::body_to_text`.
- **App:** `update_note` happy path (title+body change persists, version bumps), the
  `not_found` path, and the `validation_error` path (empty title rejected).
- **Frontend:** `NotesView` — selecting a note populates the editor; Save delegates to
  `notesApi.update` with `{id, title, body}` and invalidates the query (mock `~/shared/ipc`).

---

## Out of scope (YAGNI — Phase-1 discipline)

Rich-text formatting (bold/headings/lists), markdown, attachments, tags, folders,
note-to-note linking, autosave, soft-delete UI, optimistic concurrency on `version`.
The storage *supports* edits and versioning; this slice does not surface those as
features beyond the draft editor.

## Definition of Done

- `cd src-tauri && cargo test --workspace --exclude raki && cargo build -p raki &&
  cargo clippy --workspace -- -D warnings && cargo fmt --check` — all pass.
- `bun run typecheck && bun run test && bun run build` — all pass.
- **Manual `tauri dev` walkthrough (required — not claimable from tests):** create a note
  → open it → type a body → Save → reopen it and see the body → **search for a word that
  appears only in the body and find the note** (proves clean indexing) → ask a QA
  question the body answers and get a grounded answer citing it. Open a **pre-existing**
  note (created before this slice) → it shows **empty** (not `{}`) and is still searchable
  by title (proves the V6 re-embed + C1 legacy handling). Existing list/search still work.
