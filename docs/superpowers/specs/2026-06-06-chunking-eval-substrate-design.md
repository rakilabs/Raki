# Chunk-Level Retrieval Eval Substrate — Design

Date: 2026-06-06

Status: **Approved after adversarial revision (2026-06-06).** An adversarial review landed six
real corrections (two were factual errors in v1: see D4 and D11) and forced a reframing: chunking
is treated as an architecturally inevitable commitment, and this slice's job is to **settle the
chunking design cheaply and gate the eventual storage migration** on quality + performance,
decided on **real** notes, behind a **time-bound promotion criterion** — not to relitigate whether
to chunk. The v1 framing ("eval might show we shouldn't chunk") was wrong: whole-note embedding
fails the buried-fact case by construction. What is genuinely open is *which* chunking design and
*whether it clears the quality and performance bars before* the schema becomes load-bearing.

## Premise (why this is not "should we chunk?")

A single vector for a long note is a centroid that averages away outlier semantics: a 50-token
fact inside a 2,000-token note is drowned by the other 1,950 tokens, so a query for that fact
lands far from the note's vector. This is a mathematical property, not a hypothesis. Chunking is
therefore almost certainly required for Raki's memory layer. The open questions this slice
answers, in order: (1) which **chunk unit / prefix / aggregation** design retrieves best, (2)
whether it lifts retrieval on the **user's real notes**, (3) whether it clears the **local-first
performance budget** at real chunk volumes. Only when all three pass does the storage migration
fire — and it fires on a **time-bound** schedule so eval-first cannot rot into eval-forever.

## What this is

A second retrieval *granularity* in the eval. Each corpus note is split into **structural blocks**;
every block is embedded as its own vector; chunk hits are rolled up to the note; and the **same
scoring pipeline runs twice — whole-note vs chunked — over the same fixtures and over the user's
real notes**. The deltas (and two design knobs run as measured arms) are the output. Everything
lives in `raki-eval`; `raki-retrieval`, `raki-storage`, and `raki-domain` are untouched.

## What this is NOT

- **Not a production change (yet).** `search_notes`, the `note_vectors` schema (one vector per
  note), `list_pending`, and `hybrid_search` are unchanged in this slice. The migration is gated
  by D8/D9 and happens in a **separate** slice once quality and performance pass.
- **Not ProseMirror block IDs.** ADR-0004 names stable block IDs as the eventual unit, but the
  TipTap↔storage wiring does not exist yet (and is not scheduled — ADR-0004 lists it as a follow-up),
  so block IDs are not on stored content. Markdown structural blocks are the **proxy** and, per D8,
  the migration unit; block-ID granularity is a **separate future slice** once that wiring is built.
- **Not a verdict that "chunking must win."** A null/negative delta is a valid recorded finding
  (D10). The competing hypothesis — that chunking *hurts* the reranker by stripping coreference
  context — is explicitly tested (D5), not assumed away.
- **Not a performance benchmark by itself.** Retrieval quality and systems performance are
  **separate questions**; the quality eval says nothing about sqlite-vec latency or WAL contention.
  Those are a co-equal required gate (D9), run *after* the design arms settle.

## Decisions

- **D1 — Scope: eval-substrate first.** Chunking is added to `raki-eval` only. The retrieval
  functions operate on whatever `VectorIndex` + id-space they are handed, so chunking needs **no
  changes** to `raki-retrieval`/`raki-storage`/`raki-domain`. The risk surface is the eval's index
  build + a rollup step + the `chunk-eval` binary.

