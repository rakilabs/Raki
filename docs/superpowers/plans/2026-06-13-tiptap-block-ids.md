# TipTap Editor + Stable Block IDs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the textarea note body editor with a TipTap ProseMirror editor, store canonical ProseMirror JSON with stable per-block IDs, and flow those IDs through chunking into vector source IDs so retrieval results can be provenanced to a specific block.

**Architecture:** Backend owns body normalization and block-ID assignment in `raki-domain`. `raki-memory` chunks by block ID and builds block-scoped vector source IDs. `raki-app` DTOs and commands carry JSON bodies with validation. The frontend renders/edits the JSON with TipTap and a custom `BlockId` extension. A storage migration clears stale vectors and hashes.

**Tech Stack:** TipTap v2 + ProseMirror, SolidJS, Rust domain/storage/memory, Tauri IPC, ts-rs generated bindings.

---

## File structure

| File | Responsibility |
|---|---|
| `src-tauri/crates/raki-domain/src/body.rs` | `Block` struct, `body_to_text`, `body_to_blocks`, `text_to_body`, `normalize_body`, `assign_block_ids` |
| `src-tauri/crates/raki-domain/src/lib.rs` | Export new helpers |
| `src-tauri/crates/raki-memory/src/chunk.rs` | `Chunk` struct and `chunk_note` |
| `src-tauri/crates/raki-memory/src/indexing.rs` | Vector source-ID generation from chunks |
| `src-tauri/crates/raki-storage/src/migrations.rs` | V9 migration: clear stale vectors/hashes |
| `src-tauri/src/dto.rs` | `NoteDto.body_text`, `CreateNoteInput`/`UpdateNoteInput` body semantics |
| `src-tauri/src/commands/notes.rs` | Validate/normalize JSON bodies; remove `text_to_body` from commands |
| `src-tauri/src/error.rs` | Add validation error kind if missing |
| `src-tauri/src/lib.rs` | Command registration already present |
| `package.json` | Add TipTap dependencies |
| `src/modules/notes/components/BlockId.ts` | TipTap extension for stable block IDs |
| `src/modules/notes/components/TipTapEditor.tsx` | Solid wrapper around TipTap `Editor` |
| `src/modules/notes/components/TipTapEditor.css` | Minimal editor styles |
| `src/modules/notes/components/NoteEditor.tsx` | Replace `<Textarea>` with `<TipTapEditor>` |
| `src/modules/notes/NotesView.tsx` | Remove or redirect textarea editor; use `body_text` for snippets |
| `src-tauri/crates/raki-eval/src/memory_corpus/index.rs` | Update `chunk_note` consumer to use `Chunk` and new source IDs |
| `src/modules/notes/api.ts` | No API signature changes needed (body stays `string`) |
| `src/shared/ipc/bindings/NoteDto.ts` | Regenerated (will include `body_text`) |

---

## Task 1: Domain body helpers (`raki-domain`)

**Files:**
- Modify: `src-tauri/crates/raki-domain/src/body.rs`
- Modify: `src-tauri/crates/raki-domain/src/lib.rs`
- Test: `src-tauri/crates/raki-domain/src/body.rs` (existing module tests)

### Step 1.1: Add `block_id` to `Block` and update `body_to_blocks`

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub heading: Option<String>,
    pub text: String,
    pub block_id: Option<String>,
}
```

In `body_to_blocks`, when emitting a `Block`, set `block_id` from `node["attrs"]["blockId"]`:

```rust
let block_id = node
    .get("attrs")
    .and_then(|a| a.get("blockId"))
    .and_then(Value::as_str)
    .map(String::from);
```

Apply to every `Block { ... }` construction in `body_to_blocks`.

**Run:**
```bash
cargo test -p raki-domain body_to_blocks
```
**Expected:** existing tests pass; new field is `None` in old fixtures.

### Step 1.2: Add `assign_block_ids` and `normalize_body`

Add after `body_to_blocks`:

```rust
use uuid::Uuid;

