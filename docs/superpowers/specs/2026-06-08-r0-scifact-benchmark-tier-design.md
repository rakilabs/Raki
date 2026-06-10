# R0 — SciFact Benchmark Measurement Tier Design

**Goal:** Add a manual measurement tier to `raki-eval` that scores the real retrieval methods
(keyword / vector / hybrid / reranked) over the full BEIR **SciFact** corpus with aggregate IR
metrics — giving Raki, for the first time, a corpus where the bi-encoder *genuinely fails*
(nDCG@10 ≈ 0.65, not the toy set's 1.0). This unblocks every downstream retrieval lever (R1
reranker decision, R2 chunking, R3 query understanding) by making lift **provable** instead of
faith-based.

**Roadmap:** `docs/ROADMAP.md` Track A, milestone **R0** (benchmark-first). **ADR-0007**
(measurement-gated retrieval) is the governing decision; this spec un-shelves and updates
`2026-06-06-scifact-measurement-tier-design.md` for the evolved codebase.

**Tech Stack:** Rust, `raki-eval`, `fastembed` (bge-small via `FastEmbedProvider`), the existing
`SqliteNoteRepository`/FTS5/`SqliteVectorIndex` in-memory index, `ureq` + `zip` (download), `serde_json`.

---

## The honesty clause (why this tier, and its limits)

SciFact is **scientific-claim retrieval** — technical vocabulary, sentence-length queries,
citation structure — **not** Raki's distribution (personal notes: fragmented, temporal, vague,
self-referential). Its absolute nDCG@10 and the `reranked − hybrid` delta are therefore
**directional engineering evidence, not a faithful measure of Raki's retrieval quality.** This
was the exact reason the original SciFact tier was shelved; ADR-0007 accepts that cost because:

- The 30-note synthetic tier is **saturated** (vector recall ≈ 1.00), so no lever can show lift.
- A real, labeled, *hard* corpus is the fastest way to make rerank/chunking measurable **now**,
  without waiting on private real-notes ground truth.
- The **real-data tier remains the faithful judge.** The reranker's *final* attach/delete verdict
  is still taken on real ground truth per its kill-switch (`docs/eval/reranker-deletion-criteria.md`).
  This tier provides the *directional* signal and *pipeline calibration*, not the final word.

---

## What this is / is NOT

**Is:** a manual measurement tier; aggregate IR metrics (nDCG@10 / Recall@10 / MAP) over the full
SciFact corpus; runs `#[ignore]`/on-demand like the existing real-model gate; sits **beside** the
30-note tier; headline = `reranked − hybrid` nDCG@10 delta + a vector-sanity calibration number.

**Is NOT:**
- **Not a replacement** for the 30-note tier — that stays the fast, deterministic, required-CI
  **regression** net (per-query keyword snapshots). This tier *measures absolute quality*; it does
  not snapshot per-query rankings and is **not** a required CI job.
- **Not a production change** — `search_notes`, `raki-storage`, `raki-retrieval`, and the 30-note
  `run_eval`/snapshot/gate machinery are untouched.
- **Not the reranker decision** — R0 produces the signal; **R1** reads it and decides attach/delete.
- **Not deterministic/committed data** — the corpus is downloaded (network, large); no committed
  per-query snapshots, only a recorded author-once measurement + a coarse sanity floor.

---

## Run cadence (review M3)

Solo project — no role ceremony. The `bench --write` binary is run **before R1 planning** and
**whenever the embedding/retrieval pipeline materially changes** (new embedder, chunking in R2,
reranker config). The regenerated `docs/eval/scifact-baseline.md` is committed **with the slice
that changed the pipeline** (so the recorded numbers always match the code that produced them).
The deterministic 30-note tier remains the per-commit required gate; this tier is on-demand.

## D1 — Dataset: BEIR SciFact, downloaded on demand (dataset-agnostic loader)

BEIR SciFact: ~5,183 docs, 300 test queries, graded qrels. Fetched from the BEIR public zip and
cached under a **gitignored** `.beir_cache/scifact/`. Chosen for being small, clean, and
well-known (published bge-small nDCG@10 ≈ 0.65 — far below the 30-note ≈ 1.0, so it breaks the
ceiling) and ideal for **pipeline-sanity calibration** against a trusted baseline.

The loader is **dataset-agnostic** (BEIR layout: `corpus.jsonl {_id,title,text}`,
`queries.jsonl {_id,text}`, `qrels/test.tsv` with `query-id  corpus-id  score`): swapping to a
harder set (e.g. NFCorpus, for more reranker headroom in R1) is a one-line dataset descriptor
change.

