# R2 — Production Chunk-Level Embedding Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Migrate from whole-note embedding to chunk-level embedding with contextual prefixing (feature-flagged), compound chunk IDs, min-rank rollup, and parent-document retrieval.

**Architecture:** Structural blocks extracted from ProseMirror JSON are embedded as chunks (`note_id#index`). Vector search returns chunk IDs, which are rolled up to note IDs via min-rank. The old `note_vectors` table is preserved as stale backup. Contextual prefixing is behind a feature flag (default OFF).

**Tech Stack:** Rust, Tauri, SQLite + sqlite-vec, rusqlite, tokio, async-trait, serde_json

---

## File Structure

```
src-tauri/crates/raki-domain/src/ports.rs          MODIFY  PendingNote + VectorIndex trait
src-tauri/crates/raki-domain/src/body.rs           MODIFY  Add body_to_blocks, Block struct
src-tauri/crates/raki-memory/Cargo.toml            MODIFY  Add tokio, async-trait, tracing deps
src-tauri/crates/raki-memory/src/lib.rs            MODIFY  Export chunk + indexing modules
src-tauri/crates/raki-memory/src/chunk.rs          CREATE  chunk_note, body_to_blocks, cap_split
src-tauri/crates/raki-memory/src/indexing.rs       CREATE  embed_one, embed_pending, EmbedStats (moved from raki-app)
src-tauri/crates/raki-retrieval/src/search.rs      MODIFY  vector_search chunk IDs; hybrid_candidates min-rank + NoteId
src-tauri/crates/raki-storage/src/migrations.rs    MODIFY  V7 migration + integration test
src-tauri/crates/raki-storage/src/vectors.rs       MODIFY  chunk_vectors table; delete_by_prefix; upsert_batch
src-tauri/crates/raki-storage/src/indexing.rs      MODIFY  list_pending returns title+body; ORDER BY updated_at DESC
src-tauri/crates/raki-storage/src/notes.rs         MODIFY  soft_delete cleans chunk_vectors
src-tauri/src/indexing.rs                          MODIFY  Import from raki-memory; keep IndexingService
src-tauri/src/commands/notes.rs                    MODIFY  search_reranked uses NoteId from hybrid_candidates
```

---

### Task 1: Domain — Update `PendingNote` and `VectorIndex` trait

**Files:**
- Modify: `src-tauri/crates/raki-domain/src/ports.rs`
- Test: inline in same file

- [ ] **Step 1: Write the failing test**