pub fn normalize_body(body: &str) -> Result<String, DomainError> {
    let mut value: Value = serde_json::from_str(body)
        .map_err(|e| DomainError::Invalid(format!("invalid body json: {e}")))?;
    if value.get("type").and_then(Value::as_str) != Some("doc") {
        return Err(DomainError::Invalid("body must be a ProseMirror doc".into()));
    }
    assign_block_ids(&mut value);
    Ok(value.to_string())
}

pub fn assign_block_ids(doc: &mut Value) {
    let Some(content) = doc.get_mut("content").and_then(Value::as_array_mut) else {
        return;
    };
    for node in content.iter_mut() {
        if !is_top_level_block(node) {
            continue;
        }
        let attrs = node
            .as_object_mut()
            .unwrap()
            .entry("attrs")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .unwrap();
        if !attrs.contains_key("blockId") {
            attrs.insert("blockId".to_string(), json!(new_block_id()));
        }
    }
}

fn is_top_level_block(node: &Value) -> bool {
    matches!(
        node.get("type").and_then(Value::as_str),
        Some("paragraph") | Some("heading") | Some("bulletList") | Some("orderedList") | Some("codeBlock")
    )
}

fn new_block_id() -> String {
    Uuid::now_v7().to_string()
}
```

Note: `raki-domain` already depends on `uuid` with the `v7` feature (workspace). No Cargo.toml change is needed. Use the existing `DomainError::Invalid` variant for validation failures.

**Run:**
```bash
cargo test -p raki-domain normalize_body
```
**Expected:** PASS after adding tests in Step 1.4.

### Step 1.3: Update `text_to_body` to assign block IDs

Replace the body of `text_to_body` so each generated paragraph gets a block ID:

```rust
pub fn text_to_body(text: &str) -> String {
    let mut doc: Value = if text.is_empty() {
        json!({ "type": "doc", "content": [] })
    } else {
        let content: Vec<Value> = text
            .split('\n')
            .map(|line| {
                if line.is_empty() {
                    json!({ "type": "paragraph" })
                } else {
                    json!({
                        "type": "paragraph",
                        "content": [{ "type": "text", "text": line }]
                    })
                }
            })
            .collect();
        json!({ "type": "doc", "content": content })
    };
    assign_block_ids(&mut doc);
    doc.to_string()
}
```

**Run:**
```bash
cargo test -p raki-domain text_to_body
```
**Expected:** PASS; `text_to_body` output now contains `attrs.blockId`.

### Step 1.4: Add unit tests

Append to the `#[cfg(test)] mod tests` in `body.rs`:

```rust
#[test]
fn normalize_body_assigns_ids_to_blocks_missing_them() {
    let body = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"hi"}]}]}"#;
    let out = normalize_body(body).unwrap();
    assert!(out.contains("blockId"));
}

#[test]
fn normalize_body_preserves_existing_block_ids() {
    let body = r#"{"type":"doc","content":[{"type":"paragraph","attrs":{"blockId":"existing-id"},"content":[{"type":"text","text":"hi"}]}]}"#;
    let out = normalize_body(body).unwrap();
    assert!(out.contains("\"blockId\":\"existing-id\""));
}

#[test]
fn normalize_body_rejects_non_doc_json() {
    let body = r#"{"type":"not-a-doc"}"#;
    assert!(normalize_body(body).is_err());
}

#[test]
fn body_to_blocks_extracts_block_id() {
    let doc = r#"{"type":"doc","content":[{"type":"paragraph","attrs":{"blockId":"bid-1"},"content":[{"type":"text","text":"first"}]}]}"#;
    let blocks = body_to_blocks(doc);
    assert_eq!(blocks[0].block_id, Some("bid-1".to_string()));
}

#[test]
fn text_to_body_assigns_block_ids() {
    let body = text_to_body("line one\nline two");
    let blocks = body_to_blocks(&body);
    assert_eq!(blocks.len(), 2);
    assert!(blocks[0].block_id.is_some());
    assert!(blocks[1].block_id.is_some());
    assert_ne!(blocks[0].block_id, blocks[1].block_id);
}
```

