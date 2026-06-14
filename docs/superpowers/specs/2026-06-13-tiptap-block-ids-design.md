# Design: TipTap editor + stable block IDs for Raki notes

**Date:** 2026-06-13  
**Status:** Draft — pending implementation plan  
**Depends on:** ADR-0004 (ProseMirror JSON canonical note format), Phase-1 retrieval closure  
**Enables:** Phase 2 cross-module linking graph, durable block-level chunk provenance

## Goal

Replace the plain `<textarea>` body editor with a **TipTap** (ProseMirror) block editor. The frontend edits the canonical ProseMirror JSON body directly, and every top-level block carries a **stable UUID**. Block IDs flow through chunking and into vector source IDs so that retrieval results can be provenanced to a specific block in a specific note.

## Out of scope (YAGNI)

- Tables, embeds, images, callouts, diagrams, drawings, whiteboards.
- Markdown round-trip import/export (Phase 2 follow-up).
- Wikilink autocomplete or typed properties (Phase 2).
- Real-time collaboration or multi-cursor editing.
- Mobile-optimized editing layout.
- Autosave / continuous persistence — this slice keeps explicit Save (see §Save behavior).

## Background

Raki's backend already stores note bodies as ProseMirror JSON (ADR-0004). The frontend currently edits a plain-text projection of that JSON and round-trips it through `text_to_body()` on every save, which **regenerates the document structure each time**. That makes block IDs unstable and breaks the foundation for block-level linking.

Because Raki has not been released to users, there is **no user-data migration requirement**. Existing local data may be rewritten or normalized on demand, but a storage-level migration is still required to clear stale vectors and hashes after the body/chunking format changes.

## Block ID strategy

Block IDs are stored as ProseMirror node attributes on every top-level block.

### JSON shape

```json
{
  "type": "doc",
  "content": [
    {
      "type": "paragraph",
      "attrs": { "blockId": "0192a8f4-4e3b-7d..." },
      "content": [{ "type": "text", "text": "Hello world" }]
    }
  ]
}
```

### Rules

| Operation | Behavior |
|---|---|
| New block | Assign a fresh UUID v7. |
| Existing block | Preserve `blockId` across saves and edits. |
| Split paragraph | Original block keeps its ID; the new block gets a fresh ID. |
| Merge paragraphs | Surviving block keeps its ID; the merged-away ID is discarded. |
| Legacy body (plain text or JSON without IDs) | Normalize on the backend: wrap plain text into paragraphs, assign IDs to all top-level blocks. |
| Empty body | Canonical empty doc: `{"type":"doc","content":[]}`. |
| Duplicate IDs within a doc | After every TipTap transaction, deduplicate `blockId` values and reassign fresh IDs to duplicates. |

### ID scope

IDs are unique **within the note**, not globally. Global uniqueness is unnecessary for block-level provenance because the link combines `note_id + block_id`.

## Architecture changes

| Crate / slice | Change |
|---|---|
| `raki-domain` | Add `block_id: Option<String>` to `Block`. Add `normalize_body(json)` and `assign_block_ids(doc)` helpers. Update `text_to_body()` to assign IDs. Update `body_to_blocks()` to extract `blockId`. |
| `raki-storage` | No `notes` schema change. Body remains `TEXT`. Add a migration/dev backfill that clears `embedded_hash` and deletes `chunk_vectors` rows so the indexer rebuilds with the new source-id format. |
| `raki-memory` | `chunk_note` returns `Vec<Chunk>` where each chunk carries its source `block_id`. `embed_one` builds vector source IDs as `{note_id}:{block_id}:{split_index}`. Keep note-level delete-and-reindex for simplicity. |
| `raki-retrieval` | Search results carry `block_id` provenance. |
| `raki-app` / DTOs | `CreateNoteInput.body`, `UpdateNoteInput.body`, and `NoteDto.body` are all canonical ProseMirror JSON strings. Add `NoteDto.body_text` for list/search previews. Commands validate/normalize input bodies. |
| `src/modules/notes` | Replace textarea `NoteEditor` with `TipTapEditor`. Add `BlockId` TipTap extension. Audit/regenerate all `NoteDto.body` consumers. Remove or migrate `NotesView.tsx` textarea editor. |

