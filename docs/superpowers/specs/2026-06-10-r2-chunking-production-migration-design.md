# R2 — Production Chunk-Level Embedding Migration (Approach A+)

**Date:** 2026-06-10
**Status:** Design — approved
**Governing prior art:**
- `docs/superpowers/specs/2026-06-06-chunking-eval-substrate-design.md` (chunking substrate, D8 promotion gate)
- `docs/superpowers/specs/2026-06-10-r2-chunking-baseline-record-design.md` (synthetic baseline, winning arm)
- ADRs: ADR-0003 (sqlite-vec), ADR-0004 (ProseMirror JSON), ADR-0006 (staged retrieval), ADR-0007 (measurement-gated)
**Roadmap:** `docs/ROADMAP.md` Track A — R2 production migration

---

## Honesty clause (read first)

The synthetic baseline recorded `bare/min-rank` as the design-settled winning arm, but the **binding verdict** (D8: +0.05 Success@3 on the long-note stratum, real notes, by 2026-09-06) remains open. This slice ships the **production infrastructure** for chunk-level retrieval — schema, pipeline, search rollup — so that real-notes evaluation can run against an actual chunking deployment, not a mocked one. The eval itself (growing the real corpus, running the comparison, deciding the verdict) is a **separate, ongoing activity** that this slice enables but does not complete.

**This slice is an irreversible bet.** V7 creates `chunk_vectors` but does **not** drop `note_vectors` — the old table is preserved as stale backup data. If chunking fails to clear D8, remediation requires a follow-up slice (recreating `note_vectors` and re-indexing). There is no runtime rollback switch.

This design also incorporates ecosystem research (2024-2026) that was not available when the original chunking spec was written. Two findings inform the design:
- **Anthropic Contextual Retrieval** (prepending note title + heading path to each chunk) reduces retrieval failures by 35-49% with zero LLM cost. It is implemented as a **feature flag** (`use_contextual_prefix`, default OFF) and gated by its own measurement before becoming the default.
- **Overlap is overrated.** A Jan 2026 systematic study found 10-20% overlap adds index bloat with no measurable recall gain. Production uses **zero overlap** with boundary-aligned blocks.

The eval baseline remains valid for the *structural boundary* decision; the contextual prefix is an **additive layer** behind a flag, not a replacement.

---

## What this is

A production migration from whole-note embedding to **chunk-level embedding** with **contextual prefixing**, implemented as:

1. **Schema migration (V7):** `note_vectors` → `chunk_vectors` with compound chunk IDs (`note_id#index`).
2. **Chunking logic:** Promoted from `raki-eval` to `raki-memory`, adapted to work on ProseMirror JSON bodies (Raki's canonical format), producing structural blocks with heading-context and a title-prefix.
3. **Indexing pipeline:** `embed_one` chunks the note, embeds all chunks in one batch call, deletes old chunks, upserts new chunks, and stamps the note.
4. **Search pipeline:** `vector_search` returns chunk IDs → parses note IDs → **min-rank rollup** to note-level ranking → hybrid backfill → optional rerank → hydrate notes (parent-document pattern).
5. **Lifecycle:** Note delete/soft-delete cleans up all chunks. Edit triggers re-chunking and re-embedding.
6. **Forward seams:** Late Chunking and Matryoshka truncation are architected as future additive layers, not rewrites.

---

## What this is NOT

- **Not** a new ADR or a change to the `chunk-eval` binary, synthetic fixtures, or the D8 promotion gate. The eval harness is untouched.
- **Not** ProseMirror block-ID-level chunking. Block IDs are not yet stored on content (ADR-0004 follow-up, unscheduled). The migration unit is **structural blocks extracted from ProseMirror JSON** — a proxy that aligns with markdown-block semantics.
- **Not** semantic or agentic chunking. Research shows marginal end-to-end gains for high compute cost. Structural blocks + contextual prefix is the proven sweet spot.
- **Not** a new embedding model. The existing `EmbeddingProvider` port and `fastembed` stack are retained. A future slice can swap to Nomic/Jina for Late Chunking without schema changes.
- **Not** HyDE, query expansion, or hierarchical indexing. These are Track A R3/R4 territory, gated on this foundation.

---

## Decisions

### D1 — Compound chunk IDs: `note_id#index`

Chunk IDs are strings of the form `"{uuid}#{index}"` — e.g. `note-uuid#0`, `note-uuid#1`. `source_id` now semantically means chunk ID. The note ID is recovered by splitting on `'#'` and taking the first segment.

The `VectorIndex` trait gains two entity-agnostic methods:
```rust
async fn delete_by_prefix(&self, prefix: &str) -> Result<(), DomainError>;
async fn upsert_batch(&self, items: &[(String, Embedding)]) -> Result<(), DomainError>;
```
`delete_by_prefix` removes all vectors whose `source_id` starts with the given prefix (e.g. `"note-id#"`). `upsert_batch` inserts multiple vectors in one operation. Both are generic — they work for any entity that uses prefixed IDs. No entity-specific semantics leak into the port.

**Why not auxiliary columns or a metadata table?** sqlite-vec auxiliary columns are alpha-stage. A separate `chunk_meta` table adds a second write path and a lookup join for every query. Compound IDs are zero-overhead, zero-trait-changes, and the parsing cost is negligible compared to vector search.

**Why not stable block hashes?** Block IDs are not yet persisted in storage (ADR-0004 follow-up). Using an integer index is simple, deterministic, and stable across re-embeds of the same note content. If the block-ID wiring lands later, the ID scheme can migrate to `note_id#block_id` without changing the schema.

### D2 — Schema migration (V7)

```sql
-- Create the new chunk-level vector table alongside the old one.
-- note_vectors is NOT dropped — it is preserved as stale backup data.
CREATE VIRTUAL TABLE chunk_vectors USING vec0(
    chunk_id TEXT PRIMARY KEY,
    embedding float[384]
);

-- Force a complete re-index into the new table: every note's chunk embedding is stale.
UPDATE notes SET embedded_hash = NULL;
```

The migration is a **hard cutover** into `chunk_vectors`: old note-level vectors in `note_vectors` are left untouched but ignored. New chunk vectors are built by the background indexer on next start. `note_vectors` is preserved as an emergency fallback — if chunking fails to clear D8, a follow-up slice can recreate the whole-note pipeline without data loss. This is acceptable because:
- Raki has no production users yet (solo dev phase).
- The background indexer is idempotent and single-flight.
- Search gracefully degrades to keyword-only during the brief re-index window.

`embedded_hash`, `embedded_model`, and `content_hash` columns on `notes` are **retained unchanged**. Staleness tracking stays at the **note level** — when a note changes, all its chunks are invalidated and re-embedded. Chunk-level staleness is unnecessary complexity for personal scale.

After V7, every note is pending. `list_pending` queries `ORDER BY updated_at DESC` so recently edited notes index first, preventing starvation of active content during the full-corpus re-index.

### D3 — Chunking logic lives in `raki-memory`, works on ProseMirror JSON

The eval's `to_blocks` + `chunk` + `cap_split` are promoted from `raki-eval` to `raki-memory` and adapted:

- **Source format:** ProseMirror JSON (ADR-0004 canonical), not markdown.
- **Extractor:** `body_to_blocks(body: &str) -> Vec<Block>` parses the ProseMirror `doc` tree:
  - Top-level `paragraph` → one block
  - Top-level `bulletList` / `orderedList` → **one block** (all items joined, matching eval D2)
  - Top-level `codeBlock` → one block
  - `heading` updates running section context for subsequent blocks (heading text is not a standalone block)
  - Other nodes → best-effort text extraction
- **Block struct:** `{ heading: Option<String>, text: String }`
- **Prefix (feature-gated):** When `use_contextual_prefix` is ON, `"{title} > {heading}: {block_text}"` (contextual retrieval pattern). If no heading, `"{title}: {block_text}"`. When OFF, bare block text only (eval baseline).
- **Cap split:** Blocks exceeding `CHUNK_CHAR_CAP` (1,600 chars, ~<512 tokens) are split with `cap_split` (eval-proven).
- **No overlap** between chunks.
- **Chunk cap:** Max 32 chunks per note. Excess blocks are dropped with a `tracing::warn` log. At personal scale, notes exceeding 32 blocks (~51KB text) are pathological.

The chunking function signature:
```rust
pub fn chunk_note(title: &str, body: &str, use_prefix: bool) -> Vec<String>;
```
Returns the list of chunk texts to embed. If `body_to_blocks` yields zero blocks (e.g. doc contains only images or horizontal rules), returns `["{title}"`] regardless of body emptiness.

### D4 — `PendingNote` carries title + body; `embed_one` becomes batch-chunk embed

`PendingNote` changes from pre-flattened `text` to raw `title` + `body`:

```rust
pub struct PendingNote {
    pub id: NoteId,
    pub title: String,
    pub body: String,
    pub content_hash: String,
}
```