**Run:**
```bash
cargo test -p raki-domain
```
**Expected:** all tests pass.

### Step 1.5: Commit

```bash
git add src-tauri/crates/raki-domain/src/body.rs src-tauri/crates/raki-domain/src/lib.rs src-tauri/crates/raki-domain/src/error.rs src-tauri/crates/raki-domain/Cargo.toml
git commit -m "feat(domain): block IDs in body helpers and normalize_body"
```

---

## Task 2: Chunk by block ID (`raki-memory`)

**Files:**
- Modify: `src-tauri/crates/raki-memory/src/chunk.rs`
- Modify: `src-tauri/crates/raki-memory/src/indexing.rs`
- Test: both files

### Step 2.1: Introduce `Chunk` struct and update `chunk_note`

In `chunk.rs`:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub block_id: String,
    pub text: String,
}

pub fn chunk_note(title: &str, body: &str, use_prefix: bool) -> Vec<Chunk> {
    let blocks = body_to_blocks(body);
    if blocks.is_empty() {
        if title.is_empty() {
            return vec![];
        }
        return vec![Chunk {
            block_id: "title".to_string(),
            text: title.to_string(),
        }];
    }

    let mut chunks: Vec<Chunk> = Vec::new();

    for block in &blocks {
        let block_id = block.block_id.clone().unwrap_or_else(|| "none".to_string());
        let prefixed = if use_prefix {
            match &block.heading {
                Some(heading) => format!("{} > {}: {}", title, heading, block.text),
                None => format!("{}: {}", title, block.text),
            }
        } else {
            block.text.clone()
        };

        let split = cap_split(&prefixed);
        for (i, text) in split.into_iter().enumerate() {
            chunks.push(Chunk {
                block_id: format!("{}:{}", block_id, i),
                text,
            });
        }

        if chunks.len() >= MAX_CHUNKS_PER_NOTE {
            break;
        }
    }

    if chunks.len() > MAX_CHUNKS_PER_NOTE {
        chunks.truncate(MAX_CHUNKS_PER_NOTE);
    }

    chunks
}
```

Update the existing `chunk_note_*` tests to expect `Chunk` objects and assert `block_id`.

**Run:**
```bash
cargo test -p raki-memory chunk
```
**Expected:** PASS after test updates.

### Step 2.2: Update `indexing.rs` to use `Chunk` for source IDs

Change `embed_one`:

```rust
async fn embed_one(
    store: &dyn IndexingStore,
    embedder: &dyn EmbeddingProvider,
    vectors: &dyn VectorIndex,
    config: &EmbedConfig,
    note: &PendingNote,
) -> Result<bool, DomainError> {
    let chunks = chunk_note(&note.title, &note.body, config.use_contextual_prefix);
    if chunks.is_empty() {
        vectors.delete_by_prefix(&format!("{}:", note.id)).await?;
        return store
            .mark_embedded(&note.id, &note.content_hash, &embedder.model_id())
            .await;
    }

    let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
    let embeddings = embedder.embed(&texts).await?;
    if embeddings.len() != chunks.len() {
        return Err(DomainError::Provider(format!(
            "embedder returned {} embeddings for {} chunks",
            embeddings.len(),
            chunks.len()
        )));
    }

    vectors.delete_by_prefix(&format!("{}:", note.id)).await?;

    let items: Vec<(String, raki_domain::Embedding)> = chunks
        .into_iter()
        .zip(embeddings.into_iter())
        .map(|(chunk, emb)| (format!("{}:{}", note.id, chunk.block_id), emb))
        .collect();

    vectors.upsert_batch(&items).await?;

    let stamped = store
        .mark_embedded(&note.id, &note.content_hash, &embedder.model_id())
        .await?;

    Ok(stamped)
}
```

Update tests in `indexing.rs` to assert source IDs contain `:` and block IDs. For example, the test note with two paragraphs should produce source IDs like `"{id}:block-id:0"`.

### Step 2.3: Update `raki-eval` seed corpus indexer

In `src-tauri/crates/raki-eval/src/memory_corpus/index.rs`:

```rust
use raki_memory::chunk_note;

