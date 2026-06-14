# Spec Review: TipTap Editor + Stable Block IDs

**Spec:** `docs/superpowers/specs/2026-06-13-tiptap-block-ids-design.md`  
**Review date:** 2026-06-13  
**Effort:** medium (4 finders, 1 verifier pass)  
**Verdict:** CONDITIONAL GO — spec direction is sound, but two CRITICAL and seven MAJOR gaps must be fixed before implementation planning.

## Summary

The spec correctly identifies the core problem (the textarea round-trips plain text through `text_to_body`, destroying stable block structure) and proposes the right high-level solution (TipTap editor + ProseMirror JSON bodies + per-block IDs). However, it understates the contract changes across the IPC boundary and the retrieval/indexing changes needed for block IDs to actually matter. Several architectural-rule gaps and scope ambiguities would cause implementation to fail or drift.

## Verified findings

### CRITICAL

#### 1. Input contract undefined: `CreateNoteInput`/`UpdateNoteInput` still treated as plain text

- **Quote:** "`NoteDto.body` becomes the canonical ProseMirror JSON string" and "Backend stores JSON string as-is"
- **Issue:** The spec updates the *output* DTO but never updates the *input* DTOs or the command implementations. Current `src-tauri/src/commands/notes.rs:165` and `:213` call `text_to_body(&input.body)`. If the editor sends JSON, the backend will wrap the literal JSON string in a paragraph node, corrupting the body and losing block IDs.
- **Fix:** Explicitly state that `CreateNoteInput.body` and `UpdateNoteInput.body` now carry canonical ProseMirror JSON. Remove `text_to_body()` from commands and use `normalize_body()` for validation/normalization before persistence. Add a round-trip regression test.

#### 2. Block IDs do not reach the vector index

- **Quote:** "Chunking already uses `blocks_from_body()`; it now receives stable IDs. No logic change beyond reading the field."
- **Issue:** Current `raki-memory/src/indexing.rs:87-92` builds vector source IDs as `{note_id}#{chunk_index}` and deletes all vectors for a note on re-index. Without changing the source-id format to `{note_id}#{block_id}` and the deletion strategy to block-level diff, block IDs provide no durable provenance.
- **Fix:** Either (a) narrow this slice to "swap editor to TipTap with JSON body" and defer stable block IDs to Phase 2, or (b) fully specify the new `Block` shape, chunk source-id format, vector deletion/reconciliation strategy, and an embedding-invalidation plan.

### MAJOR

#### 3. Frontend normalization violates AGENTS.md §5

- **Quote:** "Backend NoteDto.body ──▶ frontend normalize (assign IDs if missing)"
- **Issue:** Assigning block IDs in the frontend duplicates domain logic in components and contradicts "No business logic in components."
- **Fix:** Backend normalizes on read (`get_note`) and write (`create_note`/`update_note`). Frontend receives valid canonical JSON and emits it unchanged.

#### 4. Concurrent autosaves and save-failure handling unspecified

- **Quote:** "Debounce `onChange` at **500 ms**. Autosave only when the document has actually changed. Show a subtle 'Saved' status via the existing toast system."
- **Issue:** No serialization, optimistic-concurrency handling, retry, or error UX is defined. With `version`-based concurrency, overlapping autosaves can fail silently or lose edits.
- **Fix:** Specify in-flight-save serialization, `version` mismatch behavior, and error UX. Consider keeping manual Save for this slice.

#### 5. Embedding invalidation/backfill strategy missing

- **Quote:** "Migration (none): Because Raki is unreleased, no explicit migration is required."
- **Issue:** Switching to TipTap JSON changes raw `(title, body)`, so every existing note mismatches `content_hash` and forces a full-corpus re-embed. Old `chunk_vectors` rows keyed by chunk index may be orphaned.
- **Fix:** State how `embedded_hash`/`chunk_vectors` are invalidated and rebuilt, even if no user migration is needed.

#### 6. Validation boundaries for JSON bodies unspecified

- **Quote:** (omission)
- **Issue:** No max JSON bytes, unsupported node types, or malformed-JSON behavior is defined. `validate()` in `commands/notes.rs:136-157` only checks string length today.
- **Fix:** Add validation rules: allow-listed top-level node types, byte/block count limits, and whether malformed bodies are rejected or normalized.

#### 7. `NoteDto.body` breaking change leaves frontend consumers mismatched

- **Quote:** "`NoteDto.body` becomes the canonical ProseMirror JSON string. Add `NoteDto.body_text` for list/search previews."
- **Issue:** `NotesView.tsx` and `NoteEditor.tsx` currently bind `body` to textareas as plain text. The spec does not require auditing/regenerating all consumers and bindings.
- **Fix:** Add acceptance criterion: "All `NoteDto.body` consumers except the TipTap editor are migrated to `body_text`; bindings regenerated; `bun run typecheck` passes."

#### 8. Copy-paste/drag-drop/undo can create duplicate block IDs

- **Quote:** "IDs are unique within the note, not globally." and BlockId extension sketch
- **Issue:** Clone operations duplicate `attrs.blockId`, violating within-note uniqueness.
- **Fix:** Add a post-transaction deduplication pass in the BlockId extension and a unit test for copy-paste.

#### 9. Retrieval logic change is understated

- **Quote:** "Chunking already uses `blocks_from_body()`; it now receives stable IDs. No logic change beyond reading the field."
- **Issue:** `Block` struct, `body_to_blocks`, and chunk source-ID generation all need changes. This is not a read-only change.
- **Fix:** Specify exact changes to `raki-domain::Block`, `body_to_blocks`, and `raki-memory` indexing.

## Refuted findings

- **`body_text` is YAGNI:** REFUTED. The acceptance criterion "The note list still shows a plain-text snippet" requires it, and the current UI only shows titles.
- **Autosave/debounce is scope creep:** REFUTED. The spec explicitly defines the save behavior, though it should also be reflected in acceptance criteria.

## Required spec revisions before GO

1. Decide whether this slice is **(A) editor swap only** or **(B) editor swap + durable block-level provenance**. If (A), remove block-ID claims from acceptance criteria. If (B), fully specify vector source IDs and indexing changes.
2. Update IPC input contracts (`CreateNoteInput`, `UpdateNoteInput`) and command implementations.
3. Move normalization to the backend.
4. Define validation rules and error UX for JSON bodies.
5. Define save concurrency semantics or keep manual Save.
6. Add embedding invalidation/backfill plan.
7. Add frontend-consumer migration acceptance criterion.
8. Address duplicate block IDs from clone operations.

## Memory update

- Block-level features that claim provenance must specify the full path from editor node → domain block → chunk source ID → vector ID. "Stable IDs" are not sufficient without indexing/reconciliation changes.
- Any body-format change must explicitly invalidate `content_hash` / `embedded_hash` because the hash covers raw `(title, body)`.
- Editor swaps are IPC-contract changes: both input and output DTOs, command adapters, generated bindings, and all frontend consumers must be audited.