- **D2 — Strategy: structural blocks, list-as-one-block, heading-as-context.** Split the markdown
  body via the `pulldown-cmark` events the eval already parses. A **content block** is: a
  paragraph, a **whole list** (all its items joined — *not* one chunk per item, which would
  explode a 50-item list into 50 trivial chunks), or a code-block. **Token-cap (required
  correctness, not tuning):** any block exceeding the embedding model's input limit (~512 tokens for
  bge-small-en-v1.5) is **hard-split at the cap into sequential chunks**, never silently truncated —
  otherwise item 47 of a 50-item list, or the bottom of a 300-line YAML block, becomes invisible to
  vector search. `chunk-eval` surfaces how many blocks were split. (Overlap and semantic-aware
  splitting stay deferred size-targeting; the cap is the floor needed to avoid a silent-truncation
  *bug*, distinct from quality *tuning*.) A **heading is not a standalone chunk**; its text is folded
  into the blocks beneath it as section context. **Known limitation (measured, not assumed away):**
  markdown boundaries are *visual*, not semantic — a thought spanning two paragraphs is bisected,
  and a qualifier in the next block is isolated. The coreference/cross-block fixtures (D6) are
  designed to surface this; if it bites, semantic chunking is a future option, not this slice.

- **D3 — Run-twice-and-diff, not new Method variants.** `run_eval_over` gains a
  `chunk: ChunkStrategy` argument. The `chunk-eval` binary calls it per configuration over the same
  data and diffs the `Report`s. The per-query scoring loop and `Method` enum are unchanged.
  `run_eval` passes `WholeNote`, so the **keyword snapshot gate is unaffected** (the refactor guard).