Add to the bottom of `ports.rs` inside `#[cfg(test)]` (create the module if it doesn't exist):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_note_has_title_and_body() {
        let note = PendingNote {
            id: NoteId::new(),
            title: "Title".into(),
            body: "{}".into(),
            content_hash: "hash".into(),
        };
        assert_eq!(note.title, "Title");
        assert_eq!(note.body, "{}");
    }

    #[test]
    fn vector_index_trait_has_batch_methods() {
        // Compile-time check: ensure the trait requires the new methods.
        fn _assert<T: VectorIndex>() {}
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p raki-domain --lib -- ports::tests::pending_note_has_title_and_body -v`

Expected: FAIL with "no variant or associated item named `new`" or similar compilation error because `PendingNote` still has `text` field.

- [ ] **Step 3: Update `PendingNote` struct**

Replace the `PendingNote` struct in `ports.rs` (around line 126):

```rust
/// A note awaiting (re)embedding: its id, the raw title and body (for chunking),
/// and the content hash (used for the compare-and-stamp guard).
pub struct PendingNote {
    pub id: NoteId,
    pub title: String,
    pub body: String,
    pub content_hash: String,
}
```

- [ ] **Step 4: Add `delete_by_prefix` and `upsert_batch` to `VectorIndex`**

Replace the `VectorIndex` trait (around line 106):

```rust
#[async_trait]
pub trait VectorIndex: Send + Sync {
    async fn upsert(&self, source_id: &str, embedding: &Embedding) -> Result<(), DomainError>;
    async fn query(&self, embedding: &Embedding, k: usize) -> Result<Vec<VectorHit>, DomainError>;
    /// Delete all vectors whose source_id starts with `prefix`.
    async fn delete_by_prefix(&self, prefix: &str) -> Result<(), DomainError>;
    /// Upsert multiple vectors in one operation. Each item is (source_id, embedding).
    async fn upsert_batch(&self, items: &[(String, Embedding)]) -> Result<(), DomainError>;
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p raki-domain --lib -- ports::tests -v`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-domain/src/ports.rs
git commit -m "feat(domain): PendingNote carries title+body; VectorIndex gains delete_by_prefix + upsert_batch"
```

---

### Task 2: Domain — Add `body_to_blocks` ProseMirror parser

**Files:**
- Modify: `src-tauri/crates/raki-domain/src/body.rs`
- Test: inline in same file

- [ ] **Step 1: Write the failing test**

Add inside `body.rs`'s existing `#[cfg(test)] mod tests`:

```rust
#[test]
fn body_to_blocks_extracts_paragraphs() {
    let body = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"First"}]},{"type":"paragraph","content":[{"type":"text","text":"Second"}]}]}"#;
    let blocks = body_to_blocks(body);
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].text, "First");
    assert_eq!(blocks[1].text, "Second");
}

#[test]
fn body_to_blocks_joins_list_items() {
    let body = r#"{"type":"doc","content":[{"type":"bulletList","content":[{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"milk"}]}]},{"type":"listItem","content":[{"type":"paragraph","content":[{"type":"text","text":"eggs"}]}]}]}]}"#;
    let blocks = body_to_blocks(body);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "milk\neggs");
}

#[test]
fn body_to_blocks_tracks_headings() {
    let body = r#"{"type":"doc","content":[{"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Section"}]},{"type":"paragraph","content":[{"type":"text","text":"Under section"}]}]}"#;
    let blocks = body_to_blocks(body);
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].heading, Some("Section".into()));
    assert_eq!(blocks[0].text, "Under section");
}

#[test]
fn body_to_blocks_returns_empty_for_invalid_json() {
    let blocks = body_to_blocks("not json");
    assert!(blocks.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p raki-domain --lib -- body::tests::body_to_blocks_extracts_paragraphs -v`

Expected: FAIL — `body_to_blocks` not found.

- [ ] **Step 3: Add `Block` struct and `body_to_blocks` function**

Add to `body.rs` after the existing `extract_text` function (before `text_to_body`):

```rust
/// A structural block extracted from a ProseMirror document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// The most recent heading text before this block, if any.
    pub heading: Option<String>,
    /// The textual content of the block.
    pub text: String,
}

/// Extract structural blocks from a ProseMirror JSON body.
/// Paragraphs become individual blocks; lists become one block (items joined).
/// Headings provide context for subsequent blocks but are not blocks themselves.
pub fn body_to_blocks(body: &str) -> Vec<Block> {
    let doc: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let mut blocks = Vec::new();
    let mut current_heading: Option<String> = None;

    if let Some(content) = doc.get("content").and_then(|v| v.as_array()) {
        for node in content {
            match node.get("type").and_then(|v| v.as_str()) {
                Some("heading") => {
                    current_heading = Some(extract_text(node));
                }
                Some("paragraph") => {
                    let text = extract_text(node);
                    if !text.is_empty() {
                        blocks.push(Block {
                            heading: current_heading.clone(),
                            text,
                        });
                    }
                }
                Some("bulletList") | Some("orderedList") => {
                    let mut items = Vec::new();
                    if let Some(list_content) = node.get("content").and_then(|v| v.as_array()) {
                        for item in list_content {
                            let item_text = extract_text(item);
                            if !item_text.is_empty() {
                                items.push(item_text);
                            }
                        }
                    }
                    if !items.is_empty() {
                        blocks.push(Block {
                            heading: current_heading.clone(),
                            text: items.join("\n"),
                        });
                    }
                }
                Some("codeBlock") => {
                    let text = extract_text(node);
                    if !text.is_empty() {
                        blocks.push(Block {
                            heading: current_heading.clone(),
                            text,
                        });
                    }
                }
                _ => {} // skip unknown nodes
            }
        }
    }
    blocks
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p raki-domain --lib -- body::tests::body_to_blocks -v`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-domain/src/body.rs
git commit -m "feat(domain): add body_to_blocks ProseMirror parser with heading context"
```

---

### Task 3: raki-memory — Create `chunk.rs` with chunking logic

**Files:**
- Create: `src-tauri/crates/raki-memory/src/chunk.rs`
- Modify: `src-tauri/crates/raki-memory/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/crates/raki-memory/src/chunk.rs` with tests first:

```rust
//! Chunking: structural blocks from ProseMirror → capped chunks → prefixed strings.

use raki_domain::{body_to_blocks, Block};

const CHUNK_CHAR_CAP: usize = 1600;
const MAX_CHUNKS_PER_NOTE: usize = 32;

/// Split `text` into chunks no longer than `CHUNK_CHAR_CAP` chars.
/// Prefers splitting at whitespace; if a single word exceeds the cap, it is kept intact.
fn cap_split(text: &str) -> Vec<String> {
    if text.len() <= CHUNK_CHAR_CAP {
        return vec![text.to_string()];
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let end = (start + CHUNK_CHAR_CAP).min(text.len());
        let split_at = if end == text.len() {
            end
        } else {
            text[start..end].rfind(' ').map(|i| start + i).unwrap_or(end)
        };
        chunks.push(text[start..split_at].to_string());
        start = split_at;
        if start < text.len() && text[start..].starts_with(' ') {
            start += 1;
        }
    }
    chunks
}

/// Chunk a note's title and body into strings ready for embedding.
/// If `use_prefix` is true, each chunk is prefixed with title and heading context.
/// Returns at most `MAX_CHUNKS_PER_NOTE` chunks.
pub fn chunk_note(title: &str, body: &str, use_prefix: bool) -> Vec<String> {
    let blocks = body_to_blocks(body);
    let mut chunks: Vec<String> = blocks
        .into_iter()
        .flat_map(|block| {
            let base = if use_prefix {
                match block.heading {
                    Some(h) => format!("{} > {}: {}", title, h, block.text),
                    None => format!("{}: {}", title, block.text),
                }
            } else {
                block.text
            };
            cap_split(&base)
        })
        .collect();

    if chunks.is_empty() {
        chunks.push(title.to_string());
    }

    if chunks.len() > MAX_CHUNKS_PER_NOTE {
        tracing::warn!(
            "Note '{}' produced {} chunks; truncating to {}",
            title,
            chunks.len(),
            MAX_CHUNKS_PER_NOTE
        );
        chunks.truncate(MAX_CHUNKS_PER_NOTE);
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_note_bare_blocks() {
        let body = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello world"}]}]}"#;
        let chunks = chunk_note("Note", body, false);
        assert_eq!(chunks, vec!["Hello world"]);
    }

    #[test]
    fn chunk_note_with_prefix() {
        let body = r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello"}]}]}"#;
        let chunks = chunk_note("Note", body, true);
        assert_eq!(chunks, vec!["Note: Hello"]);
    }

    #[test]
    fn chunk_note_with_heading_prefix() {
        let body = r#"{"type":"doc","content":[{"type":"heading","attrs":{"level":2},"content":[{"type":"text","text":"Section"}]},{"type":"paragraph","content":[{"type":"text","text":"Text"}]}]}"#;
        let chunks = chunk_note("Note", body, true);
        assert_eq!(chunks, vec!["Note > Section: Text"]);
    }

    #[test]
    fn chunk_note_empty_body_returns_title() {
        let chunks = chunk_note("Title", "{}", false);
        assert_eq!(chunks, vec!["Title"]);
    }

    #[test]
    fn chunk_note_zero_block_body_returns_title() {
        let body = r#"{"type":"doc","content":[{"type":"horizontalRule"}]}"#;
        let chunks = chunk_note("Title", body, false);
        assert_eq!(chunks, vec!["Title"]);
    }

    #[test]
    fn cap_split_does_not_silently_truncate() {
        let long = "a ".repeat(1000); // ~2000 chars
        let chunks = cap_split(&long);
        let recovered: String = chunks.join(" ");
        assert_eq!(recovered.trim(), long.trim());
    }

    #[test]
    fn chunk_note_respects_max_chunks() {
        let body = r#"{"type":"doc","content":["#;
        // Create 40 tiny paragraphs to exceed MAX_CHUNKS_PER_NOTE
        let paragraphs: Vec<String> = (0..40)
            .map(|i| format!(r#"{{"type":"paragraph","content":[{{"type":"text","text":"{}"}}]}}"#, i))
            .collect();
        let body = format!("{}}}", paragraphs.join(","));
        let chunks = chunk_note("Note", &body, false);
        assert_eq!(chunks.len(), MAX_CHUNKS_PER_NOTE);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p raki-memory --lib -- chunk::tests::chunk_note_bare_blocks -v`

Expected: FAIL — `chunk` module not found because `lib.rs` hasn't exported it yet.

- [ ] **Step 3: Export chunk module from `lib.rs`**

Modify `src-tauri/crates/raki-memory/src/lib.rs`:

```rust
//! The memory layer: embedding pipeline, memory lifecycle, and context assembly.

mod chunk;
mod context;
pub mod indexing;

pub use chunk::chunk_note;
pub use context::{assemble_context, AssembledContext, Candidate, ContextItem};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p raki-memory --lib -- chunk::tests -v`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-memory/src/chunk.rs src-tauri/crates/raki-memory/src/lib.rs
git commit -m "feat(memory): add chunk_note with body_to_blocks, cap_split, and prefix support"
```

---

### Task 4: raki-memory — Create `indexing.rs` (move embed pipeline from raki-app)

**Files:**
- Create: `src-tauri/crates/raki-memory/src/indexing.rs`
- Modify: `src-tauri/crates/raki-memory/Cargo.toml`

- [ ] **Step 1: Add dependencies to `raki-memory/Cargo.toml`**

```toml
[package]
name = "raki-memory"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
raki-domain = { workspace = true }
async-trait = { workspace = true }
tokio = { workspace = true }
tracing = { workspace = true }
```

- [ ] **Step 2: Create `indexing.rs`**

Create `src-tauri/crates/raki-memory/src/indexing.rs`:

```rust
//! The embedding pipeline orchestration: drain stale notes through embed → upsert
//! vector → compare-and-stamp, with per-note failure isolation.

use std::collections::HashSet;

use raki_domain::{
    DomainError, EmbeddingProvider, IndexingStore, NoteId, PendingNote, VectorIndex,
};

use crate::chunk::chunk_note;

/// Outcome of one drain.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EmbedStats {
    pub embedded: usize,
    pub failed: usize,
}

/// Configuration for the embedding pipeline.
#[derive(Clone, Copy, Debug)]
pub struct EmbedConfig {
    pub use_contextual_prefix: bool,
}

impl Default for EmbedConfig {
    fn default() -> Self {
        Self {
            use_contextual_prefix: false,
        }
    }
}

/// Embed every stale live note for the embedder's model, idempotently.
pub async fn embed_pending(
    store: &dyn IndexingStore,
    embedder: &dyn EmbeddingProvider,
    vectors: &dyn VectorIndex,
    config: &EmbedConfig,
    batch: usize,
) -> Result<EmbedStats, DomainError> {
    let model_id = embedder.model_id();
    let mut embedded = 0usize;
    let mut failed: HashSet<NoteId> = HashSet::new();

    loop {
        let pending = store.list_pending(&model_id, batch).await?;
        let todo: Vec<PendingNote> = pending
            .into_iter()
            .filter(|p| !failed.contains(&p.id))
            .collect();
        if todo.is_empty() {
            break;
        }
        for note in todo {
            match embed_one(store, embedder, vectors, config, &note).await {
                Ok(true) => embedded += 1,
                Ok(false) => { /* superseded mid-flight */ }
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
    config: &EmbedConfig,
    note: &PendingNote,
) -> Result<bool, DomainError> {
    let chunks = chunk_note(&note.title, &note.body, config.use_contextual_prefix);
    if chunks.is_empty() {
        return Ok(true);
    }
    let embeddings = embedder.embed(&chunks).await?;

    let prefix = format!("{}#", note.id);
    vectors.delete_by_prefix(&prefix).await?;

    let items: Vec<(String, raki_domain::Embedding)> = embeddings
        .into_iter()
        .enumerate()
        .map(|(i, emb)| (format!("{}#{}", note.id, i), emb))
        .collect();
    vectors.upsert_batch(&items).await?;

    let stamped = store
        .mark_embedded(&note.id, &note.content_hash, &embedder.model_id())
        .await?;

    // Compensating delete: if the note was soft-deleted mid-flight, clean up orphaned chunks.
    if !stamped {
        vectors.delete_by_prefix(&prefix).await?;
    }

    Ok(stamped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{Embedding, EmbeddingProvider, Locality};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    struct FakeEmbedder {
        dim: usize,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl EmbeddingProvider for FakeEmbedder {
        fn dimension(&self) -> usize { self.dim }
        fn locality(&self) -> Locality { Locality::Local }
        fn model_id(&self) -> String { "fake".into() }
        async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, DomainError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(inputs.iter().map(|_| Embedding(vec![1.0; self.dim])).collect())
        }
    }

    struct FakeVectors {
        upserted: Arc<AtomicUsize>,
        deleted: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl VectorIndex for FakeVectors {
        async fn upsert(&self, _id: &str, _e: &Embedding) -> Result<(), DomainError> {
            self.upserted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn query(&self, _e: &Embedding, _k: usize) -> Result<Vec<raki_domain::VectorHit>, DomainError> {
            Ok(vec![])
        }
        async fn delete_by_prefix(&self, _prefix: &str) -> Result<(), DomainError> {
            self.deleted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn upsert_batch(&self, items: &[(String, Embedding)]) -> Result<(), DomainError> {
            self.upserted.fetch_add(items.len(), Ordering::SeqCst);
            Ok(())
        }
    }

    struct FakeStore;

    #[async_trait]
    impl IndexingStore for FakeStore {
        async fn backfill_content_hashes(&self) -> Result<(), DomainError> { Ok(()) }
        async fn list_pending(&self, _model: &str, _limit: usize) -> Result<Vec<PendingNote>, DomainError> {
            Ok(vec![])
        }
        async fn mark_embedded(&self, _id: &NoteId, _hash: &str, _model: &str) -> Result<bool, DomainError> {
            Ok(true)
        }
    }

    #[tokio::test]
    async fn embed_one_chunks_and_upserts_batch() {
        let embedder = FakeEmbedder { dim: 2, calls: Arc::new(AtomicUsize::new(0)) };
        let vectors = FakeVectors { upserted: Arc::new(AtomicUsize::new(0)), deleted: Arc::new(AtomicUsize::new(0)) };
        let store = FakeStore;
        let note = PendingNote {
            id: NoteId::new(),
            title: "Title".into(),
            body: r#"{"type":"doc","content":[{"type":"paragraph","content":[{"type":"text","text":"Hello"}]},{"type":"paragraph","content":[{"type":"text","text":"World"}]}]}"#.into(),
            content_hash: "hash".into(),
        };
        let config = EmbedConfig::default();
        let result = embed_one(&store, &embedder, &vectors, &config, &note).await.unwrap();
        assert!(result);
        assert_eq!(embedder.calls.load(Ordering::SeqCst), 1); // one batch call
        assert_eq!(vectors.deleted.load(Ordering::SeqCst), 1); // delete old chunks
        assert_eq!(vectors.upserted.load(Ordering::SeqCst), 2); // two chunks
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p raki-memory --lib -- indexing::tests::embed_one_chunks_and_upserts_batch -v`

Expected: FAIL — compilation errors because `lib.rs` doesn't export `indexing` yet and `EmbedConfig` might conflict.

- [ ] **Step 4: Export indexing module from `lib.rs`**

`lib.rs` was already updated in Task 3 to include `pub mod indexing;`. Verify it exists:

```rust
pub mod indexing;
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p raki-memory --lib -- indexing::tests -v`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-memory/src/indexing.rs src-tauri/crates/raki-memory/Cargo.toml src-tauri/crates/raki-memory/src/lib.rs
git commit -m "feat(memory): move embed pipeline to raki-memory with chunk support"
```

---

### Task 5: raki-app — Update `indexing.rs` to use `raki-memory`

**Files:**
- Modify: `src-tauri/src/indexing.rs`

- [ ] **Step 1: Update imports and remove duplicated logic**

Replace the contents of `src-tauri/src/indexing.rs` with:

```rust
//! DI + single-flight wrapper around `embed_pending`. `trigger` fires a background
//! pass and silently skips if one is already running.

use std::sync::Arc;

use tokio::sync::Mutex;

use raki_domain::{EmbeddingProvider, IndexingStore, VectorIndex};
use raki_memory::indexing::{embed_pending, EmbedConfig, EmbedStats};

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

    pub async fn run_once(&self) -> Result<EmbedStats, raki_domain::DomainError> {
        self.store.backfill_content_hashes().await?;
        let config = EmbedConfig::default();
        embed_pending(
            self.store.as_ref(),
            self.embedder.as_ref(),
            self.vectors.as_ref(),
            &config,
            self.batch,
        )
        .await
    }

    pub fn trigger(self: &Arc<Self>) {
        let this = self.clone();
        tauri::async_runtime::spawn(async move {
            let Ok(_guard) = this.running.try_lock() else {
                return;
            };
            if let Err(e) = this.run_once().await {
                eprintln!("indexing pass failed: {e}");
            }
        });
    }
}
```

- [ ] **Step 2: Verify compilation**

Run: `cd src-tauri && cargo check -p raki`

Expected: Should compile (may warn about unused imports; fix if needed).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/indexing.rs
git commit -m "refactor(app): delegate embed pipeline to raki-memory"
```

---

### Task 6: raki-retrieval — Update search for chunk IDs and min-rank rollup

**Files:**
- Modify: `src-tauri/crates/raki-retrieval/src/search.rs`

- [ ] **Step 1: Write the failing test**

Add to the existing `#[cfg(test)] mod tests` in `search.rs`:

```rust
#[tokio::test]
async fn hybrid_candidates_rolls_up_chunk_ids_with_min_rank() {
    let index = FakeVectors(vec!["note-a#0", "note-b#1", "note-a#2"]);
    let keyword = FakeKeyword(vec!["note-c"]);
    let embedder = FakeEmbed;
    let ids = hybrid_candidates(&keyword, &index, &embedder, "q", 10).await.unwrap();
    // note-a first (chunk #0), note-b second (chunk #1), note-c backfilled
    assert_eq!(ids.len(), 3);
    assert_eq!(ids[0], NoteId::parse("note-a").unwrap());
    assert_eq!(ids[1], NoteId::parse("note-b").unwrap());
    assert_eq!(ids[2], NoteId::parse("note-c").unwrap());
}

#[tokio::test]
async fn hybrid_candidates_never_emits_raw_chunk_ids() {
    let index = FakeVectors(vec!["note-a#0", "note-b#1"]);
    let keyword = FakeKeyword(vec![]);
    let embedder = FakeEmbed;
    let ids = hybrid_candidates(&keyword, &index, &embedder, "q", 10).await.unwrap();
    for id in &ids {
        // NoteId::parse would have panicked during the test if a chunk ID leaked through
        let _ = id.to_string(); // just verify it's a valid NoteId
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p raki-retrieval --lib -- search::tests::hybrid_candidates_rolls_up_chunk_ids_with_min_rank -v`

Expected: FAIL — compilation error because `hybrid_candidates` still returns `Vec<String>`.

- [ ] **Step 3: Implement chunk ID parsing and min-rank rollup**

Replace `search.rs` with:

```rust
//! The query-time ranking seams: `search` (keyword), `vector_search` (semantic), and
//! `hybrid_search`, a vector-primary recall union of the two.

use std::collections::HashSet;

use raki_domain::{DomainError, EmbeddingProvider, KeywordIndex, NoteId, VectorIndex};

const HYBRID_CANDIDATE_POOL: usize = 20;

fn note_id_from_chunk(chunk_id: &str) -> NoteId {
    let raw = chunk_id.split('#').next().unwrap_or(chunk_id);
    NoteId::parse(raw).expect("chunk ID must start with a valid note ID")
}

pub async fn hybrid_candidates(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    query: &str,
    pool: usize,
) -> Result<Vec<NoteId>, DomainError> {
    let depth = pool.max(HYBRID_CANDIDATE_POOL);
    let chunk_ids = vector_search(vectors, embedder, query, depth).await?;

    let mut seen = HashSet::new();
    let mut out: Vec<NoteId> = Vec::new();

    // Vector recall with min-rank rollup: first occurrence of each note wins.
    for chunk_id in chunk_ids {
        let nid = note_id_from_chunk(&chunk_id);
        if seen.insert(nid.clone()) {
            out.push(nid);
        }
    }

    // Keyword backfill: append note IDs vector did not already return.
    for keyword_id in search(keyword, query, depth).await? {
        let nid = NoteId::parse(&keyword_id)?;
        if seen.insert(nid.clone()) {
            out.push(nid);
        }
    }

    Ok(out)
}

pub async fn hybrid_search(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    query: &str,
    k: usize,
) -> Result<Vec<NoteId>, DomainError> {
    let mut out = hybrid_candidates(keyword, vectors, embedder, query, k).await?;
    out.truncate(k);
    Ok(out)
}

pub async fn search(
    keyword: &dyn KeywordIndex,
    query: &str,
    k: usize,
) -> Result<Vec<String>, DomainError> {
    let hits = keyword.query(query, k).await?;
    Ok(hits.into_iter().map(|h| h.source_id).collect())
}

pub async fn vector_search(
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    query: &str,
    k: usize,
) -> Result<Vec<String>, DomainError> {
    let mut embedded = embedder.embed_query(&[query.to_string()]).await?;
    let emb = embedded
        .pop()
        .ok_or_else(|| DomainError::Provider("empty query embedding".to_string()))?;
    let hits = vectors.query(&emb, k).await?;
    Ok(hits.into_iter().map(|h| h.source_id).collect())
}
```

- [ ] **Step 4: Update existing tests to use `NoteId`**

In the existing `search::tests` module, update the `FakeVectors` implementation and any tests that call `hybrid_candidates` directly:

The `FakeVectors` struct stays the same but `hybrid_candidates` now returns `Vec<NoteId>`. The existing test `search_returns_ids_in_index_order` only tests `search` (keyword), which still returns `Vec<String>`, so it's unaffected.

Add `use raki_domain::NoteId;` to the test module imports.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p raki-retrieval --lib -- search::tests -v`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-retrieval/src/search.rs
git commit -m "feat(retrieval): chunk ID min-rank rollup; hybrid_candidates returns NoteId"
```

---

### Task 7: raki-app — Update `search_reranked` for `NoteId`

**Files:**
- Modify: `src-tauri/src/commands/notes.rs`

- [ ] **Step 1: Update `search_reranked` to use `NoteId` directly**

Replace the `search_reranked` function (around line 78):

```rust
async fn search_reranked(
    notes: &dyn NoteRepository,
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    reranker: Option<&dyn Reranker>,
    query: &str,
) -> Result<Vec<Note>, DomainError> {
    let pool = raki_retrieval::hybrid_candidates(keyword, vectors, embedder, query, POOL).await?;

    let mut hydrated: Vec<Note> = Vec::with_capacity(pool.len());
    for nid in &pool {
        if let Some(note) = notes.get(nid).await? {
            hydrated.push(note);
        }
    }

    let candidates: Vec<(String, String)> = hydrated
        .iter()
        .map(|n| {
            let text = format!("{}\n{}", n.title, body_to_text(&n.body));
            (n.id.to_string(), cap_text(&text, MAX_RERANK_DOC_BYTES))
        })
        .collect();

    let hybrid_top_k = || -> Vec<String> {
        candidates
            .iter()
            .take(K)
            .map(|(id, _)| id.clone())
            .collect()
    };
    let ranked_ids: Vec<String> = match reranker {
        Some(r) => rerank_top_k(r, query, &candidates, K, RERANK_TIMEOUT)
            .await
            .unwrap_or_else(hybrid_top_k),
        None => hybrid_top_k(),
    };

    let mut by_id: HashMap<String, Note> = hydrated
        .into_iter()
        .map(|n| (n.id.to_string(), n))
        .collect();
    Ok(ranked_ids
        .iter()
        .filter_map(|id| by_id.remove(id))
        .collect())
}
```

- [ ] **Step 2: Verify compilation**

Run: `cd src-tauri && cargo check -p raki`

Expected: Clean compile (warnings OK, no errors).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/commands/notes.rs
git commit -m "refactor(app): search_reranked uses NoteId from hybrid_candidates"
```

---

### Task 8: raki-storage — V7 migration

**Files:**
- Modify: `src-tauri/crates/raki-storage/src/migrations.rs`

- [ ] **Step 1: Add V7 migration SQL**

Add to the `MIGRATIONS` array in `migrations.rs` (after V6):

```rust
    // V7: chunk-level vector index. Creates chunk_vectors alongside note_vectors (preserved
    // as stale backup). Clears embedded_hash so the background indexer re-chunks and
    // re-embeds the entire corpus on next start.
    "CREATE VIRTUAL TABLE chunk_vectors USING vec0(
        chunk_id TEXT PRIMARY KEY,
        embedding float[384]
    );
    UPDATE notes SET embedded_hash = NULL;",
```

- [ ] **Step 2: Write the failing test**

Add to `migrations.rs` tests:

```rust
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

    // Populate note_vectors BEFORE the migration.
    let id = "00000000-0000-7000-8000-000000000001";
    conn.execute(
        "INSERT INTO notes (id, title, body, created_at, updated_at, deleted_at, version, content_hash, embedded_hash, embedded_model)
         VALUES (?1, 'T', '{}', 1, 1, NULL, 1, 'h', 'h', 'm')",
        rusqlite::params![id],
    ).unwrap();
    let blob = vec![0u8; 384 * 4];
    conn.execute(
        "INSERT INTO note_vectors (note_id, embedding) VALUES (?1, ?2)",
        rusqlite::params![id, blob],
    ).unwrap();

    migrate(&conn).unwrap(); // applies V7

    // note_vectors still exists (preserved).
    let old_count: i64 = conn
        .query_row("SELECT count(*) FROM note_vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(old_count, 1, "note_vectors must be preserved");

    // chunk_vectors was created.
    let new_count: i64 = conn
        .query_row("SELECT count(*) FROM chunk_vectors", [], |r| r.get(0))
        .unwrap();
    assert_eq!(new_count, 0, "chunk_vectors starts empty");

    // embedded_hash cleared → note is pending.
    let embedded_hash: Option<String> = conn
        .query_row("SELECT embedded_hash FROM notes WHERE id = ?1", [id], |r| r.get(0))
        .unwrap();
    assert_eq!(embedded_hash, None, "V7 clears embedded_hash");
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cd src-tauri && cargo test -p raki-storage --lib -- migrations::tests::v7_creates_chunk_vectors_without_dropping_note_vectors -v`

Expected: FAIL — test not found because V7 SQL not added yet.

- [ ] **Step 4: Add V7 SQL (already done in Step 1)**

Verify the SQL is in the `MIGRATIONS` array.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p raki-storage --lib -- migrations::tests::v7 -v`

Expected: PASS

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-storage/src/migrations.rs
git commit -m "feat(storage): V7 migration creates chunk_vectors, preserves note_vectors, clears embedded_hash"
```

---

### Task 9: raki-storage — Update `SqliteVectorIndex` for chunks

**Files:**
- Modify: `src-tauri/crates/raki-storage/src/vectors.rs`

- [ ] **Step 1: Update table name and add new methods**

Replace `vectors.rs`:

```rust
//! The sqlite-vec-backed VectorIndex. Vectors are stored as compact little-endian
//! f32 blobs in the `chunk_vectors` vec0 table (declared `float[384]`).

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
                let tx = c.unchecked_transaction()?;
                tx.execute("DELETE FROM chunk_vectors WHERE chunk_id = ?1", params![id])?;
                tx.execute(
                    "INSERT INTO chunk_vectors (chunk_id, embedding) VALUES (?1, ?2)",
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
                    "SELECT chunk_id, distance
                     FROM chunk_vectors
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

    async fn delete_by_prefix(&self, prefix: &str) -> Result<(), DomainError> {
        let p = prefix.to_string();
        self.db
            .call(move |c| {
                c.execute(
                    "DELETE FROM chunk_vectors WHERE chunk_id LIKE ?1",
                    params![format!("{}%", p)],
                )?;
                Ok(())
            })
            .await
    }

    async fn upsert_batch(&self, items: &[(String, Embedding)]) -> Result<(), DomainError> {
        let batch: Vec<(String, Vec<u8>)> = items
            .iter()
            .map(|(id, emb)| (id.clone(), embedding_to_blob(emb)))
            .collect();
        self.db
            .call(move |c| {
                let tx = c.unchecked_transaction()?;
                for (id, blob) in batch {
                    tx.execute("DELETE FROM chunk_vectors WHERE chunk_id = ?1", params![id])?;
                    tx.execute(
                        "INSERT INTO chunk_vectors (chunk_id, embedding) VALUES (?1, ?2)",
                        params![id, blob],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{Embedding, VectorIndex};
    use crate::db::Database;

    fn basis(i: usize) -> Embedding {
        let mut v = vec![0.0_f32; 384];
        v[i] = 1.0;
        Embedding(v)
    }

    #[tokio::test]
    async fn upsert_then_query_returns_nearest_first() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db);
        index.upsert("a#0", &basis(0)).await.unwrap();
        index.upsert("b#0", &basis(1)).await.unwrap();
        index.upsert("c#0", &basis(2)).await.unwrap();

        let hits = index.query(&basis(1), 3).await.unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(hits[0].source_id, "b#0", "exact match ranks first");
    }

    #[tokio::test]
    async fn upsert_is_idempotent_overwrite() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db.clone());
        index.upsert("a#0", &basis(0)).await.unwrap();
        index.upsert("a#0", &basis(5)).await.unwrap();

        let n: i64 = db
            .call(|c| c.query_row("SELECT count(*) FROM chunk_vectors", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(n, 1, "re-upserting the same id overwrites");
    }

    #[tokio::test]
    async fn delete_by_prefix_removes_chunks() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db.clone());
        index.upsert("note-a#0", &basis(0)).await.unwrap();
        index.upsert("note-a#1", &basis(1)).await.unwrap();
        index.upsert("note-b#0", &basis(2)).await.unwrap();

        index.delete_by_prefix("note-a#").await.unwrap();

        let n: i64 = db
            .call(|c| c.query_row("SELECT count(*) FROM chunk_vectors", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(n, 1, "only note-b chunk remains");
    }

    #[tokio::test]
    async fn upsert_batch_inserts_multiple() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db.clone());
        let items = vec![
            ("a#0".to_string(), basis(0)),
            ("a#1".to_string(), basis(1)),
        ];
        index.upsert_batch(&items).await.unwrap();

        let n: i64 = db
            .call(|c| c.query_row("SELECT count(*) FROM chunk_vectors", [], |r| r.get(0)))
            .await
            .unwrap();
        assert_eq!(n, 2);
    }

    #[tokio::test]
    async fn query_limits_to_k() {
        let db = Database::open_in_memory().unwrap();
        let index = SqliteVectorIndex::new(db);
        for i in 0..5 {
            index.upsert(&format!("n{i}#0"), &basis(i)).await.unwrap();
        }
        let hits = index.query(&basis(0), 2).await.unwrap();
        assert_eq!(hits.len(), 2);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p raki-storage --lib -- vectors::tests -v`

Expected: PASS

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/raki-storage/src/vectors.rs
git commit -m "feat(storage): SqliteVectorIndex uses chunk_vectors with delete_by_prefix + upsert_batch"
```

---

### Task 10: raki-storage — Update `SqliteIndexingStore::list_pending`

**Files:**
- Modify: `src-tauri/crates/raki-storage/src/indexing.rs`

- [ ] **Step 1: Update `list_pending` to return title + body with ORDER BY**

Replace `list_pending` in `indexing.rs`:

```rust
async fn list_pending(
    &self,
    model_id: &str,
    limit: usize,
) -> Result<Vec<PendingNote>, DomainError> {
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
                 ORDER BY updated_at DESC
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
                    Ok(PendingNote {
                        id: note_id_from_row(&id)?,
                        title,
                        body,
                        content_hash,
                    })
                })
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .await
}
```

- [ ] **Step 2: Update tests**

Update the test `lists_pending_then_stops_after_stamp` to access `pending[0].title` instead of `pending[0].text` (add an assertion for title).

- [ ] **Step 3: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p raki-storage --lib -- indexing::tests -v`

Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-storage/src/indexing.rs
git commit -m "feat(storage): list_pending returns title+body, orders by updated_at DESC"
```

---

### Task 11: raki-storage — Update `SqliteNoteRepository::soft_delete` for chunks

**Files:**
- Modify: `src-tauri/crates/raki-storage/src/notes.rs`

- [ ] **Step 1: Update `soft_delete` to delete from `chunk_vectors`**

Replace the `soft_delete` method:

```rust
async fn soft_delete(&self, id: &NoteId, at_ms: i64) -> Result<(), DomainError> {
    let id_str = id.to_string();
    self.db
        .call(move |c| {
            let tx = c.unchecked_transaction()?;
            let changed = tx.execute(
                "UPDATE notes SET deleted_at = ?2, version = version + 1,
                    embedded_hash = NULL, embedded_model = NULL
                 WHERE id = ?1 AND deleted_at IS NULL",
                params![id_str, at_ms],
            )?;
            if changed > 0 {
                tx.execute("DELETE FROM notes_fts WHERE note_id = ?1", params![id_str])?;
                tx.execute(
                    "DELETE FROM chunk_vectors WHERE chunk_id LIKE ?1",
                    params![format!("{}#%", id_str)],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
        .await
}
```

- [ ] **Step 2: Update test `soft_delete_removes_vector`**

Replace the test (around line 392):

```rust
#[tokio::test]
async fn soft_delete_removes_vector() {
    let db = Database::open_in_memory().unwrap();
    let repo = SqliteNoteRepository::new(db.clone());
    let id = NoteId::new();
    repo.upsert(&sample(id, "Hello")).await.unwrap();

    let id_str = id.to_string();
    db.call(move |c| {
        let blob = vec![0u8; 384 * 4];
        c.execute(
            "INSERT INTO chunk_vectors (chunk_id, embedding) VALUES (?1, ?2)",
            rusqlite::params![format!("{}#0", id_str), blob],
        )?;
        Ok(())
    })
    .await
    .unwrap();
    assert_eq!(chunk_count(&db, &id.to_string()).await, 1);

    repo.soft_delete(&id, 2000).await.unwrap();
    assert_eq!(chunk_count(&db, &id.to_string()).await, 0);
}
```

Also add a helper `chunk_count` in the test module:

```rust
async fn chunk_count(db: &Database, note_id: &str) -> i64 {
    db.call(move |c| {
        c.query_row(
            "SELECT count(*) FROM chunk_vectors WHERE chunk_id LIKE ?1",
            [format!("{}#%", note_id)],
            |r| r.get(0),
        )
    })
    .await
    .unwrap()
}
```

- [ ] **Step 3: Run tests to verify they pass**

Run: `cd src-tauri && cargo test -p raki-storage --lib -- notes::tests::soft_delete_removes_vector -v`

Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-storage/src/notes.rs
git commit -m "feat(storage): soft_delete cleans up chunk_vectors"
```

---

### Task 12: Full suite verification

**Files:**
- All touched files

- [ ] **Step 1: Run deterministic suite**

Run: `cd src-tauri && cargo test --workspace --exclude raki`

Expected: All tests PASS. If any fail, fix the issue and re-run.

- [ ] **Step 2: Run clippy and fmt**

Run: `cd src-tauri && cargo clippy --workspace --exclude raki --all-targets -- -D warnings && cargo fmt --check`

Expected: Clean (no warnings, no fmt diffs).

- [ ] **Step 3: Verify frontend is untouched**

Run: `npx tsc --noEmit && npx vitest run`

Expected: PASS (no frontend changes in this slice).

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: R2 chunk-level embedding migration — full suite green"
```

---

### Task 13: Manual verification (developer-run)

- [ ] **Step 1: Build and run the app**

```bash
cd src-tauri && cargo tauri dev
```

- [ ] **Step 2: Manual walkthrough**

1. Create a long note (2+ paragraphs) with a specific phrase in the second paragraph.
2. Save and wait 5-10 seconds for background indexing.
3. Search for the specific phrase → the note should appear in results.
4. Edit the note → save → wait for re-index.
5. Search again → note still appears.
6. Delete the note → search → note should NOT appear.
7. Open SQLite browser, verify `chunk_vectors` has multiple rows for the long note.

- [ ] **Step 3: Document results**

If any step fails, file a bug and fix before claiming Done.

---

## Self-Review

**1. Spec coverage:**

| Spec Section | Plan Task |
|---|---|
| D1 — Compound chunk IDs | Task 1 (VectorIndex trait), Task 9 (vectors.rs) |
| D2 — V7 migration | Task 8 |
| D3 — Chunking logic | Task 2 (body_to_blocks), Task 3 (chunk_note) |
| D4 — embed_one batch pipeline | Task 4 (indexing.rs) |
| D5 — Search min-rank rollup | Task 6 |
| D6 — soft_delete cleanup | Task 11 |
| D7/D8 — Forward seams | Acknowledged in code comments; no implementation needed |
| D9 — Measurement plan | Task 13 (manual verification enables eval) |
| Honesty clause — irreversible bet | Task 8 (note_vectors preserved) |
| Feature flag — contextual prefix | Task 4 (EmbedConfig) |

**2. Placeholder scan:** No TBD, TODO, or vague requirements found. Every step has code or exact commands.

**3. Type consistency:**
- `PendingNote` has `title: String, body: String` consistently across domain, storage, and memory.
- `VectorIndex` has `delete_by_prefix` and `upsert_batch` consistently across domain and storage.
- `hybrid_candidates` returns `Vec<NoteId>` consistently across retrieval and app commands.
- Chunk IDs use `"{note_id}#{index}"` format consistently in storage and retrieval.

No gaps found. Plan is ready for execution.
