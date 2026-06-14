# Design: TipTap editor + stable block IDs for Raki notes

**Date:** 2026-06-13  
**Status:** Draft — pending implementation plan  
**Depends on:** ADR-0004 (ProseMirror JSON canonical note format), Phase-1 retrieval closure  
**Enables:** Phase 2 cross-module linking graph, durable block-level chunk provenance

## Goal

Replace the plain `<textarea>` body editor with a **TipTap** (ProseMirror) block editor that edits Raki's canonical ProseMirror JSON body directly. Every top-level block carries a **stable UUID** so that chunk provenance and future block-level links survive edits.

## Non-goals (YAGNI)

- Tables, embeds, images, callouts, diagrams, drawings, whiteboards.
- Markdown round-trip import/export (Phase 2 follow-up).
- Wikilink autocomplete or typed properties (Phase 2).
- Real-time collaboration or multi-cursor editing.
- Mobile-optimized editing layout.

## Background

Raki's backend already stores note bodies as ProseMirror JSON (ADR-0004). The frontend currently edits a plain-text projection of that JSON and round-trips it through `text_to_body()` on every save, which **regenerates the document structure each time**. That makes block IDs unstable and breaks the foundation for block-level linking.

Because Raki has not been released to users, there is **no migration requirement**. Existing local data may be rewritten or normalized on demand.

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
| Legacy body (plain text or JSON without IDs) | Normalize on load: wrap plain text into paragraphs, assign IDs to all top-level blocks. |
| Empty body | Canonical empty doc: `{"type":"doc","content":[]}`. |

### ID scope

IDs are unique **within the note**, not globally. Global uniqueness is unnecessary for block-level provenance because the link combines `note_id + block_id`.

## Architecture changes

| Crate / slice | Change |
|---|---|
| `raki-domain` | Add `block_id: Option<String>` to `Block`. Add `normalize_body(json)` and `assign_block_ids(doc)` helpers. Update `text_to_body()` to assign IDs. |
| `raki-storage` | No schema change. Body remains `TEXT`. |
| `raki-memory` / `raki-retrieval` | Chunking already uses `blocks_from_body()`; it now receives stable IDs. No logic change beyond reading the field. |
| `raki-app` / DTOs | `NoteDto.body` becomes the canonical ProseMirror JSON string. Add `NoteDto.body_text` for list/search previews. |
| `src/modules/notes` | Replace textarea `NoteEditor` with `TipTapEditor`. Add `BlockId` TipTap extension. |

## Data flow

```
Backend NoteDto.body  ──▶  frontend normalize (assign IDs if missing)
                              │
                              ▼
                        TipTapEditor renders JSON
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
                        Backend stores JSON string as-is
                              │
                              ▼
                        Indexer re-chunks with stable block IDs
```

## Frontend component

### Dependencies

```json
{
  "@tiptap/core": "^3.25.0",
  "@tiptap/starter-kit": "^3.25.0",
  "@tiptap/extension-placeholder": "^3.25.0",
  "solid-tiptap": "^0.8.0"
}
```

Use `solid-tiptap` only if it supports TipTap v3 cleanly; otherwise manage a vanilla `Editor` instance with Solid `createEffect`/`onCleanup`.

### Props

```tsx
interface TipTapEditorProps {
  bodyJson: string;              // canonical ProseMirror JSON string
  onChange: (bodyJson: string) => void;
  placeholder?: string;
}
```

### BlockId extension (sketch)

```ts
const BlockId = Extension.create({
  name: "blockId",
  addGlobalAttributes() {
    return [
      {
        types: ["paragraph", "heading", "bulletList", "orderedList", "codeBlock"],
        attributes: {
          blockId: {
            default: null,
            parseHTML: (el) => el.getAttribute("data-block-id"),
            renderHTML: (attrs) => ({ "data-block-id": attrs.blockId }),
          },
        },
      },
    ];
  },
  // New top-level blocks receive an ID via appendTransaction or nodeViews.
});
```

The exact mechanism (appendTransaction vs inputRules) will be decided during implementation.

### Save behavior

- Debounce `onChange` at **500 ms**.
- Autosave only when the document has actually changed.
- Show a subtle "Saved" status via the existing toast system.

## Backend domain helpers

### `normalize_body(body: &str) -> String`

```rust
pub fn normalize_body(body: &str) -> String {
    // 1. Try to parse as ProseMirror JSON.
    // 2. If valid, ensure every top-level block has a blockId.
    // 3. If invalid, treat as plain text and build a ProseMirror doc with one paragraph per line, each with a new blockId.
}
```

### `assign_block_ids(doc: &mut Value)`

Walk `doc["content"]`, and for every top-level block missing `attrs.blockId`, assign a UUID v7.

## DTO changes

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
```

The list view and search results use `body_text`. The editor uses `body`.

## Migration (none)

Because Raki is unreleased, no explicit migration is required. Bodies that are plain text or lack block IDs are normalized on first load in the frontend.

If a future release needs to migrate user data, the `normalize_body` helper is the tool.

## Testing

| Level | Test |
|---|---|
| Domain unit | `normalize_body` assigns IDs to legacy text; preserves IDs in valid JSON; handles empty doc. |
| Domain unit | `blocks_from_body` returns `block_id` for each block. |
| TipTap extension | Split/merge operations preserve/assign IDs correctly. |
| Component | `TipTapEditor` renders from JSON and emits JSON on change. |
| E2E / eval | Run chunk-eval and verify that editing a note does not change the IDs of untouched blocks. |

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| TipTap adds bundle size | Start with `starter-kit` only; measure bundle before adding extensions. |
| Block ID instability on complex edits | Restrict MVP to paragraph/heading/list/code; add split/merge tests. |
| SolidJS integration edge cases | Prefer vanilla `Editor` if `solid-tiptap` is stale. |
| Search/list previews | Add `body_text` to `NoteDto` so UI never parses JSON. |

## Acceptance criteria

- [ ] The note editor is a TipTap instance, not a textarea.
- [ ] Saving a note stores ProseMirror JSON with block IDs.
- [ ] Editing a paragraph does not change the block IDs of other paragraphs.
- [ ] Splitting a paragraph gives the new paragraph a new ID and keeps the old ID.
- [ ] The note list still shows a plain-text snippet.
- [ ] `chunk-eval` and `real-eval` continue to pass.
- [ ] No regression in `cargo test`, `bun run test`, `bun run typecheck`.

## Open questions for implementation planning

1. Use `solid-tiptap` or vanilla TipTap `Editor`?
2. Implement block ID assignment via `appendTransaction` or TipTap node input rules?
3. Should `body_text` be computed on the frontend from `body`, or backend-derived in `NoteDto`?
4. Do we keep the existing `NotesView.tsx` textarea editor or replace it entirely with `NoteEditor.tsx`?
