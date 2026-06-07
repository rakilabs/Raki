# Chunk-Level Retrieval Eval Substrate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add chunk-level (structural-block) embedding to the `raki-eval` pipeline so retrieval can be measured whole-note vs chunked — across prefix and aggregation arms, on synthetic and real notes — without touching production storage or retrieval.

**Architecture:** All changes live in `raki-eval`. A pure `to_blocks` (markdown → content blocks) and a pure `chunk()` (note → chunk texts, token-capped) feed a new chunked index-build path in `run_eval_over`, gated by a `ChunkStrategy` arg whose `WholeNote` value is byte-identical to today's behavior (so the keyword snapshot gate is untouched). Chunk hits roll up to notes by min-rank (free) or score-max (scored ports). A `chunk-eval` binary runs the arms and prints whole-vs-chunked deltas.

**Tech Stack:** Rust, `raki-eval` driver crate, `pulldown-cmark` (already a dep), real bge embedder + jina reranker (reused from Slices 1–4), `raki-domain` ports (`VectorIndex::query`, `Reranker::rerank`).

**Spec:** `docs/superpowers/specs/2026-06-06-chunking-eval-substrate-design.md` (D1–D11 + Limitations). This plan implements all of it.

**Verified facts (read before starting):**
- `run_eval_over(corpus: &[CorpusNote], queries: &[EvalQuery], embedder: Arc<dyn EmbeddingProvider>, reranker: Arc<dyn Reranker>, k: usize) -> Result<EvalRun, DomainError>` in `crates/raki-eval/src/lib.rs:~260`. Per CorpusNote it mints a stable uuid `00000000-0000-7000-8000-{idx+1:012x}`, `repo.upsert`s the whole note (FTS5/keyword), embeds `"{title}\n\n{body}"`, `vectors.upsert(uuid, emb)`, and records `fixture_of[uuid]=slug`, `text_of[slug]=doc`. The per-query loop maps retrieved uuids→slugs via `to_fixture` then `score_one`. `run_eval` calls `run_eval_over(&load_corpus(), &load_queries(), …)`.
- `to_fixture(uuids: &[String], fixture_of: &HashMap<String,String>) -> Vec<String>` (`lib.rs:456`) maps ids→slugs via `filter_map` (drops unknown ids).
- `CorpusNote { id, title, body }`; `EvalQuery { query, category, set, relevant_ids, grades, primary }` (all `pub`).
- Metric primitives in `raki-retrieval`: `recall_at_k`, `reciprocal_rank`, etc. Retrieval id-only wrappers `vector_search`/`rerank` return `Vec<String>` and **discard scores**; the scored ports `VectorIndex::query -> Vec<VectorHit{source_id, distance}>` (lower distance = better) and `Reranker::rerank -> Vec<RerankScore{index, score}>` (higher = better) are what score-max uses.
- The deterministic gate `keyword_snapshot_is_deterministic` (`tests/eval_gate.rs`, NOT `#[ignore]`) checks the **Keyword** method only. Keyword retrieval is FTS5 over the whole `notes` table — unaffected by chunking as long as `repo.upsert(whole note)` still happens. This is the refactor guard for Task 3.
- `markdown.rs` already has `strip_frontmatter` (pub), `to_plain_text`, `first_h1` (uses `pulldown_cmark::{Event, Parser, Tag, TagEnd, HeadingLevel}`).
- `local_corpus::load_local(dir) -> Result<LocalData, LoadError>` sets `CorpusNote.body = to_plain_text(strip_frontmatter(raw))` — **collapses** paragraph boundaries, so it cannot feed chunking (Task 7 adds a raw-markdown loader).

---

## File Structure

```
raki-eval/src/markdown.rs            MODIFY  add Block + to_blocks (content blocks + nearest heading; list = one block)
raki-eval/src/chunk.rs               CREATE  ChunkStrategy/PrefixMode/Rollup; chunk() (texts, token-capped); cap_split
raki-eval/src/lib.rs                 MODIFY  run_eval_over gains (strategy, prefix, rollup); chunked index build; dedup_to_note; score-max rollup; run_eval passes WholeNote/Title/MinRank
raki-eval/src/bin/chunk-eval.rs      CREATE  run arms over synthetic + real; whole-vs-chunked deltas; length stratification; split counts
raki-eval/src/local_corpus.rs        MODIFY  load_local_raw (body = raw markdown, frontmatter stripped, NOT collapsed)
raki-eval/fixtures/chunking/         CREATE  committed synthetic corpus.json + queries.json (D6 controls)
raki-eval/Cargo.toml                 MODIFY  [[bin]] chunk-eval
docs/eval/real-data-protocol.md      MODIFY  messiest-notes sampling + length stratification (D7)
```

`run_eval`, the synthetic `fixtures/corpus.json`, `snapshot.json`, the gate, and `search_notes` are **untouched**.

---

## Task 1: `to_blocks` — markdown → content blocks with heading context

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/markdown.rs`

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` in `markdown.rs`:

```rust
    #[test]
    fn to_blocks_splits_content_and_tracks_heading_with_list_as_one_block() {
        let md = "---\ntags: [x]\n---\n# Title\n\n## Logistics\nCheck-in is 3pm. Payment is cash only.\n\n- milk\n- eggs\n- bread\n\n```rust\nfn main() { println!(\"hi\"); }\n```\n";
        let blocks = to_blocks(md);
        // frontmatter gone; the H1 is folded as context, not a content block.
        // content blocks: the Logistics paragraph, the whole list (ONE block), the code block.
        assert_eq!(blocks.len(), 3, "para + whole-list + code = 3 content blocks");
        // the paragraph carries its nearest heading (the H2), not a standalone heading chunk.
        let para = &blocks[0];
        assert_eq!(para.heading.as_deref(), Some("Logistics"));
        assert!(para.text.contains("Payment is cash only"));
        // the list is a single block joining its items.
        let list = &blocks[1];
        assert!(list.text.contains("milk") && list.text.contains("eggs") && list.text.contains("bread"));
        // code contents survive intact; fences/lang do not leak.
        let code = &blocks[2];
        assert!(code.text.contains("fn main() { println!(\"hi\"); }"));
        assert!(!code.text.contains("```"));
        assert!(blocks.iter().all(|b| !b.text.contains("tags:")));
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-eval --lib to_blocks_splits_content`
Expected: FAIL — `to_blocks` / `Block` not defined.

- [ ] **Step 3: Implement `Block` + `to_blocks`**

In `markdown.rs`, add (the `use` line already imports `Event, Parser, Tag, TagEnd`; add `HeadingLevel` if not present):

```rust
/// One content block of a note, with the nearest preceding heading as section context.
/// Headings are NOT standalone blocks; they annotate the blocks beneath them.
#[derive(Debug, Clone)]
pub struct Block {
    pub heading: Option<String>,
    pub text: String,
}

/// Split markdown into content blocks: each paragraph, each WHOLE list (items joined — not one
/// block per item), and each code block. A heading updates the running section context applied to
/// the blocks that follow it. Frontmatter is stripped; HTML is dropped; code contents are kept.
pub fn to_blocks(md: &str) -> Vec<Block> {
    use pulldown_cmark::HeadingLevel;
    let body = strip_frontmatter(md);
    let mut blocks = Vec::new();
    let mut heading: Option<String> = None;

    // Accumulators for the block currently being assembled.
    let mut buf = String::new();
    let mut in_heading = false;
    let mut heading_buf = String::new();
    let mut list_depth: usize = 0; // >0 while inside a list: keep items in ONE block
    let mut in_code = false;

    let flush = |buf: &mut String, blocks: &mut Vec<Block>, heading: &Option<String>| {
        let t: String = buf.split_whitespace().collect::<Vec<_>>().join(" ");
        if !t.is_empty() {
            blocks.push(Block { heading: heading.clone(), text: t });
        }
        buf.clear();
    };

    for event in Parser::new(body) {
        match event {
            Event::Start(Tag::Heading { .. }) => {
                in_heading = true;
                heading_buf.clear();
            }
            Event::End(TagEnd::Heading(_)) => {
                in_heading = false;
                heading = Some(heading_buf.trim().to_string()).filter(|s| !s.is_empty());
            }
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => {
                list_depth = list_depth.saturating_sub(1);
                if list_depth == 0 {
                    flush(&mut buf, &mut blocks, &heading); // whole list = one block
                }
            }
            Event::Start(Tag::CodeBlock(_)) => in_code = true,
            Event::End(TagEnd::CodeBlock) => {
                in_code = false;
                flush(&mut buf, &mut blocks, &heading);
            }
            Event::End(TagEnd::Paragraph) => {
                if list_depth == 0 {
                    flush(&mut buf, &mut blocks, &heading);
                } else {
                    buf.push(' '); // paragraph inside a list item: keep accumulating
                }
            }
            Event::End(TagEnd::Item) => buf.push(' '),
            Event::Text(t) | Event::Code(t) => {
                if in_heading {
                    heading_buf.push_str(&t);
                } else {
                    buf.push_str(&t);
                    if in_code {
                        buf.push(' ');
                    }
                }
            }
            Event::SoftBreak | Event::HardBreak => buf.push(' '),
            _ => {}
        }
    }
    flush(&mut buf, &mut blocks, &heading); // trailing block, if any
    blocks
}
```

- [ ] **Step 4: Run the test**

Run: `cd src-tauri && cargo test -p raki-eval --lib to_blocks_splits_content`
Expected: PASS. (If `pulldown-cmark 0.12`'s `TagEnd::List`/`TagEnd::Item` arity differs, adjust the match arms; the event *kinds* are stable.)

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/src/markdown.rs
git commit -m "Add to_blocks: markdown content blocks with heading context (list = one block)"
```

---

## Task 2: `chunk.rs` — strategy, prefix arms, token-cap split

**Files:**
- Create: `src-tauri/crates/raki-eval/src/chunk.rs`
- Modify: `src-tauri/crates/raki-eval/src/lib.rs` (add `pub mod chunk;`)

- [ ] **Step 1: Create the module with failing tests**

Create `src-tauri/crates/raki-eval/src/chunk.rs`:

```rust
//! Chunking strategies for the eval. Pure: turns a note's (title, body) into the chunk *texts*
//! to embed. The eval composes chunk *ids* (note-uuid for WholeNote, `uuid#i` for Blocks).

use crate::markdown::to_blocks;

/// Granularity: the whole note as one chunk (today's behavior) vs structural blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkStrategy {
    WholeNote,
    Blocks,
}

/// What context to prepend to a block chunk (a measured arm; D6). Inert for WholeNote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixMode {
    Bare,
    Title,
    TitleHeading,
}

/// How chunk hits roll up to a note ranking (a measured arm; D4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rollup {
    MinRank,
    ScoreMax,
}

/// Approximate per-chunk character cap (~ conservative <512 bge tokens at ~3.1 chars/token).
/// A correctness floor against silent embedding truncation (D2), NOT quality tuning.
pub const CHUNK_CHAR_CAP: usize = 1600;

