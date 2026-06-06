# Cross-Encoder Reranker (Slice 4) — Design

Date: 2026-06-06

Status: Approved (pending implementation plan). Implements the **rerank** stage of ADR-0006
(staged retrieval: recall → rerank → generate), scoped **eval-substrate first** — measured in
the eval, not yet wired into the app's `search_notes`. Hardened after an adversarial review that
correctly flagged the first draft as architecture-for-its-own-sake; this revision reframes the
deliverable, states the real costs, and commits a deletion criterion.

## What this is

The precision stage of the staged pipeline: a local cross-encoder (ONNX via the existing
`fastembed` stack) that reads each *(query, note)* pair jointly and reorders the recall union. It
lands as a measured stage in the eval — reported as `reranked (= hybrid + rerank)` — gated by the
existing per-query snapshots and per-method floors. Production wiring (the app's `search_notes`)
is a deliberate later step.

## Why build it now — the eval framework is the deliverable

The honest justification is **not** "complete the recall→rerank→generate diagram" (the preceding
recall stage already scores ~1.00, so there is no box that *needs* filling). It is this:

**Phase 1's real product is a trustworthy, extensible measurement framework** — the thing that
stops us lying to ourselves about retrieval quality once real data arrives. This slice is that
framework's first **integration test under real load**: a real ONNX cross-encoder model, a new
port abstraction (`Reranker`), and the full recall→rerank path exercised end-to-end and
regression-gated. If the framework can wire, measure, and *honestly report a nil delta* for a
cross-encoder, it has proven it can measure the reranker's real value the moment real ground truth
exists.

Completing the staged-pipeline architecture so the reranker is ready-and-gated when real notes
arrive is a genuine secondary benefit — but it is secondary, and it does not by itself justify the
slice. The deletion criterion (D-DELETE) is what keeps this from becoming architecture-for-its-own
-sake: we build the experiment with a kill-switch attached.

## What this is NOT

- **Not a product-quality claim.** On the current 30-note corpus the bi-encoder already scores
  vector recall@3 ≈ 1.00 in every real category. A measured positive delta would be a *graded
  ordering win on a toy corpus*, not evidence the product retrieves well.
- **Not "cheap."** "No new Cargo dependency" is literally true (reranking ships in the `fastembed`
  crate we already use) but it is **not** free. A cross-encoder is a new **runtime model**: the
  reranker is ~280M parameters (~8× the 33M-param embedder) — hundreds of MB of ONNX weights to
  download, cache, hold in memory, and run inference over. That is real CI time, disk, and new
  failure modes, incurred for a lever with thin measurable headroom. We state the cost rather than
  hide it behind the crate boundary.
- **Not a corpus change.** We do **not** author or tune fixtures to manufacture reranker headroom
  (the author-against-a-number treadmill ADR-0005 forbids, and the move that under-shot in 3b).
- **Not a production behavior change.** `search_notes` stays on `hybrid`. No latency budget, no
  background model load, no feature flag — those ship with production wiring, later.
- **Not the generate stage** (LLM query rewriting / HyDE / synthesis) or chunking — separate,
  later slices.

## What the eval can and cannot see (stated plainly)

A cross-encoder has two jobs; this corpus can only see the weaker one:

- **Recall-rescue (the headline value) — UNMEASURABLE here.** Pulling a relevant note from deep in
  the pool up into the top-k is the reranker's primary worth. It is structurally invisible on this
  corpus: recall@3 ≈ 1.00 means the relevant notes are *already* in the top-3, so there is nothing
  to rescue.
- **Graded reordering — visible, but thin.** Among an already-relevant pool, lifting the grade-3
  *direct* answer above grade-1 siblings. This is visible only on the graded categories (3-vs-1
  labels), where vector nDCG@3 sits at `lexical-cluster` 0.92 / `paraphrase-distractor` 0.91 /
  `dense-near-duplicate` 1.00 — i.e. **~0.08 of headroom on two categories.** Graded nDCG carries a
  real (minor) ordering signal; it is not nothing, but it is the *secondary* job, not the one that
  justifies a reranker in production.

This is why D-FALSIFY treats a nil *synthetic* delta as an acceptable recorded finding, and why
D-DELETE pins the keep/kill decision to *real* ground truth, where recall-rescue becomes visible.

## Decisions

- **D1 — Model: smallest adequate fastembed reranker, swappable behind the port.** The `Reranker`
  port is model-agnostic. Because quality differences between rerankers are *noise* at this N
  (benchmarking them now would be the exact over-investment this slice avoids), the principled
  choice is the **cheapest** one: pick the smallest/fastest reranker `fastembed` offers that loads
  cleanly (the implementation plan verifies the available `RerankerModel` variants and their sizes;
  `bge-reranker-base` is the fallback only if no clearly-smaller variant works). The choice is
  explicitly **unbenchmarked** and **revisited when real data can distinguish models**, recorded in
  `baseline.md` via `model_id()`.

- **D2 — A `Reranker` port mirroring `EmbeddingProvider`** (`raki-domain/src/ports.rs`):
  `locality()` (Local — no egress), `model_id()`, and
  `async fn rerank(&self, query: &str, documents: &[String]) -> Result<Vec<RerankScore>, DomainError>`
  with `RerankScore { index: usize, score: f32 }` (higher = more relevant). Two adapters in
  `raki-ai`: `FastEmbedReranker` (real, wraps `fastembed::TextRerank`) and `FakeReranker`.

  - **`FakeReranker` is an orchestration stub only.** It scores by deterministic query/document
    token-overlap so `run_eval` runs offline and the fake-embedder harness test has something
    non-trivial to assert. Its doc comment must state, in so many words, that it is *structurally
    uncorrelated with real cross-encoder scoring* — it proves the plumbing (index→id mapping,
    truncation, empty pools), **not** the model. Real-model behavior is validated only by the
    `#[ignore]` integration test (D8).

- **D3 — Recall and truncation are split, behind a characterization test.** `hybrid_search` is
  refactored to `hybrid_candidates(...).truncate(k)`, exposing the uncut union (`pool = 20`, the
  existing `HYBRID_CANDIDATE_POOL`). This is behavior-preserving *by construction* (truncating an
  unchanged union equals the old truncate-of-union), but "by construction" is not a proof: a
  **characterization test pinning `hybrid_search`'s exact output ordering on a fixed corpus is
  committed before the refactor and re-run after** (red-green safety, not trust). `reranked` =
  `rerank(query, hybrid_candidates(...))`.

- **D4 — A pure `rerank` function in `raki-retrieval`.**
  `rerank(reranker: &dyn Reranker, query: &str, candidates: &[(String, String)], k) -> Result<Vec<String>, DomainError>`
  ((id, text) pairs in; reordered top-k ids out). The cross-encoder needs note *text* — not the id
  lists the bi-encoder stages pass — so the recall union is re-hydrated to `(id, text)` before
  rerank. In the eval, `run_eval` supplies text from its in-memory corpus.

- **D5 — `reranked` is measured as `hybrid + rerank`, headlined by the delta.** Mechanically it is
  a first-class `Method::Reranked` (the cleanest way to reuse the snapshot/floor machinery):
  `QueryResult.reranked`, `Report.overall_reranked`, the `method()` arm; `MethodScores` reused
  unchanged. But **presentation must not imply a peer recall strategy** — `keyword`/`vector`/`hybrid`
  are recall methods; `reranked` is a *reorder of hybrid*. The report labels it
  `reranked (= hybrid + rerank)`, and the **headline figure recorded in `judge-log.md`/`baseline.md`
  is the `reranked − hybrid` nDCG delta on the graded categories**, never a standalone four-method
  bar chart that invites false comparison.

- **D6 — Gate posture (additive).** The deterministic gate (`keyword_snapshot_is_deterministic`)
  is **unchanged** — `[Keyword]` only (reranked is model-dependent). The real-model gate extends
  its snapshot check to `[Vector, Hybrid, Reranked]`, adds `Reranked` to the ordering-nDCG floor
  loop, and floors reranked recall/MAP **~0.10 below observed** (measure-then-floor). Existing
  floors are **not** moved — this is an *additive* re-baseline (new method's snapshot block + new
  floors), not a downward recalibration.

- **D7 — One-time additive snapshot/baseline regen.** Adding `reranked` to `QueryResult` grows
  `snapshot.json` (a new per-query block) and `baseline.md` (the `rr` column + reranker `model_id`).
  Regenerated once via `eval-report --write`, reviewed. The **fixtures fingerprint is unchanged**
  (fixtures untouched — only the schema grew); a changed fingerprint here would be a bug.

- **D-FALSIFY — A nil *synthetic* delta is an acceptable recorded finding.** Done = the stage ships
  gated and the measured `reranked − hybrid` nDCG delta on the graded categories is recorded. A
  positive delta is a real (small) ordering win. A ~nil/negative delta is **not hidden**: it is the
  recorded finding ("the cross-encoder does not lift bge-small's *visible* ordering at this scale —
  its real job, recall-rescue, is unmeasurable here; revisit with real data"). No corpus tuning to
  produce a delta.

- **D-DELETE — The kill-switch, committed now (before attachment).** D-FALSIFY is only honest if we
  act on it. So this slice also produces a tracked **deletion ticket** with a concrete tripwire:
  *when real-notes ground truth exists (≥ ~100 labeled real queries), if `reranked` does not beat
  `hybrid` on nDCG by a stable, meaningful margin (default **+0.03**, re-set once the real
  distribution is known), then the `Reranker` port, both adapters, the `rerank` function, and the
  eval column are removed.* A nil delta on the *synthetic* set does **not** trigger deletion (the
  substrate cannot see recall-rescue); a nil delta on *real* ground truth does. The ticket is
  written as part of this slice, not deferred to someday.

## Architecture / data flow

```
                 ┌─ search (keyword) ─────────────────┐
query ──┐        ├─ vector_search ────────────────────┼─> hybrid_candidates (union, pool=20)
        └────────┘                                     │        │
                                                       │        ├─ .truncate(k) ─────────────> hybrid   (production, unchanged)
                                                       │        └─ rerank(query, (id,text)…) ─> reranked (= hybrid + rerank; eval-only)
                                                       │                 │
raki-domain::Reranker ◀── FastEmbedReranker (real) / FakeReranker (stub) ┘
```

## Components touched

- `raki-domain/src/ports.rs` — `Reranker` trait + `RerankScore`; crate-root export.
- `raki-ai/` — `FastEmbedReranker` (wraps `fastembed::TextRerank`), `FakeReranker` (token-overlap
  stub, with the orchestration-only warning comment); `lib.rs` exports. No `Cargo.toml` change.
- `raki-retrieval/src/search.rs` — extract `hybrid_candidates`; add pure `rerank`; the
  `hybrid_search` characterization test; `lib.rs` exports.
- `raki-eval/src/lib.rs` — `Method::Reranked`, `reranked` fields, `method()` arm, `run_eval`
  reranker arg, reranked scoring; harness test asserts reranked computed + graded nDCG.
- `raki-eval/src/main.rs` — construct the reranker; `rr` column labeled `reranked (= hybrid +
  rerank)`; report the `reranked − hybrid` delta; `--write` includes reranked.
- `raki-eval/tests/eval_gate.rs` — construct rerankers; `Reranked` in snapshot methods, floors,
  ordering-nDCG loop; the real-model edge-case test (empty doc, oversized text, empty pool).
- `docs/eval/snapshot.json`, `docs/eval/baseline.md` — regenerated once (additive; fingerprint
  unchanged).
- `docs/eval/judge-log.md` — the measured `reranked − hybrid` nDCG delta record (D-FALSIFY).
- The **deletion ticket** (D-DELETE) — tracked wherever the project tracks work.

## Testing & verification

- **`raki-retrieval`:** the `hybrid_search` **characterization test** (pins exact output before/after
  the D3 refactor); `rerank` pure unit tests (reorders, maps `index`→id, truncates to `k`, empty /
  `k > len` edges) with a stub reranker; `hybrid_candidates` test.
- **`raki-ai`:** `FakeReranker` deterministic unit test; `FastEmbedReranker` `#[ignore]` real-model
  test that loads the model **and exercises edge cases** — empty document, oversized/over-token text,
  empty candidate list — asserting no panic and well-formed scores (this is where real ONNX failures
  hide, per the review).
- **`raki-eval`:** harness test (fake embedder + fake reranker) asserts `reranked` is computed, in
  `[0,1]`, nDCG present for graded categories. `real_model_gate` exercises the reranked snapshot +
  floors.
- **Gate:** deterministic gate unchanged (no new model in the required CI path); real-model gate
  green against the additively re-baselined snapshot + new floors.
- Full `cargo test --workspace --exclude raki` / `fmt` / `clippy` green;
  `bun run typecheck && bun run build` green (frontend untouched).

## Consequences

- The measurement framework gains its first cross-encoder integration test: a new port, a real
  ONNX model, and the full recall→rerank path, regression-gated — with the cost stated and a
  deletion criterion attached.
- `hybrid_candidates` becomes the shared recall primitive both `hybrid` and `reranked` compose, so
  the eventual production path reranks exactly the union it is measured on — proven behavior-
  preserving by the characterization test, not asserted.
- The snapshot schema grows by one method (one-time additive regen; fingerprint unchanged).
- A nil synthetic delta is recorded honestly (D-FALSIFY) and does **not** by itself keep the code
  alive — D-DELETE ties survival to real ground truth. This is the antidote to the build-trap: the
  experiment ships with its own kill-switch.

## Non-goals (each its own later slice)

- Wiring rerank into `search_notes`; the latency budget and background model load that come with it.
  **Pool/latency note:** the eval reuses the existing `pool = 20` constant; when production picks a
  latency-bounded rerank window, the eval re-measures at *that* window — no architecture is
  hardcoded here beyond reusing a constant that already exists.
- Feature-flag plumbing; LLM query understanding / HyDE / synthesis; chunk-level embedding;
  real-notes capture UX (the next strategic slice, and the one that makes D-DELETE decidable).
- Any change to `raki-storage`; any change to `hybrid_search`'s observable output.