- **D4 — Aggregation is a MEASURED ARM, and the v1 "max-pool" claim is RETRACTED.** First-occurrence
  in a rank-ordered list is **min-rank pooling** (a note's best-*ranked* chunk), which is **not** the
  same as max-pooling (a note's best-*scored* chunk). They coincide only for a pure vector search
  (where rank is sorted by score) and **diverge after reranking**, where a spurious high-ranked
  chunk can outrank the real one (a risk amplified by prefix domination — D-arm below). So we run
  **two aggregation arms** and report both: (a) **min-rank** (free, from the ordered id list); (b)
  **score-max** — the eval calls the **scored ports directly** (`VectorIndex::query → VectorHit`
  distances, `Reranker::rerank → RerankScore`), because the id-only wrappers `vector_search`/`rerank`
  **discard scores**. This is an **additional** path in `chunk-eval`, not a change to the existing
  id-list scoring loop (which stays for min-rank and keeps the keyword snapshot gate intact). The
  headline compares the **vector** and **reranked** legs (the legs that actually change); the
  **hybrid** delta is reported but **demoted** for attribution — chunked-vector + whole-note-keyword
  is a granularity mismatch whose combined effect is not cleanly isolable. (The review's RRF-based
  "fusion is poisoned" argument does **not** apply: per ADR-0006, `hybrid_search` is vector-primary +
  keyword backfill, not RRF.) **But the demoted hybrid delta is still read as a deployment-risk
  signal:** chunk-granular vector + note-granular keyword is *exactly the first production state* of a
  partial migration, so a **negative hybrid delta is deployment-blocking even when vector/reranked are
  positive** — it is the realistic user-facing config, not an artifact.

- **D5 — Reranker scores passages, and the counter-hypothesis is tested.** Under `Blocks`, the
  rerank pool returns chunk ids and `text_of` maps each to its **chunk text**, so the cross-encoder
  judges *(query, passage)* pairs. **But chunking may HURT the reranker** by stripping the
  coreference/pronoun context whole notes provided ("he postponed the launch" is unrankable in
  isolation). The fixtures (D6) include **coreference-dependent queries** specifically so the eval
  can falsify "chunking helps reranking," not just confirm it. The `reranked-chunked − reranked-
  whole` delta is symmetric: a negative result is recorded plainly.

- **D6 — Prefix is a MEASURED ARM, and synthetic fixtures settle DESIGN only.** Title/heading
  prefixing is **not** a fiat decision; the review correctly flagged that prefixing every chunk
  pulls a note's chunks together by shared prefix and can let the prefix dominate a short chunk's
  embedding, eroding cross-note discrimination. So prefixing is run as **three arms** — `bare`,
  `title`, `title+heading` — and reported. The committed synthetic set `fixtures/chunking/
  {corpus.json, queries.json}` (~8–12 notes) settles the design arms only and includes deliberate
  **controls**: a buried fact that is *not* cleanly paragraph-bounded; a coreference-dependent
  query; a 50-item list; a code-heavy note; vague "future-self" query phrasing (not lexically
  precise echoes of the fact). Synthetic numbers are **design-settling**, never the verdict.

- **D7 — REAL notes are the decisive measurement.** The chunked-vs-whole comparison is run over the
  user's real notes via the real-data substrate. The protocol (`docs/eval/real-data-protocol.md`)
  **already exists** (privacy, gitignored `eval-data/`, slug ids, labeling discipline); this slice
  **extends it** with two additions: (1) a **sampling of the messiest notes** (long, list-heavy,
  code-heavy, mixed-language — where structural chunking and prefix↔tokenization interactions are
  most likely to break), not just the longest; and (2) **length stratification**. Results are
  reported **per length stratum (short < ~200 tokens / medium / long)** because chunking is expected
  to help long notes and may *degrade* short ones (boundary errors + prefix noise on atomic
  thoughts) — an unstratified mean could bury a real long-note win under short-note degradation.
  **Reproducibility:** the chunked-vs-whole delta is computed **within a single run over the
  identically-loaded note set**, so attribution is exact; cross-run drift (a living corpus) is
  inherent and acceptable for a directional gate. **Realism bar (ADR-0006):** this real set must be
  broad and noisy enough that whole-note vector *fails somewhere* — a thin or saturated set makes the
  D8 threshold measure noise, so satisfying ADR-0006's realism prerequisite is a gate-reading
  precondition (see D8), not an afterthought. Synthetic settles *how* to chunk; real notes decide
  *whether it helps the user*.

- **D8 — Promotion gate (D-PROMOTE): a time-bound trigger, not a treadmill.** State the criterion up
  front so discipline converts into a decision:
  > **Precondition (ADR-0006 realism — read this first).** The gate is only *decidable* once the
  > real-notes set (D7) is realistic enough that whole-note vector retrieval **demonstrably fails on
  > some long-note queries**, *and* the long stratum holds enough queries that **+0.05 clears
  > run-to-run noise**. If the substrate is thin or saturated, the verdict is **"not yet decidable —
  > grow the corpus first,"** never "chunking failed." Growing the substrate to that bar (ADR-0006's
  > stated first dependency of every retrieval improvement) is the real-data slice's job and a
  > prerequisite to reading this gate.
  >
  > **Trigger.** Migrate production storage to chunk granularity **iff** the **best chunked
  > configuration that clears the performance bar (D9)** beats whole-note by **≥ +0.05 Success@3 on
  > the long-note stratum (and not worse on MRR overall)** on the real notes. *The best config that
  > clears both bars* — not "the quality winner, hoping it's fast enough"; D9 tests the top configs
  > jointly so this is a real choice.
  >
  > **Unit + date (branch collapsed).** The editor↔storage block-ID wiring is **not scheduled** —
  > ADR-0004 lists it as a round-trip-fidelity follow-up, not a committed slice. So the migration unit
  > is **markdown-block granularity, on 2026-09-06.** The block-ID migration is a **separate future
  > slice**, to run when that wiring is built — a bounded, known second migration, accepted as the
  > cost of not staying on whole-note retrieval indefinitely.
  (The +0.05 threshold and the 2026-09-06 date are the team's to confirm — written concretely so the
  gate is actionable. +0.05 on the long stratum is a defensible minimum detectable effect for a
  small-N personal corpus; 2026-09-06 is roughly one vertical-slice cycle.)

- **D9 — Performance/scale spike: a co-equal required gate, sequenced AFTER the design arms.** A
  separate spike (its own slice) stress-tests the **real storage adapter** at realistic chunk volumes
  (e.g. 10k notes × ~8 blocks = ~80k vectors): sqlite-vec exact-search p95 latency, WAL writer
  serialization / lock contention when chunk upserts share the UI save path, and `list_pending` queue
  semantics flipping from note-grained to chunk-grained. Run on the **top 2–3 quality configurations
  (the Pareto frontier), not just the single quality winner** — quality and perf trade off (e.g.
  `title+heading` may win Success@3 but produce longer, slower-to-embed, latency-bloating chunks
  while `bare` scores nearly as well far more cheaply). The promotion gate (D8) then picks the **best
  config that clears both bars.** Perf tells us *how* to chunk (and whether we need quantization /
  ANN / candidate pre-filtering), not *whether*. A perf failure changes the implementation, not the
  decision.

- **D10 — Honest reporting.** `chunk-eval` prints per-method tables and the `chunked − whole` deltas
  (vector and reranked headlined; hybrid demoted), across both aggregation arms and all three prefix
  arms, with the buried-fact and coreference categories called out. The recorded finding states the
  delta plainly — lift, null, or regression.

- **D11 — Forward seam: harness reused, measurement re-run (v1 "no metric change" RETRACTED).** The
  abstraction `Chunk { id, text }` + the `dedup_to_note` rollup + the scoring pipeline are reused
  unchanged when real block IDs land — that decoupling is genuine. But markdown-block chunks ≠
  ProseMirror-block chunks (Enter-for-spacing, images, horizontal rules, nested task lists
  pulldown-cmark never emits), so the **measurement must be re-run** on real blocks; the seam saves
  the harness, not the result. D8's time-bound unit rule exists precisely because of this.

## Architecture / data flow

```
CorpusNote {id, title, body}
   │  chunk(note, ChunkStrategy)        prefix arm ∈ {bare, title, title+heading}
   ▼
WholeNote → [ "{title}\n\n{body}" ]                      (1 chunk; today's behavior, byte-identical)
Blocks    → [ <para>, <whole list>, <code block>, … ]    (to_blocks(body); heading folded as context)
   │  index build: vectors.upsert("slug#i", embed(prefix + chunk)); fixture_of["uuid#i"]=slug
   ▼
per query:  vector_search / hybrid_candidates / rerank → ordered chunk hits (+ scores, for score-max)
   │  to_fixture → note slugs (repeats under Blocks)
   │  rollup:  min-rank  OR  score-max   ← measured arm
   ▼
note-level ranking → existing score_one / metrics (unchanged)
   │  run per {strategy × prefix-arm × rollup-arm}  → Reports → chunked − whole deltas
   ▼  on synthetic fixtures (settle design) AND on real notes (decide promotion, D7/D8)
```

## Components touched

- `crates/raki-eval/src/markdown.rs` — MODIFY: add `to_blocks(md) -> Vec<String>` (block-preserving;
  list joined to one block; heading folded as context). `to_plain_text`/`first_h1` unchanged.
- `crates/raki-eval/src/chunk.rs` — CREATE: `ChunkStrategy { WholeNote, Blocks }`,
  `PrefixMode { Bare, Title, TitleHeading }`, `Chunk { id, text }`,
  `chunk(note, strategy, prefix) -> Vec<Chunk>`; applies the **token-cap split** (D2; approximate
  via a char/word heuristic to avoid tokenizer coupling) and reports a split count.
- `crates/raki-eval/src/lib.rs` — MODIFY: `run_eval_over(..., chunk, prefix, rollup)`; chunk-keyed
  index build; **min-rank** `dedup_to_note` from the id list **plus** a **score-max** rollup that
  calls `VectorIndex::query` / `Reranker::rerank` directly for raw scores (the id-only wrappers
  discard them) — an additional path; the existing id-list loop is unchanged; `run_eval` passes
  `WholeNote` (gate unaffected); `pub mod chunk;`.
- `crates/raki-eval/src/bin/chunk-eval.rs` — CREATE: run the arms over synthetic fixtures and over
  the real-data set; print tables + deltas (vector/reranked headlined, hybrid demoted but read as a
  **deployment-risk** signal), **stratified by note length**; surface split-block counts. `[[bin]]`.
- `crates/raki-eval/fixtures/chunking/{corpus.json, queries.json}` — CREATE: committed synthetic
  design-settling set with the D6 controls (non-paragraph-bounded fact, coreference query, 50-item
  list, code-heavy note, vague phrasing).
- `docs/eval/real-data-protocol.md` — MODIFY: add the messiest-notes sampling guidance + the
  length-stratification note (D7). (The protocol already exists; this slice extends it.)
- `crates/raki-eval/Cargo.toml` — MODIFY: add the `[[bin]]`.

**Integration note (must not be missed in the plan):** `to_blocks` needs the **raw markdown**, but
the real-data loader (`local_corpus`) currently stores `body = to_plain_text(raw)`, which *collapses*
the paragraph boundaries chunking splits on. So for the real-data path the chunker must receive the
**raw markdown** (either `local_corpus` retains it alongside the plain-text body, or `chunk-eval`
re-reads the source files). The synthetic chunking fixtures sidestep this by carrying
structure-preserving bodies (`\n\n` paragraph breaks) in their JSON.

The performance spike (D9) and the storage migration (D8 trigger) are **separate future slices**,
named here only as the gates this slice feeds.

## Testing & verification

- **`to_blocks` fidelity**: multi-paragraph + heading + fenced-code + list markdown yields the
  expected block count; a list is **one** block; code contents intact; frontmatter stripped; no
  fence/HTML leakage.
- **chunker**: `WholeNote` returns one chunk byte-identical to `"{title}\n\n{body}"`; `Blocks` count
  is correct; each prefix arm produces the expected leading text.
- **rollups**: `min-rank` collapses repeated slugs to first-occurrence order (no-op under `WholeNote`);
  `score-max` orders notes by their best constituent score and **diverges from min-rank** on a
  constructed case (the spurious-high-rank chunk) — proving the two arms are genuinely different.
- **Refactor guard**: `keyword_snapshot_is_deterministic` stays green.
- Full `cargo test --workspace --exclude raki` / `cargo fmt --check` / `cargo clippy … -D warnings`
  green; frontend untouched (`bun run typecheck && bun run build`).
- **Manual** (real model): `cargo run -p raki-eval --bin chunk-eval` prints synthetic + real deltas.

## Consequences

- Retrieval *granularity*, *prefixing*, and *aggregation* become measured variables; the buried-fact
  tripwire (ADR-0006) and the coreference counter-hypothesis are both exercised.
- The storage migration is **pre-scoped and de-risked** (the chunk-keyed index + rollup are the exact
  shapes production needs) but **not paid for** until quality (D7/D8) and performance (D9) pass — on
  a bounded timeline (D8) that prevents indefinite deferral.
- The reranker's keep/kill case gets its first fair test: a granularity where deep recall pools
  actually contain something to rescue — or a measured demonstration that fragments hurt it.

## Limitations

- **Markdown boundaries are syntactic, not semantic** (D2) — cross-block dependencies can be severed;
  measured via the D6 controls, not solved here.
- **Prefix domination / intra-note clustering** (D6) — mitigated by running prefix as an arm; the
  winning arm may still trade cross-note discrimination for intra-note context.
- **Min-rank can be fooled in the tail** (D4) — hence score-max is run alongside it.
- **Short notes may degrade under chunking** — boundary errors + prefix noise on atomic thoughts;
  measured via D7 length stratification so it cannot silently swamp the long-note win (the promotion
  threshold reads the long stratum).
- **Token-cap split is approximate** (D2) — a char/word heuristic, not the real bge tokenizer, so a
  block near the limit may split a few tokens early/late; acceptable for avoiding silent truncation.
- **Synthetic is still synthetic** — design-settling only; the real-notes run (D7) is the verdict, and
  even it carries authorship bias on the query side (the substrate's protocol limits, not eliminates).
- **Messy real notes** (huge code blocks, mixed-language, prefix↔tokenization interactions) may break
  structural chunking; D7 samples them deliberately so the failure is *seen*, and D9 catches the
  vector-count blowup they cause.

## Non-goals (each its own later slice)

- The production storage migration (D8 trigger) and the performance/scale spike (D9).
- ProseMirror block IDs as the real unit (the editor↔storage wiring; D8's time-bound branch).
- Chunk-size merging/splitting/overlap and oversized-code-block splitting (size-targeting).
- Keyword-leg chunking; semantic (embedding-boundary or LLM) chunking; structure/recency; generate stage.