## Data flow

```
Backend NoteDto.body (JSON)  ──▶  TipTapEditor renders JSON
                              │
                              ▼
                        User edits; BlockId extension maintains IDs
                              │
                              ▼
                        onChange emits ProseMirror JSON string
                              │
                              ▼
                        update_note(id, title, body=JSON)
                              │
                              ▼
                        Backend validates/normalizes JSON, stores as-is
                              │
                              ▼
                        Indexer chunks by block_id, source ID = note:block:split
                              │
                              ▼
                        Retrieval results provenanced to block_id
```

## IPC contract

### DTOs

```rust
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct NoteDto {
    pub id: String,
    pub title: String,
    pub body: String,              // canonical ProseMirror JSON
    pub body_text: String,         // plain-text preview for lists/search
    // ... timestamps
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct CreateNoteInput {
    pub title: String,
    pub body: String,              // canonical ProseMirror JSON; empty doc accepted
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct UpdateNoteInput {
    pub id: String,
    pub title: String,
    pub body: String,              // canonical ProseMirror JSON
}
```

### Command behavior

- `create_note`: validate title; normalize `body` with `normalize_body`; create `Note`.
- `update_note`: validate title; fetch existing note; normalize `body` with `normalize_body`; call `note.edit(...)`; upsert. If the body fails validation, return a typed `AppError` (not silent normalization).
- `get_note`: return stored body. All writes normalize, so reads are already canonical.
- `list_notes`, `search_notes`: return `body_text` computed via `body_to_text`.

### Validation rules

- `body` must be valid JSON.
- Top-level `type` must be `"doc"`.
- Allowed top-level content node types: `paragraph`, `heading`, `bulletList`, `orderedList`, `codeBlock`.
- Maximum JSON byte length: **256 KB** (same as current body cap).
- Maximum top-level blocks: **1,024** (defensive cap).
- Malformed bodies are **rejected** with `AppError::Validation`, not silently flattened.

## Frontend component

### Dependencies

```json
{
  "@tiptap/core": "^2.11.0",
  "@tiptap/starter-kit": "^2.11.0",
  "@tiptap/extension-placeholder": "^2.11.0"
}
```

Use a vanilla TipTap `Editor` instance managed with Solid `createEffect`/`onCleanup`. Do not use `solid-tiptap` unless it is verified to support the installed TipTap major version.

### Props

```tsx
interface TipTapEditorProps {
  bodyJson: string;              // canonical ProseMirror JSON string
  onChange: (bodyJson: string) => void;
  placeholder?: string;
}
```

### BlockId extension

- Adds a `blockId` attribute to `paragraph`, `heading`, `bulletList`, `orderedList`, `codeBlock`.
- Parses/renders via `data-block-id` DOM attribute.
- Assigns fresh UUID v7 to new top-level blocks.
- After each transaction, scans all top-level block IDs and reassigns duplicates.

### Save behavior

- Keep **manual Save** for this slice.
- Track dirty state by comparing current editor JSON to last-saved JSON.
- Save button calls `onChange` only when dirty.
- Show "Saved" toast on success, error toast on failure.
- Defer autosave to a future UX slice.

## Backend domain helpers

### `normalize_body(body: &str) -> Result<String, DomainError>`

```rust
pub fn normalize_body(body: &str) -> Result<String, DomainError> {
    // 1. Parse input as JSON.
    // 2. Validate top-level shape (doc + allowed content types).
    // 3. Assign blockId to any top-level block missing one.
    // 4. Re-serialize to compact JSON string.
}
```

### `assign_block_ids(doc: &mut Value)`

Walk `doc["content"]`, and for every top-level block missing `attrs.blockId`, assign a UUID v7.

### `body_to_blocks(body: &str) -> Vec<Block>`

Update to return `Block { heading, text, block_id }` by reading `attrs.blockId`. Blocks without an ID are assigned one in-memory (with a warning log) so chunking never fails.

## Indexing / vector changes

### Chunk output

```rust
pub struct Chunk {
    pub block_id: String,
    pub text: String,
}
```