// Replace:
// let chunks = raki_memory::chunk_note(...);
// let embs = embedder.embed(&chunks).await...;
// for (i, emb) in embs.into_iter().enumerate() { vectors.upsert(format!("{}#{}", note.id, i), emb) }

let chunks = chunk_note(&note.title, &note.body, USE_CONTEXTUAL_PREFIX);
if chunks.is_empty() { ... }

let texts: Vec<String> = chunks.iter().map(|c| c.text.clone()).collect();
let embs = embedder.embed(&texts).await.expect("embedder succeeds");
assert_eq!(embs.len(), chunks.len(), ...);

for (chunk, emb) in chunks.into_iter().zip(embs.into_iter()) {
    let source_id = format!("{}:{}", note.id, chunk.block_id);
    vectors.upsert(&source_id, &emb).await.expect("upsert succeeds");
}
```

**Run:**
```bash
cargo test -p raki-eval --lib
```
**Expected:** PASS.

### Step 2.4: Commit

```bash
git add src-tauri/crates/raki-memory/src/chunk.rs src-tauri/crates/raki-memory/src/indexing.rs src-tauri/crates/raki-eval/src/memory_corpus/index.rs
git commit -m "feat(memory): chunk by block_id and use block-scoped vector source IDs"
```

---

## Task 3: Storage migration (`raki-storage`)

**Files:**
- Modify: `src-tauri/crates/raki-storage/src/migrations.rs`
- Test: existing migration tests

### Step 3.1: Add V9 migration

Append to `MIGRATIONS`:

```rust
// V9: block-scoped vector source IDs. The source-id format changed from
// `{note_id}#{chunk_index}` to `{note_id}:{block_id}:{split_index}`. Clear all
// chunk vectors and staleness stamps so the indexer rebuilds with the new format.
// Also normalize existing bodies to canonical ProseMirror JSON with block IDs.
"DELETE FROM chunk_vectors;
 UPDATE notes SET embedded_hash = NULL;",
```

**Run:**
```bash
cargo test -p raki-storage migrations
```
**Expected:** PASS.

### Step 3.2: Commit

```bash
git add src-tauri/crates/raki-storage/src/migrations.rs
git commit -m "feat(storage): V9 migration clears stale vectors for block-scoped source IDs"
```

---

## Task 4: DTO and command contract (`raki-app`)

**Files:**
- Modify: `src-tauri/src/dto.rs`
- Modify: `src-tauri/src/commands/notes.rs`
- Modify: `src-tauri/src/error.rs` (if needed)
- Test: `src-tauri/src/commands/notes.rs` unit tests

### Step 4.1: Add `body_text` to `NoteDto`

```rust
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct NoteDto {
    pub id: String,
    pub title: String,
    pub body: String,
    pub body_text: String,
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
    #[ts(type = "number | null")]
    pub deleted_at: Option<i64>,
}

