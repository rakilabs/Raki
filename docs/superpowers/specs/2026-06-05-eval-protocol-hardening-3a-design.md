# Eval Evaluator & Protocol Hardening (Slice 3a) — Design

Date: 2026-06-05

Status: Approved (pending implementation plan) · v3 (revised after three adversarial reviews)

## What this is, and is not

This slice makes the retrieval eval a **small-N regression tripwire and reviewable
instrument** — explicitly **not** a statistical benchmark. Its protection comes from two
things, in this order:

1. **Per-query snapshots** — a committed record of each query's ordered results per method;
   the gate fails if *any individual query* regresses. This is exact **against a fixed,
   deterministic run** (yes/no per query), not statistical, and is the real teeth.
2. **Coarse per-method average floors** — a smoke alarm, secondary to the snapshots.

Confidence intervals, train/dev/test splits, and significance testing are **deliberately
not used**: at N ≈ 18–30 queries they would be false rigor. The honest substitute for
statistics here is the per-query check — which is exact against a fixed, deterministic run,
**not** assumption-free: it holds only insofar as the whole stack is pinned (model artifact,
embedding output, SQLite/sqlite-vec ordering, tie-breaking, platform). Making that pinning
real is the job of D5 (stable ordering) and D10 (recorded environment).

The current evaluator is already correct and discriminating (it moved keyword recall
0.29 → 0.85 on the FTS5 OR-fix and caught a fusion regression). 3a does not rebuild it; it
adds the qrel semantics, ordering signal, labeling protocol, per-query snapshots, reproducible
artifact, and per-method gate that a benchmark needs once we optimize *against* it. **Slice
3b** (later) authors the adversarial corpus *under* this protocol — so the honest instrument
exists and is recorded before the data is expanded by someone who knows the planned fixes.

This is an evaluator + protocol slice: no change to `raki-retrieval`, `raki-storage`, or the
app.

## Scope

In: explicit qrel semantics (no relevance cap); a `coverage` metric/category; graded nDCG
wired in and gated for ordering categories; a two-phase (pooling) labeling rubric; a
held-out discipline set; committed per-query snapshots; a reproducible baseline artifact;
per-method gate floors; a CI workflow that runs the real-model gate; a label audit of the
current set.

Out (each its own later slice): new adversarial notes (3b); the cross-encoder reranker (4);
chunk-level embedding (5); any retrieval/storage/app code change.

## Decisions

- **D1 — True qrels, no cap.** Relevance is the truth about the corpus, never bent to the
  metric. A query with four correct answers has four relevant ids. The earlier
  `|relevant| ≤ k` rule is **removed** — it forced judges to delete real answers so recall@3
  could reach 1.0. Consequence handled honestly in D2.

- **D2 — Metric set matched to failure type** (no single metric pretends to cover all):
  - `recall@3` — "is the answer up top," the product-immediate signal, reported for every
    query. For a query with `|relevant| > 3`, recall@3's ceiling is `3/|relevant|`; that
    ceiling is **recorded per query in the artifact**, and e.g. recall@3 = 0.75 for a
    4-answer query is logged as *correct*, not a failure.
  - `MAP@3`, `MRR` — ranking quality.
  - `nDCG@3` — ordering quality, **graded** (D4).
  - `recall@K_cov` — **coverage**: the real "show me all notes about X" need, reported for
    `coverage`-tagged queries and gated for that category. `coverage_k` is a **recorded
    parameter** (in the artifact, D10), set to **10** for the current 22-note corpus (top-10
    spans ~45% of it — a sensible "find most" horizon); it is revisited and re-justified, not
    silently kept, as the corpus grows in 3b.

- **D3 — Held-out discipline set** (renamed from "locked"; honest about what it is). Each
  query carries `set: "dev" | "holdout"`.
  - `dev` — used freely while building Slices 4–5.
  - `holdout` — run only by the gate; not printed by the default report; **by developer
    discipline, not inspected or tuned against** during Slices 4–5.
  - It lives in the repo and is therefore *inspectable* — so this is a discipline boundary,
    **not** a cryptographic or statistical anti-overfitting guarantee. A stronger future
    option (an out-of-repo holdout) is noted, not adopted now.