`chunk_note` returns `Vec<Chunk>`. Long blocks are still split by `cap_split`; the split index is tracked separately from `block_id`.

### Vector source ID

```
{note_id}:{block_id}:{split_index}
```

Example: `0000...:0192...:0`.

### Reconciliation

For this slice, keep the existing note-level re-index strategy:

1. On note save, `IndexingService` deletes all vectors with prefix `{note_id}:`.
2. Re-chunk and re-embed the note.
3. Upsert new vectors.

Future optimization: block-level diff to avoid re-embedding unchanged blocks.

### Embedding invalidation

Because the body format and source-id format change, every existing vector is stale. Add a one-time storage migration/dev backfill:

```sql
DELETE FROM chunk_vectors;
UPDATE notes SET embedded_hash = NULL;
UPDATE notes SET fts_updated = 0; -- or equivalent trigger
```

This is acceptable because Raki is unreleased. After this backfill, the normal indexing pipeline re-populates vectors.

## Migration / backfill

No user-data migration. A storage backfill is required:

- Clear `chunk_vectors` (source-id format changed).
- Clear `notes.embedded_hash` (raw body format changed).
- Optionally rewrite existing note bodies through `normalize_body` so persisted JSON already contains block IDs.

Since there are no released users, this can be a forward-only migration in `raki-storage`.

## Testing

| Level | Test |
|---|---|
| Domain unit | `normalize_body` accepts valid JSON; rejects malformed JSON; assigns IDs to legacy text; preserves existing IDs. |
| Domain unit | `body_to_blocks` returns `block_id` for each block; assigns IDs for missing blocks. |
| Command unit | `update_note` rejects invalid JSON body; `create_note` normalizes empty body to empty doc. |
| TipTap extension | Split/merge/dedupe operations preserve/assign IDs correctly; copy-paste duplicates are reassigned. |
| Component | `TipTapEditor` renders from JSON and emits JSON on explicit save. |
| Integration | After saving a note, `chunk_vectors` contains rows with `{note_id}:{block_id}:{split_index}` source IDs. |
| E2E / eval | Run `chunk-eval` and `real-eval`; verify no regression. |

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| TipTap bundle size | Start with `starter-kit` only; measure bundle before adding extensions. |
| Block ID instability on complex edits | Restrict MVP to paragraph/heading/list/code; add split/merge/copy-paste tests. |
| SolidJS integration edge cases | Use vanilla `Editor` instance if `solid-tiptap` is stale. |
| IPC contract breakage | Audit all `NoteDto.body` consumers; regenerate bindings; gate on `bun run typecheck`. |
| Full-corpus re-index on first run | Acceptable for unreleased app; backfill clears hashes/vectors. |

## Acceptance criteria

- [ ] `CreateNoteInput.body`, `UpdateNoteInput.body`, and `NoteDto.body` all carry canonical ProseMirror JSON.
- [ ] The note editor is a TipTap instance, not a textarea.
- [ ] Backend normalizes/validates bodies; frontend does not assign block IDs.
- [ ] Saving a note stores ProseMirror JSON with block IDs.
- [ ] Editing a paragraph does not change the block IDs of other paragraphs.
- [ ] Splitting a paragraph gives the new paragraph a new ID and keeps the old ID.
- [ ] Copy-pasting a block within a note creates a fresh ID for the duplicate.
- [ ] Invalid/malformed bodies are rejected, not silently flattened.
- [ ] `chunk_vectors` rows use source IDs in the form `{note_id}:{block_id}:{split_index}`.
- [ ] The note list shows a plain-text snippet via `NoteDto.body_text`.
- [ ] All existing frontend consumers of `NoteDto.body` are migrated; bindings regenerated; `bun run typecheck` passes.
- [ ] `chunk-eval` and `real-eval` continue to pass.
- [ ] No regression in `cargo test`, `bun run test`, `bun run typecheck`.

## Open questions for implementation planning

1. Use vanilla TipTap `Editor` or `solid-tiptap`?
2. Implement block ID assignment via `appendTransaction` or node input rules?
3. Do we keep the existing `NotesView.tsx` textarea editor or replace it entirely with `NoteEditor.tsx`?