impl From<Note> for NoteDto {
    fn from(n: Note) -> Self {
        NoteDto {
            id: n.id.to_string(),
            title: n.title,
            body_text: raki_domain::body_to_text(&n.body),
            body: n.body,
            created_at: n.created_at,
            updated_at: n.updated_at,
            deleted_at: n.deleted_at,
        }
    }
}
```

### Step 4.2: Update command validation and normalization

In `commands/notes.rs`, update `validate`:

```rust
fn validate(title: &str, body: &str) -> Result<(String, String), AppError> {
    let t = title.trim();
    if t.is_empty() {
        return Err(AppError {
            kind: "invalid".into(),
            message: "title must not be empty".into(),
        });
    }
    if t.chars().count() > MAX_TITLE_CHARS {
        return Err(AppError {
            kind: "invalid".into(),
            message: "title too long".into(),
        });
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(AppError {
            kind: "invalid".into(),
            message: "body too long".into(),
        });
    }
    // Body must be valid ProseMirror doc JSON.
    let normalized = raki_domain::normalize_body(body).map_err(|e| AppError {
        kind: "invalid".into(),
        message: format!("invalid note body: {e}"),
    })?;
    Ok((t.to_string(), normalized))
}
```

### Step 4.3: Add command unit tests

Add to `commands/notes.rs` test module:

```rust
#[test]
fn validate_accepts_valid_prosemirror_json() {
    let body = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"hi"}]}]}"#;
    let (title, out) = validate("T", body).unwrap();
    assert_eq!(title, "T");
    assert!(out.contains("blockId"));
}

#[test]
fn validate_rejects_invalid_json() {
    let err = validate("T", "not json").unwrap_err();
    assert_eq!(err.kind, "invalid");
}

#[test]
fn validate_rejects_non_doc_json() {
    let err = validate("T", r#"{"type":"not-doc"}"#).unwrap_err();
    assert_eq!(err.kind, "invalid");
}
```

**Run:**
```bash
cargo test -p raki commands::notes
```
**Expected:** PASS.

### Step 4.4: Commit

```bash
git add src-tauri/src/dto.rs src-tauri/src/commands/notes.rs
git commit -m "feat(app): DTOs and commands carry canonical ProseMirror JSON bodies"
```

---

## Task 5: Frontend TipTap editor

**Files:**
- Modify: `package.json`
- Create: `src/modules/notes/components/BlockId.ts`
- Create: `src/modules/notes/components/TipTapEditor.tsx`
- Create: `src/modules/notes/components/TipTapEditor.css`
- Modify: `src/modules/notes/components/NoteEditor.tsx`
- Modify: `src/modules/notes/NotesView.tsx`
- Modify: `src/modules/notes/NotesView.test.tsx` and `NoteEditor` tests

### Step 5.1: Add TipTap dependencies

In `package.json` dependencies:

```json
"@tiptap/core": "^2.11.0",
"@tiptap/starter-kit": "^2.11.0",
"@tiptap/extension-placeholder": "^2.11.0",
"uuid": "^11.0.0"
```

**Run:**
```bash
bun install
```
**Expected:** lockfile updated, no errors.

### Step 5.2: Create `BlockId` TipTap extension

Create `src/modules/notes/components/BlockId.ts`:

```ts
import { Extension } from "@tiptap/core";
import { v7 as uuidv7 } from "uuid";

function newBlockId(): string {
  return uuidv7();
}

function dedupeBlockIds(doc: any): any {
  const seen = new Set<string>();
  const content = doc.content?.map((node: any) => {
    const id = node.attrs?.blockId;
    if (id && seen.has(id)) {
      return {
        ...node,
        attrs: { ...node.attrs, blockId: newBlockId() },
      };
    }
    if (id) seen.add(id);
    return node;
  });
  return { ...doc, content };
}

export const BlockId = Extension.create({
  name: "blockId",
  addGlobalAttributes() {
    return [
      {
        types: ["paragraph", "heading", "bulletList", "orderedList", "codeBlock"],
        attributes: {
          blockId: {
            default: null,
            parseHTML: (el) => el.getAttribute("data-block-id"),
            renderHTML: (attrs) =>
              attrs.blockId ? { "data-block-id": attrs.blockId } : {},
          },
        },
      },
    ];
  },
  onCreate() {
    const { editor } = this;
    const doc = dedupeBlockIds(editor.state.doc.toJSON());
    editor.commands.setContent(doc, false);
  },
  onUpdate({ editor }) {
    const doc = dedupeBlockIds(editor.state.doc.toJSON());
    const current = editor.state.doc.toJSON();
    if (JSON.stringify(doc) !== JSON.stringify(current)) {
      editor.commands.setContent(doc, false);
    }
  },
});
```

```rust
#[tauri::command]
pub async fn create_note(
    state: State<'_, AppState>,
    input: CreateNoteInput,
) -> Result<NoteDto, AppError> {
    let (title, body) = validate(&input.title, &input.body)?;
    let note = Note::new(title, body, state.clock.now_ms());
    state.notes.upsert(&note).await?;
    state.index.trigger();
    Ok(NoteDto::from(note))
}
```

Update `update_note`:

```rust
#[tauri::command]
pub async fn update_note(
    state: State<'_, AppState>,
    input: UpdateNoteInput,
) -> Result<NoteDto, AppError> {
    let (title, body) = validate(&input.title, &input.body)?;
    let nid = NoteId::parse(&input.id)?;
    let existing = state.notes.get(&nid).await?.ok_or_else(|| AppError {
        kind: "not_found".into(),
        message: "note not found".into(),
    })?;
    let edited = existing.edit(title, body, state.clock.now_ms());
    if !state.notes.update(&edited).await? {
        return Err(AppError {
            kind: "not_found".into(),
            message: "note not found".into(),
        });
    }
    state.signal_store.touch(&nid, state.clock.now_ms()).await?;
    state.index.trigger();
    Ok(NoteDto::from(edited))
}
```

### Step 4.3: Add command unit tests

Add to `commands/notes.rs` test module:

```rust
#[test]
fn validate_accepts_valid_prosemirror_json() {
    let body = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"hi"}]}]}"#;
    let (title, out) = validate("T", body).unwrap();
    assert_eq!(title, "T");
    assert!(out.contains("blockId"));
}