/// Split `text` into pieces no longer than `cap` chars, breaking on a space near the cap when
/// possible (never silently truncating). Returns at least one piece.
pub fn cap_split(text: &str, cap: usize) -> Vec<String> {
    if text.len() <= cap {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut rest = text;
    while rest.len() > cap {
        // Find a char boundary <= cap; prefer the last space before it for a clean break.
        let mut end = cap;
        while end > 0 && !rest.is_char_boundary(end) {
            end -= 1;
        }
        let split = rest[..end].rfind(' ').map(|s| s + 1).unwrap_or(end).max(1);
        out.push(rest[..split].trim().to_string());
        rest = &rest[split..];
    }
    if !rest.trim().is_empty() {
        out.push(rest.trim().to_string());
    }
    out
}

/// Produce the chunk texts to embed. `WholeNote` returns exactly `["{title}\n\n{body}"]` (byte-
/// identical to the legacy path). `Blocks` splits the body, applies the prefix arm, and token-caps.
pub fn chunk(title: &str, body: &str, strategy: ChunkStrategy, prefix: PrefixMode) -> Vec<String> {
    match strategy {
        ChunkStrategy::WholeNote => vec![format!("{title}\n\n{body}")],
        ChunkStrategy::Blocks => {
            let mut out = Vec::new();
            for b in to_blocks(body) {
                let prefixed = match prefix {
                    PrefixMode::Bare => b.text.clone(),
                    PrefixMode::Title => format!("{title} — {}", b.text),
                    PrefixMode::TitleHeading => match &b.heading {
                        Some(h) => format!("{title} — {h} — {}", b.text),
                        None => format!("{title} — {}", b.text),
                    },
                };
                out.extend(cap_split(&prefixed, CHUNK_CHAR_CAP));
            }
            if out.is_empty() {
                // A body with no parseable content blocks still needs one chunk.
                out.push(format!("{title}\n\n{body}"));
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_note_is_one_chunk_identical_to_legacy_doc() {
        let out = chunk("Title", "Body text.", ChunkStrategy::WholeNote, PrefixMode::TitleHeading);
        assert_eq!(out, vec!["Title\n\nBody text.".to_string()]);
    }

    #[test]
    fn blocks_split_and_apply_prefix_arms() {
        let body = "## Sec\nFirst para fact.\n\nSecond para.\n";
        let bare = chunk("T", body, ChunkStrategy::Blocks, PrefixMode::Bare);
        assert_eq!(bare.len(), 2);
        assert!(bare[0].starts_with("First para"));
        let titled = chunk("T", body, ChunkStrategy::Blocks, PrefixMode::Title);
        assert!(titled[0].starts_with("T — First para"));
        let th = chunk("T", body, ChunkStrategy::Blocks, PrefixMode::TitleHeading);
        assert!(th[0].starts_with("T — Sec — First para"));
    }

    #[test]
    fn cap_split_never_truncates_a_long_block() {
        let long = "word ".repeat(1000); // ~5000 chars
        let pieces = cap_split(&long, CHUNK_CHAR_CAP);
        assert!(pieces.len() >= 3, "split into multiple pieces");
        assert!(pieces.iter().all(|p| p.len() <= CHUNK_CHAR_CAP));
        // every word is preserved across the pieces (no silent loss).
        let total_words: usize = pieces.iter().map(|p| p.split_whitespace().count()).sum();
        assert_eq!(total_words, 1000);
    }
}
```

- [ ] **Step 2: Wire the module + run**

In `lib.rs`, add `pub mod chunk;` with the other module declarations.
Run: `cd src-tauri && cargo test -p raki-eval --lib chunk`
Expected: PASS (all three tests).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/raki-eval/src/chunk.rs src-tauri/crates/raki-eval/src/lib.rs
git commit -m "Add chunk(): strategy + prefix arms + token-cap split (eval)"
```

---

## Task 3: chunked index build + `dedup_to_note` (min-rank) in `run_eval_over`

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`

- [ ] **Step 1: Confirm the refactor guard is green first**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate keyword_snapshot_is_deterministic`
Expected: PASS. It must still pass after this task (the chunked build must not change the Keyword path).

- [ ] **Step 2: Add `dedup_to_note` with a failing test**

In `lib.rs` `#[cfg(test)] mod tests`, add:

```rust
    #[test]
    fn dedup_to_note_keeps_first_occurrence_order() {
        // chunk hits map to slugs with repeats; min-rank = first occurrence per note.
        let slugs = vec!["a".into(), "a".into(), "b".into(), "a".into(), "c".into()];
        assert_eq!(dedup_to_note(&slugs), vec!["a".to_string(), "b".into(), "c".into()]);
        assert_eq!(dedup_to_note(&[]), Vec::<String>::new());
    }
```

- [ ] **Step 3: Implement `dedup_to_note`**

In `lib.rs` (near `to_fixture`):

```rust
/// Roll a chunk-level slug list up to a note ranking by MIN-RANK: a note's position is its
/// first (best-ranked) chunk. A no-op when each note has one chunk (WholeNote). NOT score-max
/// (a note's best-*scored* chunk) — see `score_max_notes` and the spec D4.
fn dedup_to_note(slugs: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for s in slugs {
        if seen.insert(s.clone()) {
            out.push(s.clone());
        }
    }
    out
}
```

- [ ] **Step 4: Run the unit test**

Run: `cd src-tauri && cargo test -p raki-eval --lib dedup_to_note_keeps_first_occurrence`
Expected: PASS.

- [ ] **Step 5: Add the `strategy`/`prefix`/`rollup` params and the chunked build**

Change `run_eval_over`'s signature and index build. Add the imports `use crate::chunk::{chunk, ChunkStrategy, PrefixMode, Rollup};` and `use raki_domain::Embedding;` if needed. New signature:

```rust
pub async fn run_eval_over(
    corpus: &[CorpusNote],
    queries: &[EvalQuery],
    embedder: Arc<dyn EmbeddingProvider>,
    reranker: Arc<dyn Reranker>,
    k: usize,
    strategy: ChunkStrategy,
    prefix: PrefixMode,
    rollup: Rollup,
) -> Result<EvalRun, DomainError> {
```

Replace the index-build loop body (the `for (idx, cn) in corpus.iter().enumerate()` block) with:

```rust
    // text_of is keyed by BOTH chunk id and note uuid → the text to (re)rank for that id.
    // fixture_of maps both chunk ids and the note uuid to the fixture slug.
    for (idx, cn) in corpus.iter().enumerate() {
        const DUMMY_EPOCH_MS: i64 = 1000;
        let mut note = Note::new(cn.title.clone(), cn.body.clone(), DUMMY_EPOCH_MS);
        note.id = NoteId::parse(&format!("00000000-0000-7000-8000-{:012x}", idx + 1))
            .expect("synthetic fixture uuid is well-formed");
        let uuid = note.id.to_string();
        repo.upsert(&note).await?; // keyword/FTS5 over the WHOLE note — unchanged, gate-safe.

        // Note-level entries: keyword hits return the uuid; keyword-backfilled rerank candidates
        // need the whole-note text.
        let whole = format!("{}\n\n{}", cn.title, cn.body);
        fixture_of.insert(uuid.clone(), cn.id.clone());
        text_of.insert(uuid.clone(), whole.clone());

        let texts = chunk(&cn.title, &cn.body, strategy, prefix);
        for (j, text) in texts.iter().enumerate() {
            // WholeNote (single chunk): id == note uuid, so the vector keying is byte-identical to
            // the legacy path. Blocks: `uuid#j`.
            let chunk_id = if strategy == ChunkStrategy::WholeNote {
                uuid.clone()
            } else {
                format!("{uuid}#{j}")
            };
            let emb = embedder.embed(std::slice::from_ref(text)).await?;
            let emb = emb.first().ok_or_else(|| {
                DomainError::Provider("embedder returned empty batch".to_string())
            })?;
            vectors.upsert(&chunk_id, emb).await?;
            fixture_of.insert(chunk_id.clone(), cn.id.clone());
            text_of.insert(chunk_id, text.clone());
        }
    }
```

Then make `run_eval` pass the defaults so behavior is unchanged:

```rust
pub async fn run_eval(
    embedder: Arc<dyn EmbeddingProvider>,
    reranker: Arc<dyn Reranker>,
    k: usize,
) -> Result<EvalRun, DomainError> {
    run_eval_over(
        &load_corpus(),
        &load_queries(),
        embedder,
        reranker,
        k,
        ChunkStrategy::WholeNote,
        PrefixMode::Title,
        Rollup::MinRank,
    )
    .await
}
```

- [ ] **Step 6: Roll up the vector/hybrid/keyword id lists to notes (min-rank)**

In the per-query loop, wrap each `to_fixture(...)` result for **vector**, **hybrid**, and **keyword** with `dedup_to_note(&...)`. Concretely, the existing `kw`, `vc`, `hy` bindings become e.g.:

```rust
        let kw = dedup_to_note(&to_fixture(&search(&keyword, &q.query, cov_k.max(k)).await?, &fixture_of));
        let vc = dedup_to_note(&to_fixture(&vector_search(&vectors, embedder.as_ref(), &q.query, cov_k.max(k)).await?, &fixture_of));
        let hy = dedup_to_note(&to_fixture(&hybrid_search(&keyword, &vectors, embedder.as_ref(), &q.query, cov_k.max(k)).await?, &fixture_of));
```

For the reranked leg, the candidate pool is now chunk ids; keep mapping each id to its text via `text_of` (which now holds chunk text), then roll up the reranked id list to notes:

```rust
        let pool_ids = to_fixture(
            &hybrid_candidates(&keyword, &vectors, embedder.as_ref(), &q.query, RERANK_POOL).await?,
            &fixture_of,
        );
        // NOTE: pool_ids are slugs after to_fixture, which loses chunk identity. For the reranked
        // leg we must rerank chunk TEXTS, so build candidates from the RAW (pre-to_fixture) ids:
```

Replace the rerank candidate construction to use raw ids (so distinct chunks of one note are ranked separately), then dedup after:

```rust
        let raw_pool = hybrid_candidates(&keyword, &vectors, embedder.as_ref(), &q.query, RERANK_POOL).await?;
        let candidates: Vec<(String, String)> = raw_pool
            .iter()
            .filter_map(|id| text_of.get(id).map(|t| (id.clone(), t.clone())))
            .collect();
        let rr_ids = rerank(reranker.as_ref(), &q.query, &candidates, RERANK_POOL).await?; // chunk ids, scored order
        let rr = dedup_to_note(&to_fixture(&rr_ids, &fixture_of));
```

(`rerank` is called with `RERANK_POOL` not `k` so the rollup has the full reordered pool to dedup before truncation in `score_one`/`truncate`. `score_one` already truncates internally via `recall_at_k(ranked, …, k)`-style metrics; the `MethodResult.ranked` is truncated by the existing `truncate(&rr, k)`.)

- [ ] **Step 7: Build + run the full eval test set (behavior preserved on WholeNote)**

Run: `cd src-tauri && cargo test -p raki-eval`
Expected: PASS — including `keyword_snapshot_is_deterministic` (the Keyword path is unchanged) and `harness_scores_every_category_with_fake_embedder`.

- [ ] **Step 8: Run the deterministic gate explicitly**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate keyword_snapshot_is_deterministic`
Expected: PASS — proves the chunked-capable build preserved synthetic Keyword behavior.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/crates/raki-eval/src/lib.rs
git commit -m "run_eval_over: chunked index build + min-rank note rollup (WholeNote unchanged)"
```

---

## Task 4: score-max rollup arm (scored ports)

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`

- [ ] **Step 1: Add `score_max_notes` with a failing test**

In `lib.rs` `mod tests`:

```rust
    #[test]
    fn score_max_orders_notes_by_best_chunk_and_can_differ_from_min_rank() {
        // chunk hits (id, distance) — lower distance = better. fixture maps chunks→notes.
        let mut fx = std::collections::HashMap::new();
        fx.insert("X#0".to_string(), "X".to_string());
        fx.insert("X#1".to_string(), "X".to_string());
        fx.insert("Y#0".to_string(), "Y".to_string());
        // rank order (by distance): X#0(0.30), Y#0(0.31), X#1(0.10)
        let hits = vec![
            (("X#0").to_string(), 0.30_f32),
            (("Y#0").to_string(), 0.31),
            (("X#1").to_string(), 0.10),
        ];
        // min-rank would be [X, Y] (X first at rank 1). score-max sees X's best is 0.10 < Y's 0.31,
        // and Y's best 0.31 — so X then Y; here they agree on X, but the BEST score for X is X#1
        // not X#0, which min-rank could never surface. Construct divergence with a third note:
        fx.insert("Z#0".to_string(), "Z".to_string());
        let hits2 = vec![
            (("Z#0").to_string(), 0.20_f32), // Z best 0.20, rank 1
            (("X#0").to_string(), 0.25),     // X first appears rank 2
            (("X#1").to_string(), 0.05),     // X best 0.05 (better than Z) but rank 3
        ];
        // min-rank: [Z, X]; score-max: [X, Z] (X's best 0.05 beats Z's 0.20).
        assert_eq!(score_max_notes(&hits2, &fx), vec!["X".to_string(), "Z".into()]);
    }
```

- [ ] **Step 2: Implement `score_max_notes`**

In `lib.rs`:

```rust
/// Roll chunk hits up to a note ranking by SCORE-MAX: each note's score is its best (lowest-
/// distance) chunk; notes are ordered best-first. Distinct from min-rank when a note's best chunk
/// is not its first-appearing chunk (after rerank, or with quantization noise) — see spec D4.
fn score_max_notes(hits: &[(String, f32)], fixture_of: &std::collections::HashMap<String, String>) -> Vec<String> {
    use std::collections::HashMap;
    let mut best: HashMap<String, f32> = HashMap::new();
    for (chunk_id, dist) in hits {
        if let Some(slug) = fixture_of.get(chunk_id) {
            best.entry(slug.clone()).and_modify(|d| { if *dist < *d { *d = *dist } }).or_insert(*dist);
        }
    }
    let mut notes: Vec<(String, f32)> = best.into_iter().collect();
    notes.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    notes.into_iter().map(|(slug, _)| slug).collect()
}
```

- [ ] **Step 3: Run the unit test**

Run: `cd src-tauri && cargo test -p raki-eval --lib score_max_orders_notes`
Expected: PASS — proves score-max genuinely differs from min-rank.

- [ ] **Step 4: Branch the vector + reranked legs on `rollup`**

In the per-query loop, replace the `vc` and `rr` bindings so that under `Rollup::ScoreMax` they use the scored ports. Vector:

```rust
        let q_emb = embedder.embed(std::slice::from_ref(&q.query)).await?;
        let q_emb = q_emb.into_iter().next().ok_or_else(|| {
            DomainError::Provider("embedder returned empty batch for query".to_string())
        })?;
        let vc = match rollup {
            Rollup::MinRank => dedup_to_note(&to_fixture(
                &vector_search(&vectors, embedder.as_ref(), &q.query, cov_k.max(k)).await?,
                &fixture_of,
            )),
            Rollup::ScoreMax => {
                let hits = vectors.query(&q_emb, RERANK_POOL).await?;
                let scored: Vec<(String, f32)> = hits.into_iter().map(|h| (h.source_id, h.distance)).collect();
                score_max_notes(&scored, &fixture_of)
            }
        };
```

Reranked (reuse `candidates` from Task 3; under ScoreMax, group the cross-encoder scores by note and take the max):

```rust
        let rr = match rollup {
            Rollup::MinRank => dedup_to_note(&to_fixture(
                &rerank(reranker.as_ref(), &q.query, &candidates, RERANK_POOL).await?,
                &fixture_of,
            )),
            Rollup::ScoreMax => {
                let scores = reranker.rerank(&q.query, &candidates.iter().map(|(_, t)| t.clone()).collect::<Vec<_>>()).await?;
                // higher score = better; convert to a "distance" (negate) so score_max_notes' min works.
                let as_dist: Vec<(String, f32)> = scores.into_iter().map(|s| (candidates[s.index].0.clone(), -s.score)).collect();
                score_max_notes(&as_dist, &fixture_of)
            }
        };
```

(`vector_search`'s internal query-embed is skipped under ScoreMax since we embed `q` once and call `vectors.query` directly. Keyword and hybrid legs stay min-rank — score-max applies only to the legs that change, per spec D4; the hybrid delta is demoted regardless.)

- [ ] **Step 5: Run the full eval tests again**

Run: `cd src-tauri && cargo test -p raki-eval`
Expected: PASS (the fake-embedder harness test exercises both legs; `run_eval` still passes `MinRank` so the gate is unaffected).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-eval/src/lib.rs
git commit -m "Add score-max rollup arm via scored ports (vector + reranked legs)"
```

---

## Task 5: synthetic chunking fixtures (D6 controls)

**Files:**
- Create: `src-tauri/crates/raki-eval/fixtures/chunking/corpus.json`
- Create: `src-tauri/crates/raki-eval/fixtures/chunking/queries.json`

- [ ] **Step 1: Author the corpus (long, multi-section, with the D6 controls)**

Create `src-tauri/crates/raki-eval/fixtures/chunking/corpus.json`. Bodies use `\n\n` paragraph breaks and `##` headings so `to_blocks` splits them. Include: a buried fact in a long note; a fact NOT cleanly paragraph-bounded; a coreference-dependent note; a 50-item list; a code-heavy note.

```json
[
  { "id": "hakone-trip", "title": "Hakone trip planning",
    "body": "## Getting there\nTake the Romancecar from Shinjuku; it is a reserved-seat limited express and well worth the small surcharge for the comfort and the view.\n\n## Ryokan\nWe booked two nights near Gora with a private onsen. Check-in is from 3pm and dinner is kaiseki at 6.\n\n## Logistics\nThe ryokan does not accept credit cards, so payment is cash on arrival — bring enough yen for the full stay plus incidentals.\n\n## Onsen etiquette\nRinse fully before entering, no swimsuits, tie long hair up, and keep the small towel out of the water." },
  { "id": "espresso-troubleshooting", "title": "Espresso troubleshooting",
    "body": "Pulling good shots is about controlling extraction.\n\nThe key symptom to learn first is taste. If the shot tastes sour and thin and runs fast, the grind is too coarse and the water rushes through without extracting; the fix is to grind finer.\n\nConversely a harsh bitter shot that drips slowly is over-extracted, so coarsen the grind. Always change one variable at a time and re-taste." },
  { "id": "launch-retro", "title": "Launch retrospective",
    "body": "## Context\nThe payments revamp shipped in May after a tight quarter.\n\n## What happened\nSarah owned the rollout. She decided to postpone it by a week when the load test surfaced a connection-pool limit. That call avoided an outage during the marketing push.\n\n## Follow-ups\nAdd a pool-size alarm and document the rollback runbook." },
  { "id": "grocery-list", "title": "Big shop list",
    "body": "Weekly groceries for the month-ahead batch cook:\n\n- milk\n- eggs\n- bread\n- butter\n- rice\n- pasta\n- olive oil\n- canned tomatoes\n- onions\n- garlic\n- carrots\n- celery\n- potatoes\n- spinach\n- frozen peas\n- chicken thighs\n- ground beef\n- salmon fillets\n- parmesan\n- yogurt\n- oats\n- bananas\n- apples\n- lemons\n- coffee beans\n- black tea\n- sugar\n- flour\n- baking soda\n- soy sauce\n- sriracha\n- peanut butter\n- honey\n- almonds\n- raisins\n- chickpeas\n- black beans\n- tortillas\n- cheddar\n- salsa\n- dish soap\n- paper towels\n- trash bags\n- foil\n- sponges\n- toothpaste\n- shampoo\n- saffron threads\n- bay leaves\n- star anise" },
  { "id": "docker-compose-notes", "title": "Docker compose for local dev",
    "body": "## Why\nA reproducible local stack without polluting the host.\n\n## Compose file\n```yaml\nservices:\n  db:\n    image: postgres:16\n    volumes:\n      - pgdata:/var/lib/postgresql/data\n    healthcheck:\n      test: [\"CMD\", \"pg_isready\"]\n  app:\n    build: .\n    depends_on:\n      db:\n        condition: service_healthy\nvolumes:\n  pgdata:\n```\n\n## Run\nBring it up detached with docker compose up -d; the named volume survives a down." },
  { "id": "marathon-plan", "title": "Marathon training block",
    "body": "## Structure\nSixteen weeks, one long run a week building about 1.5km each time.\n\n## Recovery\nEvery fourth week is a cutback week to absorb the load and avoid injury.\n\n## Intensity\nKeep roughly 80 percent of mileage easy and conversational; only 20 percent is hard intervals and tempo." },
  { "id": "tax-meeting", "title": "Notes from the accountant meeting",
    "body": "## Estimated payments\nQuarterly estimated taxes are due April 15, June 15, September 15, and January 15.\n\n## Safe harbor\nPaying 110 percent of last year's tax avoids the underpayment penalty even if this year is higher.\n\n## Depreciation\nThe new laptop is a section 179 expense this year rather than depreciated." },
  { "id": "sleep-notes", "title": "Sleeping better experiments",
    "body": "## Light\nNo screens an hour before bed; the room cool and dark.\n\n## Timing\nA consistent wake time matters more than bedtime, and caffeine has a six-hour half-life so nothing past early afternoon.\n\n## When it fails\nIf I wake at 3am and can't settle, get up and read something dull in dim light rather than lying there." }
]
```

- [ ] **Step 2: Author the queries (vague phrasing; the D6 control queries)**

Create `src-tauri/crates/raki-eval/fixtures/chunking/queries.json`. Queries are deliberately vague (no lexical echo of the buried sentence), and include a coreference-dependent query and a buried list-item query.

```json
[
  { "query": "do we need to bring cash anywhere on the hakone trip", "relevant_ids": ["hakone-trip"], "primary": "hakone-trip", "category": "buried-fact-long-note" },
  { "query": "why does my coffee taste sharp and watery", "relevant_ids": ["espresso-troubleshooting"], "primary": "espresso-troubleshooting", "category": "non-paragraph-bounded" },
  { "query": "who pushed back the release and why", "relevant_ids": ["launch-retro"], "primary": "launch-retro", "category": "coreference" },
  { "query": "did I put saffron on the shopping list", "relevant_ids": ["grocery-list"], "primary": "grocery-list", "category": "buried-list-item" },
  { "query": "how do I get the database to be ready before the app starts locally", "relevant_ids": ["docker-compose-notes"], "primary": "docker-compose-notes", "category": "code-heavy" },
  { "query": "how hard should my easy runs be while marathon training", "relevant_ids": ["marathon-plan"], "primary": "marathon-plan", "category": "buried-fact-long-note" },
  { "query": "when is the september estimated tax payment", "relevant_ids": ["tax-meeting"], "primary": "tax-meeting", "category": "buried-fact-long-note" },
  { "query": "what should I do when I wake up in the middle of the night", "relevant_ids": ["sleep-notes"], "primary": "sleep-notes", "category": "buried-fact-long-note" }
]
```

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/raki-eval/fixtures/chunking
git commit -m "Add synthetic chunking fixtures (buried-fact / coreference / list / code controls)"
```

---

## Task 6: `chunk-eval` binary — arms, deltas, length stratification

**Files:**
- Create: `src-tauri/crates/raki-eval/src/bin/chunk-eval.rs`
- Modify: `src-tauri/crates/raki-eval/Cargo.toml`

- [ ] **Step 1: Register the binary**

In `crates/raki-eval/Cargo.toml`, after the existing `[[bin]]` blocks, add:

```toml
[[bin]]
name = "chunk-eval"
path = "src/bin/chunk-eval.rs"
```

- [ ] **Step 2: Add a fixture loader for the chunking corpus**

In `lib.rs`, add helpers next to `load_corpus` (which reads `fixtures/corpus.json`):

```rust
/// Load the committed synthetic chunking corpus + queries (Task 5 fixtures).
pub fn load_chunking_corpus() -> Vec<CorpusNote> {
    let raw = include_str!("../fixtures/chunking/corpus.json");
    serde_json::from_str(raw).expect("chunking corpus.json parses")
}
pub fn load_chunking_queries() -> Vec<EvalQuery> {
    let raw = include_str!("../fixtures/chunking/queries.json");
    serde_json::from_str(raw).expect("chunking queries.json parses")
}
```

(Mirror the existing `load_corpus`/`load_queries` `include_str!` pattern; confirm their exact relative path and match it.)

- [ ] **Step 3: Write the binary**

Create `src-tauri/crates/raki-eval/src/bin/chunk-eval.rs`:

```rust
//! `chunk-eval`: LOCAL whole-note-vs-chunked retrieval comparison. Runs the prefix × rollup arms
//! over the committed synthetic chunking fixtures (and, when present, the real-data set), printing
//! per-method deltas stratified by note length. Vector + reranked are headlined; hybrid is demoted
//! but read as a DEPLOYMENT-RISK signal (it mirrors the first production state). Directional only —
//! see the chunking spec, Limitations.

use std::sync::Arc;

use raki_ai::{FastEmbedProvider, FastEmbedReranker};
use raki_eval::chunk::{ChunkStrategy, PrefixMode, Rollup};
use raki_eval::{load_chunking_corpus, load_chunking_queries, run_eval_over, CorpusNote, MethodScores};

const K: usize = 10;

fn stratum(body: &str) -> &'static str {
    // crude token proxy: word count. short < 200 words, long > 500.
    let w = body.split_whitespace().count();
    if w < 200 { "short" } else if w > 500 { "long" } else { "medium" }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = load_chunking_corpus();
    let queries = load_chunking_queries();
    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let reranker = Arc::new(FastEmbedReranker::try_new()?);

    let strata: Vec<(&str, usize)> = {
        let mut m = std::collections::BTreeMap::new();
        for cn in &corpus { *m.entry(stratum(&cn.body)).or_insert(0usize) += 1; }
        m.into_iter().collect()
    };
    println!("# chunk-eval (synthetic, LOCAL). k={K}  notes per stratum: {strata:?}\n");

    // Whole-note baseline (one run).
    let whole = run_eval_over(&corpus, &queries, embedder.clone(), reranker.clone(), K,
        ChunkStrategy::WholeNote, PrefixMode::Title, Rollup::MinRank).await?;

    // Chunked arms: prefix × rollup.
    let prefixes = [("bare", PrefixMode::Bare), ("title", PrefixMode::Title), ("title+head", PrefixMode::TitleHeading)];
    let rollups = [("min-rank", Rollup::MinRank), ("score-max", Rollup::ScoreMax)];

    let line = |label: &str, w: MethodScores, c: MethodScores| {
        println!("  {label:<22} whole R{:.2} M{:.2} | chunk R{:.2} M{:.2} | Δrecall {:+.3} Δmap {:+.3}",
            w.recall, w.map, c.recall, c.map, c.recall - w.recall, c.map - w.map);
    };

    for (pl, p) in prefixes {
        for (rl, r) in rollups {
            let chunked = run_eval_over(&corpus, &queries, embedder.clone(), reranker.clone(), K,
                ChunkStrategy::Blocks, p, r).await?;
            println!("## prefix={pl}  rollup={rl}");
            line("vector (headline)", whole.report.overall_vector, chunked.report.overall_vector);
            line("reranked (headline)", whole.report.overall_reranked, chunked.report.overall_reranked);
            line("hybrid (deploy-risk)", whole.report.overall_hybrid, chunked.report.overall_hybrid);
            // per-category (the buried-fact / coreference / list controls live here).
            for cat in &chunked.report.by_category {
                let wc = whole.report.by_category.iter().find(|c| c.category == cat.category);
                if let Some(wc) = wc {
                    println!("    [{}] vec Δrecall {:+.3} | rr Δrecall {:+.3}",
                        cat.category, cat.vector.recall - wc.vector.recall, cat.reranked.recall - wc.reranked.recall);
                }
            }
            println!();
        }
    }
    eprintln!("note: synthetic numbers settle DESIGN only; the verdict is the real-notes run (spec D7/D8).");
    Ok(())
}
```

- [ ] **Step 4: Verify it builds**

Run: `cd src-tauri && cargo build -p raki-eval --bin chunk-eval`
Expected: builds clean.

- [ ] **Step 5: Run it (real model) and read the deltas**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin chunk-eval`
Expected: prints the whole-vs-chunked tables across the six arms + per-category control deltas. (No assertion — this is a measurement, recorded by reading; per spec D10 a null/negative delta is a valid finding.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-eval/Cargo.toml src-tauri/crates/raki-eval/src/bin/chunk-eval.rs src-tauri/crates/raki-eval/src/lib.rs
git commit -m "Add chunk-eval binary: whole-vs-chunked arms, deltas, length strata"
```

---

## Task 7: real-data raw-markdown path + protocol extension

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/local_corpus.rs`
- Modify: `src-tauri/crates/raki-eval/src/bin/chunk-eval.rs`
- Modify: `docs/eval/real-data-protocol.md`

- [ ] **Step 1: Add a raw-markdown loader with a failing test**

The existing `load_local` collapses bodies via `to_plain_text`, destroying the paragraph structure chunking needs. Add a sibling that keeps raw markdown (frontmatter stripped). In `local_corpus.rs`, add a test first:

```rust
    #[test]
    fn load_local_raw_keeps_paragraph_structure() {
        let data = load_local_raw(&fixture_dir()).unwrap();
        let alpha = data.corpus.iter().find(|n| n.id == "alpha").unwrap();
        // raw markdown retains structure (the to_plain_text path would collapse it).
        // alpha.md is a single short note; assert frontmatter/heading handling is intact.
        assert!(alpha.body.contains("grind finer"));
    }
```

- [ ] **Step 2: Implement `load_local_raw`**

In `local_corpus.rs`, refactor `load_local` to share a core that takes a body-extraction closure, or add a parallel function. Minimal addition (keeps `load_local` untouched):

```rust
/// Like `load_local`, but each note's `body` is the RAW markdown (frontmatter stripped, NOT
/// collapsed to plain text) — required so the chunker's `to_blocks` can see paragraph/heading
/// structure. Used by `chunk-eval`; the whole-note arm then embeds raw markdown too, which keeps
/// the chunked-vs-whole comparison internally consistent.
pub fn load_local_raw(dir: &Path) -> Result<LocalData, LoadError> {
    let notes_dir = dir.join("notes");
    if !notes_dir.is_dir() {
        return Err(LoadError::Missing(notes_dir.display().to_string()));
    }
    let mut corpus = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&notes_dir)
        .map_err(LoadError::Io)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("md"))
        .collect();
    entries.sort();
    for path in entries {
        let raw = std::fs::read_to_string(&path).map_err(LoadError::Io)?;
        let id = slug(&path);
        let stripped = crate::markdown::strip_frontmatter(&raw);
        let title = crate::markdown::first_h1(stripped).unwrap_or_else(|| id.clone());
        corpus.push(CorpusNote { id, title, body: stripped.to_string() });
    }
    let queries_path = dir.join("queries.json");
    if !queries_path.is_file() {
        return Err(LoadError::Missing(queries_path.display().to_string()));
    }
    let qtext = std::fs::read_to_string(&queries_path).map_err(LoadError::Io)?;
    let queries: Vec<EvalQuery> = serde_json::from_str(&qtext).map_err(LoadError::Json)?;
    validate(&corpus, &queries)?;
    Ok(LocalData { corpus, queries })
}
```

- [ ] **Step 3: Run the loader test**

Run: `cd src-tauri && cargo test -p raki-eval --lib load_local_raw_keeps_paragraph`
Expected: PASS.

- [ ] **Step 4: Wire the real-data path into `chunk-eval` (opt-in if present)**

In `chunk-eval.rs`, after the synthetic run, attempt the real set and run the same arms stratified — but only if `eval-data/real/` exists (else print a one-line skip, no panic):

```rust
    let real_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../eval-data/real");
    match raki_eval::local_corpus::load_local_raw(&real_dir) {
        Ok(data) => {
            println!("\n# chunk-eval (REAL notes — LOCAL, never committed). k={K}");
            let rstrata: std::collections::BTreeMap<&str, usize> = {
                let mut m = std::collections::BTreeMap::new();
                for cn in &data.corpus { *m.entry(stratum(&cn.body)).or_insert(0) += 1; }
                m
            };
            println!("notes per stratum: {rstrata:?}  (promotion gate reads the LONG stratum — spec D8)");
            let whole = run_eval_over(&data.corpus, &data.queries, embedder.clone(), reranker.clone(), K,
                ChunkStrategy::WholeNote, PrefixMode::Title, Rollup::MinRank).await?;
            for (pl, p) in prefixes {
                for (rl, r) in rollups {
                    let chunked = run_eval_over(&data.corpus, &data.queries, embedder.clone(), reranker.clone(), K,
                        ChunkStrategy::Blocks, p, r).await?;
                    println!("## REAL prefix={pl} rollup={rl}");
                    line("vector (headline)", whole.report.overall_vector, chunked.report.overall_vector);
                    line("reranked (headline)", whole.report.overall_reranked, chunked.report.overall_reranked);
                    line("hybrid (deploy-risk)", whole.report.overall_hybrid, chunked.report.overall_hybrid);
                }
            }
        }
        Err(e) => eprintln!("\n(real-notes run skipped: {e})"),
    }