**License:** SciFact is CC BY-NC 2.0. We **download at test time and never ship/redistribute** it
(research/measurement use). The NC clause is a flag only if Raki ever bundles data commercially
(we don't). Attribution recorded in `scifact-baseline.md`.

## D2 — Aggregate IR metrics over graded qrels, k = 10

Score **nDCG@10** (primary, BEIR standard), **Recall@10**, and **MAP**, meaned over the 300 test
queries, per method (keyword / vector / hybrid / reranked). k = 10 (IR convention + comparability
with published baselines; the 30-note smoke tier stays at k = 3).

**Graded relevance — reuse the existing primitive (review M1/C2).** The existing
`raki_retrieval::ndcg_at_k(ranked, grades: &HashMap<String, f64>, k)` is **already graded**: it
takes a grades map, computes graded DCG / ideal-DCG, and returns `None` when grades are empty or
ideal-DCG is 0. The BEIR qrels map (`doc_id → score`) is *exactly* its signature, so the benchmark
**reuses it directly — no new primitive, and `raki-retrieval` is not modified.** Recall@10 and MAP
treat any qrel `score > 0` as relevant (binary), per BEIR convention (the existing `recall_at_k` /
`average_precision_at_k` take a `HashSet` of relevant ids and likewise return `None` when empty).

**Zero-relevance queries (review M6).** All three primitives already return `None` when a query has
no relevant docs (empty set / zero ideal-DCG), so there is no NaN. The benchmark aggregation **means
over the `Some(_)` values, skipping `None`** (TREC convention), per method. A future dataset with
zero-relevance queries is therefore handled; a unit test covers it (D8).

Snapshotting 300 queries is the wrong question (this tier answers "how good, absolutely," not "did
a ranking silently change") and is omitted.

## D3 — Download-on-demand, cached, `#[ignore]`/manual

Mirror the existing real-model gate (which already downloads the bge model). **No network in the
required CI path.** New `raki-eval` deps: `ureq` (blocking HTTP) + `zip` — small, common. A missing
cache + no network exits cleanly with onboarding guidance (like `real-eval`'s missing-data path),
never a panic.

**Cache integrity (review M5) — no silent reuse of a corrupt cache.** Download + unzip to a temp
dir `.beir_cache/scifact.tmp.<pid>/`, then **validate** (all three expected files present; each
JSONL line parses; qrels TSV parses) and only on success **atomically rename** to
`.beir_cache/scifact/`. A run reuses `.beir_cache/scifact/` only if it exists *and* re-validates;
on any parse/validation failure it deletes the bad dir and fails with a clear "corrupt cache —
deleted; rerun to re-download" message. An interrupted download leaves only the temp dir, never a
half-populated canonical dir.

## D4 — Real retrieval over the full corpus, reusing existing machinery

`run_benchmark(embedder: &dyn EmbeddingProvider, reranker: &dyn Reranker, k: usize) -> BenchReport`:

1. Build a real in-memory index via a **shared helper (review M4 — no duplication):** extract
   `build_in_memory_index(notes: &[CorpusNote]) -> Result<(Database, SqliteNoteRepository,
   SqliteKeywordIndex, SqliteVectorIndex), _>` in `raki-eval` (the `Database::open_in_memory()` +
   `register_sqlite_vec()` + repository/index construction + upsert loop currently inlined in
   `run_eval_over`). **Both `run_eval_over` and `run_benchmark` call it** — one source of truth for
   index construction (AGENTS.md §5/§9). `run_eval_over`'s change is a behavior-preserving refactor
   guarded by the existing 30-note tests + the deterministic snapshot.
2. Embed all ~5K docs with bge-small (documents via `embed`; queries via `embed_query`).
3. Per test query: run the **production** retrieval functions (`search`, `vector_search`,
   `hybrid_search`/`hybrid_candidates`) and `rerank` over the recall pool. Reranking reorders the
   top-N recall union (N = 100, IR convention; tunable down for speed).
4. Recall is measured against the **full** corpus (all docs indexed) so the metric is meaningful.
5. Aggregate per method → `BenchReport { per_method: [MethodAgg; 4], reranked_minus_hybrid_ndcg,
   vector_ndcg }`.

Reuses the `Method` enum and the metric primitives; new code is the BEIR loader + graded-qrels
aggregate scoring.

## D5 — Headline outputs

(a) **`reranked − hybrid` nDCG@10 delta** — whether the cross-encoder rescues relevant docs when
vector fails (the directional R1 / D-DELETE signal). **Interpretation (review C1):** `delta > 0`
favors the *attach* hypothesis (the reranker helps where vector fails) — to then **confirm on real
data** per the kill-switch; `delta ≤ 0` is directional evidence toward the *deletion* path. This is
deliberately **not** a SciFact go/no-go threshold: the binding `+0.03 nDCG` criterion is on
**real-notes ground truth** (`docs/eval/reranker-deletion-criteria.md`), because SciFact is
domain-shifted. R0's job is to produce a real, recorded delta on a corpus where vector fails; R1
acts on it (attach-to-validate, or delete) either way. (b) **Pipeline-sanity:** vector nDCG@10 vs the
published ≈ 0.65 — a wildly-off number means our wiring is broken, not the model.

## D6 — Posture: recorded measurement + coarse sanity floor (not a regression gate)

- **`bench` binary** (`src/bin/bench.rs`): prints the 4-method table (nDCG@10 / Recall@10 / MAP),
  the `reranked − hybrid` delta, and the vector-sanity line. Persists the record to
  `docs/eval/scifact-baseline.md` (table + deltas + date + platform + model ids + attribution)
  **only when passed `--write`** (review m1 — matches the `eval-report` binary's existing safety
  gate); default is stdout-only, so an accidental cross-platform run can't dirty the tree.
- **`benchmark_gate`** (`tests/benchmark_gate.rs`, `#[ignore]`): runs the full tier and asserts
  **coarse sanity floors** (not quality-regression gates; no corpus tuning):
  - **Vector calibration (review M2):** vector nDCG@10 **≥ 0.55** (committed value, ~0.10 below the
    published ≈ 0.65; hard-coded in the test) — catches a broken bi-encoder/index wiring.
  - **Reranker plausibility (review M7):** the reranker path must not error/panic (a failure fails
    the gate, never a silent fallback); `reranked − hybrid` nDCG@10 is **finite and within
    [−0.10, +0.20]**, and `reranked` nDCG@10 ≥ `0.5 × hybrid` — so a garbage/misconfigured
    cross-encoder (random scores) can't pass and feed R1 a false delta.

## D7 — Components touched

- `src-tauri/crates/raki-eval/src/benchmark.rs` — **CREATE**: BEIR loader (download+parse, with
  cache-integrity per D3) + `run_benchmark` + `BenchReport`/`MethodAgg`.
- `src-tauri/crates/raki-eval/src/lib.rs` — **MODIFY**: extract `build_in_memory_index` (D4) and
  route `run_eval_over` through it (behavior-preserving).
- `src-tauri/crates/raki-eval/src/bin/bench.rs` — **CREATE**: the report binary (`--write` gated).
- `src-tauri/crates/raki-eval/tests/benchmark_gate.rs` — **CREATE**: `#[ignore]` sanity floors.
- `src-tauri/crates/raki-eval/Cargo.toml` — **MODIFY**: add `ureq`, `zip`.
- `.gitignore` — **MODIFY**: add `.beir_cache/`.
- `docs/eval/scifact-baseline.md` — **CREATE**: recorded measurement + attribution.
- `docs/ROADMAP.md` — **MODIFY**: flip R0 status + link this spec/plan.

**`raki-retrieval` reuses the existing graded `ndcg_at_k` unchanged.** All production crates
(`raki-domain`, `raki-storage`, `raki-retrieval`, `raki-ai`, the app) and the 30-note tier's
**behavior** are untouched (`run_eval_over`'s refactor is internal + behavior-preserving).

## D8 — Testing & verification

- **Loader parse test** (no network): inline corpus/queries JSONL + qrels TSV strings parse into
  the expected structs.
- **Cache-integrity test** (no network, review M5): a corrupt/partial cache dir is rejected
  (validation fails → deleted → clear error), and a temp dir is never promoted on failure.
- **Aggregation unit test**: hand-built per-query rankings + a tiny graded qrels map → expected
  nDCG@10 / Recall@10 / MAP means (reuse the existing primitives). **Includes a zero-relevance
  query (review M6):** the primitives return `None`, and the mean **skips it** rather than
  producing NaN.
- **`benchmark_gate`** (`#[ignore]`, real model + download): runs the tier, asserts the vector
  nDCG@10 ≥ 0.55 floor **and** the reranker-plausibility band (review M7); records nothing (the
  `bench --write` binary produces the measurement record).
- Required CI path unchanged: `cargo test --workspace --exclude raki` / `cargo fmt --check` /
  `cargo clippy --workspace --exclude raki --all-targets -- -D warnings` green; no new model/network
  in the required path. Frontend untouched.

---

## Definition of Done

- New deterministic tests (loader parse, cache-integrity, aggregation incl. zero-relevance) pass in
  the required CI path.
- `cargo run -p raki-eval --bin bench -- --write` (with model + network) prints the 4-method table,
  the `reranked − hybrid` nDCG@10 delta, and the vector-sanity line; writes `docs/eval/scifact-baseline.md`.
- The recorded vector nDCG@10 is **≥ 0.55** (calibrated to the published ≈ 0.65).
- `benchmark_gate` passes with `--ignored` (vector floor **and** reranker-plausibility band).
- `docs/ROADMAP.md` R0 flipped to ✅ with the baseline recorded. **R1 (reranker decision) is now
  unblocked** — meaning a corpus where vector *fails* and a recorded `reranked − hybrid` delta now
  exist for R1 to reason about (`delta > 0` ⇒ attach-to-validate-on-real-data; `delta ≤ 0` ⇒
  directional toward deletion). R1 proceeds either way; the binding verdict stays on real data.

## Out of scope (each its own later slice)

R1 reranker attach/delete decision · R2 chunk-level embeddings · R3 generate-stage query
understanding · the note-import command · any `search_notes`/production change · per-query
snapshotting or required-CI gating of SciFact · a second dataset (NFCorpus is a one-line add when R1
wants more headroom).
