# SciFact Measurement Tier (Eval) — Design

Date: 2026-06-06

Status: **SHELVED (2026-06-06), before implementation.** An adversarial review established
that SciFact measures a *different retrieval problem* (scientific-claim retrieval — technical
vocabulary, sentence-length queries, citation structure) than Raki's (personal notes —
fragmented, temporal, vague, self-referential). Its absolute nDCG@10 and the `reranked − hybrid`
delta therefore have no validated correlation with Raki's retrieval quality, and the one
defensible value — external *pipeline calibration* against a published baseline — is a cheap
**run-once** check (best done when the embedding pipeline materially changes, e.g. chunking),
not a maintained download-on-demand tier worth its bitrot, license, and cognitive cost.

**Superseded by a pivot to real-data eval:** a minimal programmatic note-import command +
dogfooded personal notes (~50–200) + a small real query set authored under the 3a protocol —
an eval substrate that reflects Raki's actual data model and failure topology. The design below
is retained for the record (and as the reference for any future run-once external calibration).

---

The original (pre-shelving) design follows.

Adds a second, parallel eval tier — a
downloaded public IR dataset (SciFact) scored with aggregate IR metrics — to break the
measurement ceiling the synthetic 30-note corpus hit (3b under-shot; Slice 4's reranker came
back net-negative *because* vector never fails on the toy set).

## What this is

A **manual measurement tier** that runs real retrieval (keyword / vector / hybrid / reranked)
over the full BEIR **SciFact** corpus (~5,183 docs, 300 test queries, with qrels) and reports
**aggregate IR metrics** (nDCG@10, Recall@10, MAP) per method. It downloads the dataset on
demand (cached), runs `#[ignore]`/manual like the existing real-model gate, and sits **beside**
the 30-note tier without changing it. Its headline output is the `reranked − hybrid` nDCG@10
delta on a corpus where vector genuinely fails — the first real signal for the reranker's
D-DELETE decision.

## What this is NOT

- **Not a replacement for the 30-note tier.** That tier stays as the fast, deterministic,
  required-CI **regression** net (per-query keyword snapshots). This tier *measures absolute
  quality*; it does not snapshot per-query rankings and is **not** a required CI job.
- **Not personal/product data.** SciFact is generic scientific-claim retrieval — a real,
  labeled, *interim* proxy with genuine difficulty, explicitly **not** a measure of how Raki
  retrieves a user's own notes. The eventual ground truth is still real personal notes/logs
  (the dual-tier decision: dataset now, personal later). The loader is dataset-shaped so a
  swap is a one-line change.
- **Not deterministic / committed.** Because the data is downloaded (network, non-deterministic
  availability) and the corpus is large, there are **no committed snapshots** for this tier —
  only a recorded author-once measurement and a coarse pipeline-sanity floor.
- **Not the import command.** The minimal programmatic note-import command (toward personal
  notes) is a separate fast-follow slice. This slice is eval-substrate only.
- **Not a production change.** `search_notes`, `raki-storage`, `raki-retrieval`, and the
  30-note `run_eval` machinery are untouched (beyond an optional tiny shared index-build helper).

## Decisions

- **D1 — Dataset: BEIR SciFact, downloaded on demand.** ~5,183 docs / 300 test queries / qrels,
  fetched from the BEIR public zip and cached under a gitignored dir. Chosen for being small,
  tidy, and well-known (published bge-small nDCG@10 ≈ 0.65 — far below the 30-note set's ≈1.0, so
  it breaks the ceiling, with the honest caveat that it has *less* reranker headroom than a
  harder set like NFCorpus). The loader is dataset-agnostic; swapping datasets is trivial.

- **D2 — Aggregate IR metrics, not per-query snapshots.** Score **nDCG@10** (primary, BEIR
  standard), **Recall@10**, and **MAP**, averaged over the 300 test queries, per method
  (keyword / vector / hybrid / reranked), at **k = 10** (IR convention + comparability with
  published baselines; the smoke tier stays at k = 3). Snapshotting 300 queries would be both
  unwieldy and the wrong question — this tier answers "how good, on an absolute scale," not "did
  a specific ranking silently change."

- **D3 — Download-on-demand, cached, `#[ignore]`/manual.** Mirrors the existing real-model gate
  (which already downloads the bge model). Fetch + unzip the dataset to `.beir_cache/scifact/`
  once; reuse the cache thereafter. No network in the required CI path. Adds `ureq` (blocking
  HTTP) + `zip` as `raki-eval` dependencies (small, common).

- **D4 — Real retrieval over the full corpus.** Build a real in-memory index
  (`SqliteNoteRepository` + FTS5 + `SqliteVectorIndex`), embed all ~5K docs with bge-small, then
  for each test query run the production retrieval functions and the reranker. Reranking reorders
  the top-N recall pool (constant, default 100 per IR convention; tunable down to ~20 for speed).
  Recall is measured against the **full** corpus (all docs indexed) so the metric is meaningful.