```

(`prefixes`/`rollups`/`line` are the same bindings from Task 6 Step 3 — they remain in scope; if the borrow checker objects to `line` being a closure reused across both runs, lift it to a `fn`.)

- [ ] **Step 5: Build + run**

Run: `cd src-tauri && cargo build -p raki-eval --bin chunk-eval && cargo run -q -p raki-eval --bin chunk-eval`
Expected: synthetic section prints; real section prints the skip line (no `eval-data/real/`), no panic.

- [ ] **Step 6: Extend the protocol doc (D7)**

Append to `docs/eval/real-data-protocol.md`:

```markdown

## Chunking measurement (added for the chunk-eval slice)
- Run `cargo run -p raki-eval --bin chunk-eval`; it reads `eval-data/real/` via the raw-markdown
  loader (preserves paragraph/heading structure for chunking) and prints whole-vs-chunked deltas.
- **Sample the messiest notes**, not just the longest: long multi-section notes, list-heavy notes,
  code-heavy notes, and mixed-language notes — these are where structural chunking and
  prefix↔tokenization interactions break. The promotion gate reads the **long-note stratum**.
- The chunked-vs-whole delta is computed within a single run over the identically-loaded set, so
  attribution is exact; cross-run drift (a living corpus) is inherent and acceptable.