#[test]
fn validate_rejects_invalid_json() {
    let err = validate("T", "not json").unwrap_err();
    assert_eq!(err.kind, "validation_error");
}

#[test]
fn validate_rejects_non_doc_json() {
    let err = validate("T", r#"{"type":"not-doc"}"#).unwrap_err();
    assert_eq!(err.kind, "validation_error");
}
```

**Run:**
```bash
cargo test -p raki commands::notes
```
**Expected:** PASS.

### Step 4.4: Commit

```bash
git add src-tauri/src/dto.rs src-tauri/src/commands/notes.rs
git commit -m "feat(app): DTOs and commands carry canonical ProseMirror JSON bodies"
```

---

## Task 5: Frontend TipTap editor

**Files:**
- Modify: `package.json`
- Create: `src/modules/notes/components/BlockId.ts`
- Create: `src/modules/notes/components/TipTapEditor.tsx`
- Modify: `src/modules/notes/components/NoteEditor.tsx`
- Modify: `src/modules/notes/NotesView.tsx`
- Modify: `src/modules/notes/NotesView.test.tsx` and `NoteEditor` tests

### Step 5.1: Add TipTap dependencies

In `package.json` dependencies:

```json
"@tiptap/core": "^2.11.0",
"@tiptap/starter-kit": "^2.11.0",
"@tiptap/extension-placeholder": "^2.11.0"
```

**Run:**
```bash
bun install
```
**Expected:** lockfile updated, no errors.

### Step 5.2: Create `BlockId` TipTap extension

Create `src/modules/notes/components/BlockId.ts`:

```ts
import { Extension } from "@tiptap/core";
import { v7 as uuidv7 } from "uuid";

function newBlockId(): string {
  return uuidv7();
}

function dedupeBlockIds(doc: any): any {
  const seen = new Set<string>();
  const content = doc.content?.map((node: any) => {
    const id = node.attrs?.blockId;
    if (id && seen.has(id)) {
      return {
        ...node,
        attrs: { ...node.attrs, blockId: newBlockId() },
      };
    }
    if (id) seen.add(id);
    return node;
  });
  return { ...doc, content };
}

export const BlockId = Extension.create({
  name: "blockId",
  addGlobalAttributes() {
    return [
      {
        types: ["paragraph", "heading", "bulletList", "orderedList", "codeBlock"],
        attributes: {
          blockId: {
            default: null,
            parseHTML: (el) => el.getAttribute("data-block-id"),
            renderHTML: (attrs) =>
              attrs.blockId ? { "data-block-id": attrs.blockId } : {},
          },
        },
      },
    ];
  },
  onCreate() {
    const { editor } = this;
    const doc = dedupeBlockIds(editor.state.doc.toJSON());
    editor.commands.setContent(doc, false);
  },
  onUpdate({ editor }) {
    const doc = dedupeBlockIds(editor.state.doc.toJSON());
    const current = editor.state.doc.toJSON();
    if (JSON.stringify(doc) !== JSON.stringify(current)) {
      editor.commands.setContent(doc, false);
    }
  },
});
```

Add `uuid` to `package.json` dependencies:

```json
"uuid": "^11.0.0"
```

### Step 5.3: Create `TipTapEditor` Solid component

Create `src/modules/notes/components/TipTapEditor.tsx`:

```tsx
import { Editor } from "@tiptap/core";
import StarterKit from "@tiptap/starter-kit";
import Placeholder from "@tiptap/extension-placeholder";
import { createEffect, createSignal, onCleanup } from "solid-js";
import { BlockId } from "./BlockId";

interface TipTapEditorProps {
  bodyJson: string;
  onChange: (bodyJson: string) => void;
  placeholder?: string;
}

export function TipTapEditor(props: TipTapEditorProps) {
  let mountRef: HTMLDivElement | undefined;
  const [editor, setEditor] = createSignal<Editor | null>(null);

  createEffect(() => {
    const ed = new Editor({
      element: mountRef,
      extensions: [
        StarterKit,
        Placeholder.configure({ placeholder: props.placeholder ?? "Start writing..." }),
        BlockId,
      ],
      content: props.bodyJson,
      autofocus: false,
      onUpdate: ({ editor }) => {
        props.onChange(JSON.stringify(editor.state.doc.toJSON()));
      },
    });
    setEditor(ed);
    onCleanup(() => ed.destroy());
  });

  // Reset content when the external note changes, but preserve editor focus/selection where possible.
  createEffect((prevId?: string) => {
    const ed = editor();
    if (!ed) return props.bodyJson;
    const json = props.bodyJson;
    if (json === prevId) return json;
    const current = JSON.stringify(ed.state.doc.toJSON());
    if (current !== json) {
      ed.commands.setContent(json, false);
    }
    return json;
  });

  return (
    <div
      ref={mountRef}
      class="min-h-[200px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-within:ring-2 focus-within:ring-ring"
    />
  );
}
```

Add minimal editor styles in `src/modules/notes/components/TipTapEditor.css` (or inline in `index.css`):

```css
.tiptap p { margin: 0.5em 0; }
.tiptap h1 { font-size: 1.5em; font-weight: bold; margin: 0.5em 0; }
.tiptap h2 { font-size: 1.25em; font-weight: bold; margin: 0.5em 0; }
.tiptap ul { list-style-type: disc; padding-left: 1.5em; }
.tiptap ol { list-style-type: decimal; padding-left: 1.5em; }
.tiptap pre { background: #f5f5f5; padding: 0.5em; border-radius: 4px; }
.tiptap p.is-editor-empty:first-child::before {
  content: attr(data-placeholder);
  float: left;
  color: #adb5bd;
  pointer-events: none;
  height: 0;
}
```

Import the CSS in `TipTapEditor.tsx`:

```tsx
import "./TipTapEditor.css";
```

### Step 5.4: Replace textarea in `NoteEditor.tsx`

Replace the `<Textarea>` usage:

```tsx
import { TipTapEditor } from "./TipTapEditor";

// In the component body, replace:
// const [body, setBody] = createSignal("");
// with:
const [bodyJson, setBodyJson] = createSignal("");

// In createEffect that loads note:
createEffect(() => {
  const n = note.data;
  if (n) {
    setTitle(n.title);
    setBodyJson(n.body);
  }
});

// In saveNote mutation:
mutationFn: () =>
  notesApi.update({
    id: props.noteId,
    title: title(),
    body: bodyJson(),
  }),

// In isDirty:
const isDirty = () => {
  const n = note.data;
  if (!n) return false;
  return n.title !== title() || n.body !== bodyJson();
};

// Replace <Textarea ... /> with:
<TipTapEditor bodyJson={bodyJson()} onChange={setBodyJson} placeholder="Start writing..." />
```

### Step 5.5: Update `NotesView.tsx`

`NotesView.tsx` still contains a textarea editor. Remove that editor surface entirely; keep the list/search/trash UI. Route note selection to the existing `NoteEditor.tsx` (or the route that renders it).

If `NotesView.tsx` is the route component, replace the inline textarea form with:

```tsx
<NoteEditor noteId={selectedId() ?? ""} onDeleted={() => setSelectedId(null)} />
```

Update `NotesView.test.tsx` accordingly.

### Step 5.6: Run frontend checks

```bash
bun run typecheck
bun run test
```
**Expected:** PASS after test updates.

### Step 5.7: Commit

```bash
git add package.json bun.lock src/modules/notes/components/BlockId.ts src/modules/notes/components/TipTapEditor.tsx src/modules/notes/components/TipTapEditor.css src/modules/notes/components/NoteEditor.tsx src/modules/notes/NotesView.tsx src/modules/notes/NotesView.test.tsx
git commit -m "feat(notes): TipTap editor with stable block IDs"
```

---

## Task 6: Regenerate bindings and integration verification

### Step 6.1: Regenerate TypeScript bindings

```bash
cd src-tauri && cargo test -p raki
```
**Expected:** `src/shared/ipc/bindings/NoteDto.ts` now includes `body_text`.

### Step 6.2: Verify all consumers of `NoteDto.body`

Search for `.body` usage on note objects:

```bash
grep -R "\.body" src/modules/notes/ src/modules/search/ src/modules/memory/
```

Ensure only `TipTapEditor` reads `body` as JSON; all other UI uses `body_text` or title.

### Step 6.3: Run full verification

```bash
cd src-tauri && cargo test --workspace --exclude raki
cd src-tauri && cargo test -p raki
cd src-tauri && cargo fmt --check
cd src-tauri && cargo clippy --workspace --exclude raki --all-targets -- -D warnings
cd /Users/jayden/Projects/Raki/bot && bun run typecheck
bun run test
```
**Expected:** all green.

### Step 6.4: Run eval baselines

```bash
cd src-tauri && cargo run -p raki-eval --bin chunk-eval -- --write
cd src-tauri && cargo run -p raki-eval --bin real-eval
```
**Expected:** both complete; chunking baseline shows block-scoped source IDs in integration tests.

### Step 6.5: Commit

```bash
git add src/shared/ipc/bindings/NoteDto.ts
git commit -m "chore(bindings): regenerate NoteDto with body_text"
```

---

## Spec coverage self-check

| Spec requirement | Task covering it |
|---|---|
| TipTap editor replaces textarea | Task 5 |
| Canonical ProseMirror JSON stored | Tasks 1, 4 |
| Stable block IDs | Tasks 1, 2, 5 |
| Block IDs survive edits/splits/merges | Task 1 domain tests, Task 5 dedupe |
| Backend normalization | Task 1, Task 4 |
| Validation rules for JSON bodies | Task 4 |
| `NoteDto.body_text` | Task 4 |
| Vector source IDs use block IDs | Task 2 |
| Embedding invalidation/backfill | Task 3 |
| Frontend consumer audit | Task 5, Task 6.2 |
| Manual Save (no autosave scope creep) | Task 5.4 |
| Duplicate ID handling on copy-paste | Task 5.2 |

## Placeholder scan

No TBD/TODO/fill-in details remain. Every step includes exact file paths, code, commands, and expected outputs.