- **D5 — Headline outputs.** (a) The `reranked − hybrid` nDCG@10 delta — whether the cross-encoder
  rescues relevant docs when vector fails (D-DELETE proxy signal). (b) A **pipeline-sanity check**:
  vector nDCG@10 vs the published ~0.65 — a wildly-off number means our wiring is broken, not the
  model.

- **D6 — Posture: recorded measurement + a coarse sanity floor.** Primary output is a recorded
  author-once measurement (`docs/eval/scifact-baseline.md`: the 4-method table + deltas + date +
  platform + model ids). Plus one light `#[ignore]` `benchmark_gate` sanity floor (e.g. vector
  nDCG@10 ≥ ~0.55) that catches a broken pipeline — **not** a quality-regression gate. No corpus
  tuning; the dataset is fixed upstream.

- **D7 — Parallel tier, shared primitives.** A new `benchmark` module + `bench` binary +
  `benchmark_gate` test in `raki-eval`, reusing the retrieval functions
  (`search`/`vector_search`/`hybrid_candidates`/`rerank`) and metric primitives
  (`recall_at_k`/`ndcg_at_k`/`average_precision_at_k`). The 30-note `run_eval`, snapshot, and gate
  are untouched. If a clean "build in-memory index from `(id, title, body)`" helper falls out, it
  may be shared; otherwise the small build loop is duplicated rather than forcing a refactor.

- **D8 — License/attribution.** SciFact is CC BY-NC 2.0 (non-commercial). We **download at test
  time and do not ship or redistribute** it — research/measurement use, standard. The NC clause is
  noted as a flag should Raki go commercial and want to bundle data (we don't). Attribution recorded
  in `scifact-baseline.md`.

## Architecture / data flow

```
scifact.zip (BEIR public URL) ──ureq download once──> .beir_cache/scifact/ (gitignored)
  corpus.jsonl ({_id,title,text}) · queries.jsonl ({_id,text}) · qrels/test.tsv (qid corpus-id score)
        │ parse (serde_json JSONL + TSV)
        ▼
  real in-memory index (SqliteNoteRepository + FTS5 + SqliteVectorIndex), embed ~5K docs (bge-small)
        │  per test query (300):
        ▼  kw / vec / hyb / reranked  ──> nDCG@10, Recall@10, MAP  ──mean──> per-method aggregate
                                                                   └─> reranked−hybrid delta + vec-sanity
```

## Components touched

- `src-tauri/crates/raki-eval/src/benchmark.rs` — CREATE: dataset loader (download + parse) and
  `run_benchmark(embedder, reranker, k) -> BenchReport` (build index, score 4 methods in aggregate).
- `src-tauri/crates/raki-eval/src/bin/bench.rs` — CREATE: `bench` binary printing the per-method
  table, the `reranked − hybrid` delta, and the vector-sanity line.
- `src-tauri/crates/raki-eval/tests/benchmark_gate.rs` — CREATE: `#[ignore]` sanity-floor test.
- `src-tauri/crates/raki-eval/Cargo.toml` — MODIFY: add `ureq`, `zip`.
- `.gitignore` — MODIFY: add `.beir_cache/`.
- `docs/eval/scifact-baseline.md` — CREATE: recorded author-once measurement + attribution.

## Testing & verification

- **Loader parse test** (no network): a tiny inline corpus/queries JSONL + qrels TSV string parses
  into the expected structs.
- **Aggregation unit test**: hand-built rankings + a tiny qrels map produce the expected
  nDCG@10/Recall@10/MAP means (reusing the metric primitives).
- **`benchmark_gate`** (`#[ignore]`, real model + download): runs the full tier and asserts the
  vector nDCG@10 sanity floor; records nothing (the measurement record is produced by the `bench`
  binary).
- Full `cargo test --workspace --exclude raki` / `cargo fmt --check` / `cargo clippy --workspace
  --exclude raki --all-targets -- -D warnings` green; required CI path (deterministic keyword
  snapshot) unaffected — no new model/network in it.
- `bun run typecheck && bun run build` green (frontend untouched).

## Consequences

- Retrieval quality is finally **measurable on an absolute scale** with genuine headroom; the
  reranker (and later chunking/structure) can show measured lift instead of faith.
- The reranker's D-DELETE decision gains a strong proxy signal (its delta on a hard set) well
  before personal-notes ground truth exists.
- The eval gains a second *measurement style* (aggregate IR metrics) cleanly separated from the
  *regression* style (per-query snapshots) — matched to different questions.
- A network/download dependency enters the eval, but only in the manual tier; the required CI path
  stays deterministic and offline.

## Non-goals (each its own later slice)

- The minimal note-import command and personal-notes ingestion (the fast-follow).
- Per-query snapshotting or required-CI gating of SciFact.
- Chunk-level embedding, structure/recency signals, the generate stage.
- Any change to `search_notes`, `raki-storage`, `raki-retrieval`, or the 30-note tier beyond an
  optional shared index-build helper.
