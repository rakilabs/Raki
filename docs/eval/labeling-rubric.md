# Eval Labeling Rubric (TREC-style Pooling)

## Relevance definition

A note is **relevant** when a user issuing this query would accept it as a correct answer.
Relevance is a judgment about the **document content**, not about retrieval rank or method behavior.

There is **no cap** on `|relevant|` — a query with four correct answers has four relevant ids.
Metrics handle multi-relevance honestly (e.g. recall@3 ceiling is `3/|relevant|` and is recorded as such).

## Grades

### Binary (default)
- **1** — relevant
- **0** — not relevant (absent from grades/relevant_ids)

### Graded (ordering categories only)
Ordering categories (`lexical-cluster`, `dense-near-duplicate`, `paraphrase-distractor`) **must** carry grades:
- **3** — the direct, exact answer the query asks for
- **1** — a genuinely-related sibling / near-duplicate / distractor that a user might accept
- **0** — not relevant (absent)

Non-ordering categories stay binary and have no nDCG.

## Two-phase pooling process

### Phase 1 — Corpus-based (before retrieval)
For each query, assign relevant ids + grades from **note content alone**, before running retrieval.
Write a one-line rationale per query.

### Phase 2 — Pooled candidates (after retrieval)
Run retrieval; any surfaced note **not yet labeled** enters a candidate pool and is judged **from its content, not its rank**.
This is standard pooling — it reduces missed-relevant labels and is not contamination because the decision is document-based.
Every label added or changed in Phase 2 is logged with reason and flagged `pool-surfaced`.

### Continuous pooling (future methods)
When a later method (e.g. reranker, chunker) surfaces an unlabeled note in `dev`, that candidate enters the rejudging pool and is judged document-based **before** its results are used to interpret the gate.
A new method is never scored against a pool blind to what only it can find.

## Provenance

Every label carries provenance `judged`.
Phase-2 additions carry `pool-surfaced`.

## Author discipline

- **Include** realistic failure modes — explicitly include cases the current system is expected to struggle with. That is good coverage.
- **Do not** bend labels or author notes so a specific planned algorithm wins.
- The constraint is honest relevance, not avoidance of hard cases.

## dev / holdout split

- `dev` — used freely while tuning retrieval (Slices 4–5).
- `holdout` — run only by the gate; by **developer discipline**, not inspected or tuned against during Slices 4–5.
- The split lives in the repo and is inspectable; this is a discipline boundary, not a cryptographic or statistical anti-overfitting guarantee.

## Coverage queries

Coverage queries measure "find all my notes about X" — the real need to surface a broad answer set.
They use `recall@K_cov` with `coverage_k = 10`.

Rationale for `coverage_k = 10`: the current 22-note corpus means top-10 spans ~45% of it — a sensible "find most" horizon.
`coverage_k` is revisited and re-justified as the corpus grows (Slice 3b), not silently kept.

## Ordering-category grades requirement

Categories whose failure mode is **ordering** must carry `grades` (see Graded section above).
`run_eval` computes `nDCG@k` for graded queries and includes it in those categories' per-method gate.
nDCG is therefore a required, gated measure where ordering is the point — not an optional side channel.