`embed_one` pipeline:
1. **Chunk:** `let chunks = chunk_note(&note.title, &note.body, config.use_contextual_prefix);`
2. **Embed batch:** `let embeddings = embedder.embed(&chunks).await?;` — single batch call. Max 32 chunks per note (see D3).
3. **Delete old chunks:** `vectors.delete_by_prefix(&format!("{}#", note.id)).await?` — removes all prior `note_id#%` chunks.
4. **Upsert batch:** `vectors.upsert_batch(&chunk_items).await?` — inserts all chunks in one operation.
5. **Stamp:** `store.mark_embedded(&note.id, &note.content_hash, model_id).await`.

Failure isolation: if step 2 fails, old chunks remain (no degradation). Steps 3 and 4 are separate storage operations; a crash between them leaves the note with zero chunks. The next indexing pass will re-embed fully because the stamp didn't happen. This window is bounded by the single-note scope and is acceptable for personal scale.

**Race condition (M6):** If a note is soft-deleted after `list_pending` yields it but before `mark_embedded`, `embed_one` will upsert fresh chunks for the deleted note. `mark_embedded` returns `false` (CAS guard checks `deleted_at IS NULL`), but the new chunks are already committed. A compensating `delete_by_prefix` call is added when `mark_embedded` returns `false` to clean up these orphaned chunks.

### D5 — Search pipeline: chunk IDs → note IDs → min-rank rollup

`raki-retrieval::vector_search` returns chunk IDs from `VectorIndex::query`. The caller (`hybrid_candidates`) parses note IDs:

```rust
fn note_id_from_chunk(chunk_id: &str) -> NoteId {
    let raw = chunk_id.split('#').next().unwrap_or(chunk_id);
    NoteId::parse(raw).expect("chunk ID must start with a valid note ID")
}
```

**Type contract:** `hybrid_candidates` MUST return `Vec<NoteId>` — never raw chunk IDs. The rollup happens inside `vector_search` before `hybrid_candidates` sees the data. Unit tests assert that `hybrid_candidates`'s output contains only valid `NoteId`s and that chunk IDs never leak through.

Min-rank rollup: iterate chunk hits in order; emit the note ID the **first time** it appears. This is the eval-proven `MinRank` aggregation, preserving vector's ranking authority.

`hybrid_candidates` (vector-primary, keyword-backfill) operates on **note IDs** after rollup:
1. Vector recall: `vector_search(vectors, embedder, query, pool)` → chunk IDs → note IDs (min-rank).
2. Keyword recall: `search(keyword, query, pool)` → note IDs.
3. Union: vector note IDs first (authoritative), then keyword-only note IDs appended.

`hybrid_search` truncates to `k` note IDs. The reranker (if present) sees full note text (parent-document pattern). Chunking affects **recall only**, not reranking or presentation.

### D6 — Note delete/soft-delete cleans up chunks

`SqliteNoteRepository::soft_delete` and any hard-delete path must delete a note's chunks:

```sql
DELETE FROM chunk_vectors WHERE chunk_id LIKE ?1
-- ?1 = format!("{}#%", note_id)
```

This is executed in the same transaction as the note delete/soft-delete to maintain Principle 5 (one source of truth).

### D7 — Forward seam: Late Chunking

The `embed_one` function is structured so that "how chunks are embedded" is swappable in a future slice. Today's implementation is **chunk-then-embed**: split text, embed each chunk independently. **Late Chunking** (future) embeds the full note token-level, then mean-pools to chunk boundaries — but the output is identical (chunk IDs + vectors), so no schema or search code changes.

Late Chunking requires a long-context embedding model (8K+ tokens) and token-level access to the model's hidden states. The `EmbeddingProvider` port can be extended with an `embed_with_token_vectors` method when that slice is undertaken.

### D8 — Forward seam: Matryoshka dimensions

Vectors are stored at full dimension (384d for bge-small). The `embedded_model` column records the model ID. A future slice can:
1. Swap to a Matryoshka-capable model (Nomic v2, 768d)
2. Store full vectors
3. Benchmark truncation to 256d / 128d
4. If quality is acceptable, migrate by re-stamping `embedded_model` with a dimension suffix and letting the indexer re-embed at truncated size

No schema change needed — the `vec0` table stores `float[N]` where N is declared at creation. A dimension change requires recreating the virtual table (a migration).

### D9 — Measurement plan: real-notes eval against production chunking