- **D4 — Graded nDCG: mandatory and gated for ordering categories.** Categories whose
  failure mode is *ordering* (e.g. `dense-near-duplicate`, `paraphrase-distractor`) **must**
  carry `grades` (e.g. 3 = direct answer, 1 = a genuinely-related sibling/near-duplicate).
  `run_eval` computes `nDCG@3` (already in `metrics.rs`, currently unwired) for graded
  queries and includes it in those categories' per-method gate. nDCG is therefore a required,
  gated measure where ordering is the point — not an optional side channel. Non-ordering
  categories stay binary and simply have no nDCG (no fake binary nDCG; preserves the existing
  contract).

- **D5 — Per-query regression snapshots (the teeth).** Commit
  `docs/eval/snapshot-<date>.json`: per query, per method, the **ordered top-k** (each id,
  its grade, and the category's gated metric values) at the baseline — not just a hit set, so
  a regression where the metric stays equal but the ordering worsens (the direct answer
  demoted below a distractor, or a more-important relevant note replaced by a lesser one) is
  still caught. The gate asserts **no individual query regresses**: no gated metric drops,
  and for ordering categories the snapshotted top-k may not degrade (the direct-answer rank
  must not increase). For this to be exact, the eval imposes a **stable total order** on every
  method's results before scoring/snapshotting — descending score/similarity, ties broken by
  id — so vector-distance ties can never make the snapshot flap (see D11 for production
  parity). It catches "this one important query stopped working," which averages (D8) hide.
  The snapshot changes only via an explicit, reviewed re-baseline that re-commits the artifact
  (D10).

- **D6 — Two-phase labeling rubric (TREC-style pooling), committed to
  `docs/eval/labeling-rubric.md`:**
  - *Phase 1 — corpus-based.* For each query the judge assigns relevant ids + grades **from
    note content alone, before running retrieval**, with a one-line rationale.
  - *Phase 2 — pooled candidates.* Run retrieval; any surfaced note not yet labeled enters a
    candidate pool and is judged **from its content, not its rank**. This is standard pooling
    — it reduces missed-relevant labels — and is not contamination because the decision is
    document-based. Every label added/changed in Phase 2 is logged with reason and flagged
    `pool-surfaced`.
  - *Continuous pooling (future methods).* When a later method (Slice 4 reranker, Slice 5
    chunker) surfaces an unlabeled note in `dev`, that candidate enters the rejudging pool and
    is judged (document-based) **before** its results are used to interpret the gate — so a
    new method is never scored against a pool blind to what only it can find.
  - Relevance definition: "a user issuing this query would accept this note as a correct
    answer." Grade meanings defined. `|relevant|` not capped. Provenance `judged`.
  - **Author discipline:** *include* realistic failure modes — explicitly including cases the
    current system is expected to struggle with (that is good coverage). Do **not** bend
    labels or author notes so a specific planned algorithm (reranker/chunker) wins. The
    constraint is honest relevance, not avoidance of hard cases.

- **D7 — Second judge, honestly scoped.** A subagent cross-check pass catches inconsistencies
  and careless errors but is **not independent** (same model ecosystem, shared assumptions);
  it is labeled a *consistency cross-check*, not a blind judge. Genuine independence requires
  a human — the user is the real second judge, may override, and disagreements are recorded
  in a committed `docs/eval/judge-log.md` (query, the two calls, the resolution).

- **D8 — Per-method average floors = coarse smoke (secondary).** Explicit
  keyword/vector/hybrid floors (recall@3 + MAP@3, plus nDCG@3 for ordering categories and
  recall@10 for `coverage`), read from the committed baseline (D10), conservative margins,
  **ratchet up only** — except a documented re-baseline on a deliberate corpus change, where
  the per-query snapshots (D5) still guard query-level rot across the change. These averages
  are explicitly the smoke alarm; D5 is the lock.

  **Regression gate ≠ quality target.** Passing D5 (no regression) and D8 (above floors)
  means "not worse, and not below the minimum" — **not** "good." Absolute quality lives in
  the baseline artifact (D10); a query can pass the snapshot while still retrieving poorly.
  The gate protects against rot; *improving* weak queries is the job of Slices 4–5, measured
  as a positive delta against the artifact, not as snapshot-passing.

- **D9 — CI: a required deterministic gate, plus a real-model gate where the model is
  guaranteed.** Add `.github/workflows/eval.yml`:
  - **Required, always, no model** (blocking status check): `cargo test --workspace`, `fmt`,
    `clippy`, loader invariants, the fake-embedder harness, **and the keyword-method per-query
    snapshot** — keyword retrieval is real FTS5 and model-independent, so its regressions are
    caught here deterministically. This always runs and always means something.
  - **Real-model gate** (vector/hybrid quality + their per-query snapshots): runs where a
    cached model is guaranteed (a runner with the fastembed cache keyed on model id, or a
    scheduled job) and is **blocking only there**. Where the model is not guaranteed it runs
    best-effort and **non-blocking**, clearly labeled — never "required-but-skipped," which is
    process debt pretending to be a gate.
  The always-enforceable protection is therefore the deterministic fake + keyword gate; the
  real-model quality gate is enforced wherever the model is available and honest about where
  it is not.

- **D10 — Reproducible baseline artifact** at `docs/eval/baseline-<date>.md`: provider +
  library versions (fastembed, ort/onnxruntime), model id + revision, tokenizer/normalization
  note, embedding dimension, sqlite-vec version, rusqlite/SQLite version, platform/arch,
  **fixture content-hash** (sha256 of `corpus.json` + `queries.json`), exact command + flags,
  date, the per-category table (kw/vec/hyb: recall@3 / MAP@3 / MRR / nDCG@3-where-graded /
  recall@10-for-coverage) + overall, `coverage_k` with its rationale (D2), and a note on
  deterministic ordering (the eval imposes a stable total order — descending score, ties by
  id — on every method's results; see D5/D11). The gate floors cite this file.

- **D11 — The eval shares the production retrieval read-path** (documented, not assumed).
  `run_eval` uses the same adapters the app uses — `SqliteKeywordIndex` (FTS5),
  `SqliteVectorIndex` (sqlite-vec), and `raki_retrieval::{search, vector_search,
  hybrid_search}` — on an in-memory DB, so ranking cannot drift between eval and product. The
  one path it bypasses is the `IndexingService` embedding orchestration (it embeds docs
  directly); that path has its own unit tests. This boundary is stated, not silently assumed.
  - **Architecture:** `raki-eval` is a top-level **driver / dev crate** (like the app's
    composition root) and is *permitted* to depend on concrete adapters; that is not an
    inward-dependency violation of the hexagonal rule (production adapters still depend only on
    `raki-domain`). Stated explicitly so the dependency is intentional, not accidental.
  - **Tie-break parity (tracked):** the stable total order D5 applies belongs in
    `raki_retrieval` too, so production and eval order *ties* identically. To keep 3a
    instrument-only it is applied eval-side now and noted as a small retrieval follow-up; until
    then, ordering of *tied* scores may differ between eval and production (non-tied ranking is
    already identical).

- **D12 — Label audit of the current set, ordered correctly.** Audit the **14 existing
  queries' labels against the 22-note corpus** using the Phase-1-then-Phase-2 protocol (D6):
  corpus-based review first, then the per-query retrieval dump (D-tooling below) only to
  surface pool candidates for document-based judgment. Fixes and their reasons are recorded
  in the artifact and judge-log. (Terminology: 22 *notes* are corpus items; 18 *queries*
  carry the qrels.)

## Architecture / data flow

No retrieval code changes. 3a adds: nDCG + coverage computation, the `set` partition,
per-query inspection output, the committed snapshot + artifact, per-method floors, and the CI
workflow.

```
queries.json (+ set, + grades on ordering cats, + coverage tag) ─┐
corpus.json (unchanged, audited)                                 ─┼─> run_eval (real adapters)
labeling-rubric.md / judge-log.md (protocol + disagreements)     ─┘     │  R@3 / MAP / MRR /
                                                                        │  nDCG@3 / R@10
                                          eval-report ──> per-query dump (dev only)
                                                       ──> docs/eval/baseline-<date>.md
                                                       ──> docs/eval/snapshot-<date>.json
                                          gate (holdout) ──> per-query snapshot (D5, exact)
                                                          ──> per-method floors (D8, smoke)
                                          CI (eval.yml) ──> fast always + real-model job
```

## Components touched

- `raki-eval/src/lib.rs` — compute `nDCG@3` (graded queries) and `recall@10` (coverage);
  add them + `set` handling to `run_eval`/`Report`/`MethodScores`; loader invariants (every
  query has a valid `set`; ordering categories carry grades; coverage queries tagged; all
  relevant/grade ids resolve). No `|relevant|` cap.
- `raki-eval/src/main.rs` (`eval-report`) — per-query inspection dump; dev/holdout selection;
  write `baseline-<date>.md` (D10) and `snapshot-<date>.json` (D5).
- `raki-eval/fixtures/queries.json` — add `set`, grades on ordering categories, coverage tags
  to the 18 existing queries.
- `raki-eval/tests/eval_gate.rs` — per-method floors (D8) + per-query snapshot assertions
  (D5) on the `holdout` set.
- `.github/workflows/eval.yml` (new) — D9.
- `docs/eval/labeling-rubric.md`, `docs/eval/judge-log.md`, `docs/eval/baseline-<date>.md`,
  `docs/eval/snapshot-<date>.json` (new) — D6, D7, D10, D5.

## Testing & verification

- **Loader test** (fake embedder, fast): every query has `set ∈ {dev, holdout}`; ordering
  categories carry grades; coverage queries tagged; all relevant/grade ids resolve; no
  relevance cap assertion (its absence is intentional).
- **Harness test** (fake embedder): nDCG computed for graded queries, omitted otherwise;
  recall@10 computed for coverage; per-method scores in `[0, 1]`; dev/holdout selectable.
- **Real-model report** (`eval-report`): produces `baseline-<date>.md` + `snapshot-<date>.json`;
  per-query dump readable; the current-set label audit (D12) completed and recorded.
- **Gate** (real model): per-query snapshots pass and per-method floors pass on `holdout`.
- **CI**: `eval.yml` runs the required deterministic gate always (incl. the keyword per-query
  snapshot), and the real-model gate blocking where the model cache is guaranteed,
  non-blocking and labeled where it is not.
- Full workspace `cargo test` / `fmt` / `clippy` green; frontend untouched.

## Consequences

- The eval becomes explicit, reproducible, auditable, and protected at the per-query level —
  so Slices 3b / 4 / 5 are measured on a ruler we trust, with deterministic regression
  detection rather than fragile averages.
- True relevance is preserved (no metric-driven label deletion); multi-answer and coverage
  queries are scored by metrics that fit them.
- Ordering failures become first-class and gated (nDCG), closing the recall-vs-ordering blind
  spot the reranker (Slice 4) must later move.
- The holdout set is honestly a *discipline* boundary; the always-required deterministic gate
  (incl. keyword snapshots) is enforced in CI, and the real-model gate blocks wherever the
  model is guaranteed and is labeled non-blocking where it is not — no claim of protection
  that isn't enforced.
- Declined on purpose at this N: confidence intervals, formal train/dev/test splits, category
  weighting — recorded so the omission is a judgment, not an oversight.

## Non-goals

New adversarial notes (Slice 3b); the cross-encoder reranker (Slice 4); chunk-level embedding
(Slice 5); any retrieval/storage/app code change.