```

- [ ] **Step 7: Commit**

```bash
git add src-tauri/crates/raki-eval/src/local_corpus.rs src-tauri/crates/raki-eval/src/bin/chunk-eval.rs docs/eval/real-data-protocol.md
git commit -m "chunk-eval: real-data raw-markdown path + protocol chunking section"
```

---

## Task 8: Verification + Definition of Done

- [ ] **Step 1: Full deterministic sweep (mirrors required CI)**

Run: `cd src-tauri && cargo test --workspace --exclude raki && cargo fmt --check && cargo clippy --workspace --exclude raki --all-targets -- -D warnings`
Expected: all pass, clean (the upstream sqlite-vec C `-Wunused-parameter` warnings are not clippy findings and are expected).

- [ ] **Step 2: The keyword snapshot gate still passes (refactor safety)**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate keyword_snapshot_is_deterministic`
Expected: PASS — `run_eval_over`'s new args (defaulted to `WholeNote/Title/MinRank` by `run_eval`) preserved synthetic Keyword behavior.

- [ ] **Step 3: score-max ≠ min-rank is proven**

Run: `cd src-tauri && cargo test -p raki-eval --lib score_max_orders_notes`
Expected: PASS — the two aggregation arms are genuinely different (answers the review's D4 correction).

- [ ] **Step 4: chunk-eval runs end-to-end on synthetic + skips real cleanly**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin chunk-eval`
Expected: synthetic arms + per-category control deltas print; real section prints the skip line; no panic.

- [ ] **Step 5: No private data, gate untouched**

Run (repo root): `git status --porcelain && git ls-files eval-data`
Expected: working tree clean; `git ls-files eval-data` prints nothing.

- [ ] **Step 6: DoD against the spec**

D1 (eval-only; `raki-retrieval`/`-storage`/`-domain` untouched) ✓ — grep shows no edits outside `raki-eval` + the protocol doc. D2 (structural blocks, list=one-block, heading-as-context, token-cap) ✓ Tasks 1,2. D3 (run-twice-and-diff; `chunk` arg; gate unaffected) ✓ Tasks 3,6. D4 (min-rank + score-max arms; vector/reranked headline; hybrid demoted) ✓ Tasks 3,4,6. D5 (reranker on passages; coreference control) ✓ Tasks 3,5. D6 (prefix arms; synthetic controls) ✓ Tasks 2,5. D7 (real notes decisive; messiest sampling; stratified) ✓ Tasks 6,7. D8/D9 (promotion gate / perf spike) — named as downstream gates, no code here (correct). D10 (honest reporting) ✓ Task 6. D11 (seam) ✓ (the `Chunk`-text/`dedup_to_note` shapes are source-agnostic). Limitations framing ✓.

- [ ] **Step 7: Frontend sanity**

Run (repo root): `bun run typecheck && bun run build`
Expected: green (no frontend files changed).

---

## Self-Review

**Spec coverage:** D1 → Task 1–7 confined to `raki-eval` (+protocol). D2 → Task 1 (`to_blocks`, list=one-block, heading context) + Task 2 (token-cap `cap_split`). D3 → Task 3 (`strategy`/`prefix`/`rollup` args, chunked build, `dedup_to_note`, gate green). D4 → Task 3 (min-rank) + Task 4 (`score_max_notes`, scored ports, vector/reranked headline) + Task 6 (hybrid demoted, printed as deploy-risk). D5 → Task 3 (rerank on chunk texts) + Task 5 (coreference fixture). D6 → Task 2 (prefix arms) + Task 5 (controls + vague queries). D7 → Task 7 (`load_local_raw`, real path, protocol extension) + Task 6 (stratification). D8/D9 → intentionally no code (downstream gates), surfaced in `chunk-eval` output + DoD. D10 → Task 6. D11 → Task 3/4 abstractions.

**Placeholder scan:** none — every step has complete code or an exact command. The only deliberately-empty runtime input is the user's real notes (by design; the synthetic fixtures let every task run today).

**Type/consistency:** `run_eval_over(corpus, queries, embedder, reranker, k, ChunkStrategy, PrefixMode, Rollup)` defined Task 3, called identically in Task 6/7 and `run_eval`. `chunk(title, body, ChunkStrategy, PrefixMode) -> Vec<String>` (Task 2) called in the Task 3 build. `dedup_to_note(&[String]) -> Vec<String>` (Task 3) and `score_max_notes(&[(String,f32)], &HashMap) -> Vec<String>` (Task 4) used in the per-query loop. `Block { heading: Option<String>, text: String }` + `to_blocks` (Task 1) consumed by `chunk` (Task 2). `VectorHit{source_id,distance}` / `RerankScore{index,score}` match `ports.rs`. `MethodScores`/`Report.overall_*`/`by_category` match `lib.rs`. Fixture slugs (the `id` field) are the ids `relevant_ids` reference — consistent between Task 5 fixtures and the loader.

**Known approximation (by design, per spec):** the token cap is char-based (not the real bge tokenizer); score-max applies to the vector + reranked legs only (keyword/hybrid stay min-rank); the whole-note arm on the real-data path embeds raw markdown (internally consistent within the comparison).

---

## Execution Handoff

(Presented to the user after saving.)