This slice enables the binding D8 verdict by providing a real chunking deployment to measure against. The protocol:

1. **Grow `eval-data/real`** with 50-100 representative queries against the user's real notes.
2. **Length stratification:** short (<~200 tokens), medium, long notes.
3. **Run `chunk-eval` against the real corpus** using the production `chunk_note` function (not the markdown proxy).
4. **Metrics:** Recall@5, MRR, nDCG@10 on retrieval; end-to-end QA accuracy (LLM-as-judge using `raki-generate`).
5. **Compare:** whole-note baseline (re-index with a mock `ChunkStrategy::WholeNote`) vs production chunking.
6. **Verdict:** Chunking is the permanent architecture if it beats whole-note by ≥ +0.05 Success@3 on the long stratum (D8). If not, a follow-up slice decides remediation (improving chunking or reverting to whole-note via a new migration).

**Privacy & egress:** All eval data is local-only (files on disk, never transmitted). The LLM-as-judge step MUST use a **local provider** (Ollama) or obtain explicit per-provider consent via the `raki-ai` egress gate with a logged `EgressDecision`. Real note excerpts used as context in eval queries must not leave the device without consent. Eval data is stored with restrictive file permissions and auto-purged after the verdict is recorded.

**Acknowledged risk (M5):** `content_hash` covers `(title, body)` but not chunking algorithm version. A future tweak to prefix format, cap threshold, or block extractor will leave embeddings stale. This is accepted as a known risk for this slice. A follow-up spec will introduce `embedding_alg_version` if algorithm changes occur before the D8 verdict.

---

## Architecture / data flow

### Ingestion (save note → background index)

```
User saves note
  → NoteRepository::upsert (transactional: notes row + notes_fts)
  → content_hash updated, embedded_hash ≠ content_hash → note is pending
  → IndexingService::trigger (single-flight background job)
    → IndexingStore::list_pending → PendingNote { id, title, body, content_hash }
    → embed_one:
        1. chunk_note(title, body, use_prefix) → Vec<String>  -- structural blocks + optional prefix
        2. EmbeddingProvider::embed(chunks)                    -- batch call (max 32)
        3. VectorIndex::delete_by_prefix("id#")                -- remove old chunks
        4. VectorIndex::upsert_batch([("id#0", emb0), ...])   -- atomic batch insert
        5. IndexingStore::mark_embedded(id, hash, model)
```

### Search (query → ranked notes)

```
User searches "payment method"
  → search_notes command
    → embedder.embed_query(["payment method"]) → query vector
    → hybrid_candidates:
        vector:  VectorIndex::query(q_vec, pool=100) → chunk hits [id#0, id#2, other#1, ...]
                 → parse note IDs → [id, other, ...] (min-rank: first occurrence wins)
        keyword: KeywordIndex::query("payment method", pool=100) → note hits [other, third, ...]
        union:   [id, other, third, ...] (vector-first, keyword backfill)
    → hydrate: NoteRepository::get(id) for each union ID
    → rerank (optional): cross-encoder scores (query, note_text) → reordered note IDs
    → truncate to K=20
    → return Vec<NoteDto>
```

### Delete / soft-delete

```
User deletes note
  → NoteRepository::soft_delete(id) (transactional: notes row + notes_fts + chunk_vectors)
  → direct SQL: DELETE FROM chunk_vectors WHERE chunk_id LIKE 'note_id#%'
```

---

## Components touched

```
crates/raki-domain/src/ports.rs                  MODIFY  PendingNote: text → title + body; VectorIndex: add delete_by_prefix + upsert_batch
crates/raki-domain/src/body.rs                   MODIFY  Add body_to_blocks() (ProseMirror-aware block extractor)
crates/raki-memory/src/lib.rs                    MODIFY  Add chunk_note(title, body, use_prefix) → Vec<String> + cap_split
crates/raki-memory/src/indexing.rs               MODIFY  embed_one: batch-chunk pipeline; delete after embed; upsert_batch
crates/raki-retrieval/src/search.rs              MODIFY  vector_search returns chunk IDs; hybrid_candidates parses + min-rank rollup; type contract: NoteId only
crates/raki-storage/src/migrations.rs            MODIFY  V7: create chunk_vectors (note_vectors preserved), clear embedded_hash
crates/raki-storage/src/vectors.rs               MODIFY  SqliteVectorIndex: table=chunk_vectors; impl delete_by_prefix + upsert_batch
crates/raki-storage/src/indexing.rs              MODIFY  SqliteIndexingStore::list_pending returns title + body; ORDER BY updated_at DESC
crates/raki-storage/src/notes.rs                 MODIFY  soft_delete: delete chunks via direct SQL on chunk_vectors
crates/raki-app/src/commands/notes.rs            NO CHANGE (search_reranked delegates to raki-retrieval; contract unchanged)
crates/raki-app/src/state.rs                     NO CHANGE
crates/raki-eval/                               NO CHANGE (eval harness untouched)
```

---

## Testing & verification

### Deterministic (CI path)

- `cargo test --workspace --exclude raki` — all existing tests pass.
- New unit tests:
  - `body_to_blocks`: ProseMirror JSON with paragraphs, lists, code blocks, headings → correct block count and heading context.
  - `chunk_note`: title + body → chunks; `use_prefix=true` → contextual prefix; `use_prefix=false` → bare blocks; empty body → `["{title}"]`; zero-block non-empty body → `["{title}"]`.
  - `chunk_note` chunk cap: body producing >32 blocks → truncated to 32 with warning log.
  - `cap_split`: never silently truncates; all words preserved.
  - `note_id_from_chunk`: parses correctly; returns valid `NoteId`; panics on malformed input.
  - `hybrid_candidates` with chunk IDs: min-rank rollup deduplicates correctly; vector order preserved; output contains only `NoteId`, never raw chunk IDs.
  - `embed_one` idempotency: re-embedding a note replaces old chunks (not appends).
  - `soft_delete` cleans up chunks: after delete, `SELECT count(*) FROM chunk_vectors WHERE chunk_id LIKE 'note_id#%'` returns 0.
- New integration test:
  - `v6_to_v7_migration_on_populated_db`: create temp SQLite at schema V6, insert notes with `note_vectors`, run V7 migration, assert `note_vectors` still exists, `chunk_vectors` exists, all `embedded_hash` are NULL, app initializes cleanly.
- `cargo clippy --workspace --exclude raki --all-targets -- -D warnings`
- `cargo fmt --check`
- Frontend: `bun run typecheck && bun run build` (no frontend changes expected).

### Integration (real model)

- `cargo test -p raki --release search_reranked_handles_chunked_notes -- --ignored --nocapture` (manual): index 5 notes with varied structure, search, verify relevant note returned.
- Manual `tauri dev` walkthrough:
  1. Create a long note with a buried fact.
  2. Wait for background indexing.
  3. Search for the buried fact → note appears.
  4. Edit the note → re-index triggers.
  5. Delete the note → search no longer returns it.
  6. Check SQLite: `chunk_vectors` has multiple rows for long notes, one for short notes.

### Measurement (post-deployment)

- Grow `eval-data/real` to 20+ queries.
- Run `chunk-eval` with real corpus using production `chunk_note`.
- Record metrics: Recall@5, MRR, nDCG@10 vs whole-note baseline.
- Decision: keep chunking if it clears D8 threshold on long stratum.

---

## Definition of Done

1. **V7 migration** applies cleanly on populated DBs: `chunk_vectors` is created alongside preserved `note_vectors`, `embedded_hash` is cleared, and the app starts without error.
2. **`PendingNote`** carries `title` + `body` (raw ProseMirror JSON), not pre-flattened `text`. `SqliteIndexingStore::list_pending` populates both fields.
3. **`chunk_note`** in `raki-memory` produces structural blocks from ProseMirror JSON with heading context. Bare blocks are the default; contextual prefix (`"{title} > {heading}: {text}"`) is available behind `use_contextual_prefix` flag (default OFF). It is unit-tested with fabricated ProseMirror JSON.
4. **`embed_one`** in `raki-memory` deletes old chunks via `delete_by_prefix`, batches new chunk embeddings, upserts all chunks via `upsert_batch` with `note_id#index` IDs, and stamps the note. Re-embedding is idempotent (no orphaned chunks).
5. **`vector_search`** in `raki-retrieval` returns chunk IDs; `hybrid_candidates` parses note IDs and applies min-rank rollup. Unit-tested with fake chunk IDs.
6. **`soft_delete`** in `raki-storage` removes all chunks for the deleted note. Integration-tested.
7. **Deterministic suite + clippy + fmt green.** Frontend untouched and passing.
8. **Manual walkthrough** (`tauri dev`) confirms: create → index → search → edit → re-index → delete → no ghost results.
9. **ROADMAP updated:** R2 status flips from "design-settled" to "▶ production migration deployed; binding verdict now measurable on real notes."
