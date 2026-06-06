# Eval label judge log

Records second-judge disagreements and pool-surfaced label changes (rubric Phase 2 / D7).
The subagent cross-check is a *consistency check*, not an independent judge; the human is the
final judge.

| date | query | change | reason | provenance |
|------|-------|--------|--------|------------|
| 2026-06-05 | (all 19 queries) | none | independent subagent cross-check (blind, corpus-only) found zero disagreements with current labels; no Phase-2 pool-surfaced additions required | judged |
| 2026-06-06 | (all 19 queries) | none | Claude (LLM) independent second-judge pass, document-based (D6 Phase-1): 17/19 labels confirmed with no disagreement; coverage set [n3,n10,n9,n19,n20,n21,n22] confirmed complete (all and only Rust notes); negative query confirmed (no corpus answer) | judged |
| 2026-06-06 | "rust borrow checker" | none — ruled Option 1 (keep [n3]) | n20 (E0502, "cannot borrow as mutable because also borrowed as immutable") is a defensible *secondary* relevant, but the human judge ruled to keep `lexical-overlap` single-best-match: label stays [n3]. | judged |
| 2026-06-06 | "E0433" | none — consistency note | label [n9] confirmed correct. Categorized `named-entity` while the sibling exact-code probes E0599/E0502 are `lexical-cluster` with grades. The split is defensible (pure exact-code lookup vs graded ordering probe) but flagged for taxonomy consistency. | judged |
| 2026-06-06 | (6 new 3b queries) | none | blind subagent cross-check of dense-near-duplicate / paraphrase-distractor / polysemy labels; subagent proposed narrower grade-1 sets for dense-near-duplicate (only symptom-direct siblings), disagreeing on 2/6 queries. Human judge ruled to KEEP current labels: the broad grade-1 espresso cluster is intentional — it creates the dense-cluster ordering signal the category is designed to test. Paraphrase-distractor and polysemy: full agreement. | judged |

## 2026-06-06 — Claude second-judge pass (notes)

A second, independent LLM judge (Claude) re-audited all 19 queries against the 22-note
corpus from note content alone, per the D6 Phase-1 rubric. This complements — does not
replace — the human judge (D7).

**Outcome:** the labels are sound. 17 of 19 queries matched with no disagreement, including
both graded ordering clusters (E0599→n21:3, E0502→n20:3, siblings:1) and the coverage set.

**Resolved** — `"rust borrow checker"`: the human judge ruled **Option 1** (keep `[n3]`
only), preserving `lexical-overlap` as single-best-match. `n20` (the E0502 borrow-conflict
note) is a defensible secondary under D1 but was not added. No label changed.

## 2026-06-06 — 3b author-once measurement

Real model, k=3, corpus = 30 notes / 25 queries. Vector OVERALL recall@3 = 0.98.
Categories where vector recall@3 < 1.00: coverage (0.43; recall@10 = 1.00). New ordering-category nDCG (vec):
dense-near-duplicate 1.00, paraphrase-distractor 0.91.

D1 expectation (recall@3 < ~0.85 AND ≥3 categories < 1.0): **under-shot**.
Per D1/D9: the note set is fixed; no notes were added to chase the number. If under-shot,
this is the recorded finding (these modes did not break bge-small as hypothesized — Slice 4
may have limited headroom on this set), not a failure of the slice.

## 2026-06-06 — Slice 4 author-once reranker measurement (D-FALSIFY)

Real model, k=3, corpus = 30 notes / 25 queries. Reranker: `jina-reranker-v1-turbo-en`
over the hybrid recall union (pool 20). `reranked` is `hybrid + rerank`. Measured once; no
tuning. (Corrected 2026-06-06: the first version of this entry mislabeled the
coverage-included OVERALL recall as "non-coverage recall"; the table below uses the gate's
actual `noncov_mean` basis and the per-category report.)

reranked − hybrid nDCG@3 delta (graded categories — the only metric with visible headroom):
- lexical-cluster:        +0.016   (0.92 → 0.94; one category, 2 queries — noise-level)
- dense-near-duplicate:   +0.000   (1.00 → 1.00; saturated)
- paraphrase-distractor:  +0.000   (0.91 → 0.91)

reranked vs hybrid, all metrics:

| metric                          | hybrid | reranked |   Δ    |
|---------------------------------|--------|----------|--------|
| non-coverage recall@3           | 1.00   | 0.98     | −0.02  |
| non-coverage MAP@3              | 0.98   | 0.96     | −0.02  |
| coverage recall@10              | 1.00   | 0.86     | −0.14  |
| OVERALL recall@3 (incl coverage)| 0.98   | 0.95     | −0.03  |
| OVERALL nDCG@3                  | 0.95   | 0.95     |  0.00  |

Reranking is **net-negative on this corpus.** It degrades, not just fails to help:
- `multi-relevant` recall@3: 1.00 → 0.83 (a relevant note reordered out of the top-3),
- `coverage` recall@10: 1.00 → 0.86 (relevant rust notes pushed past rank 10),
- `buried-fact-in-long-note` MAP@3: 1.00 → 0.75 (the correct answer demoted to rank 2).
It helped exactly one place — `semantic-paraphrase` MAP 0.83 → 1.00 — and moved
`lexical-cluster` nDCG by +0.016. Net: small recall/MAP loss, no real ordering gain.

Why this is structural, not a bug: on a corpus where hybrid recall@3 ≈ 1.0, the relevant
note is already in the top-3, so reordering the pool and re-truncating to 3 can only hold or
*lose* recall — never raise it. `reranked recall ≤ hybrid recall` is guaranteed here. The
cross-encoder's real job (recall-rescue — pulling a relevant note up from deep in the pool)
is **unmeasurable** because there is nothing deep to rescue.

Verdict (D-FALSIFY): the measured result is **no ordering gain and a small recall/MAP loss** —
not the hoped-for lift, and honestly recorded as such rather than spun as "nil." This does not
fail the slice: per the spec, a nil/negative *synthetic* delta is an expected outcome, and it
**pre-loads** the D-DELETE case rather than the reranker earning its keep. No corpus tuning was
done. The keep/kill decision is governed by D-DELETE
(`docs/eval/reranker-deletion-criteria.md`), decided on REAL ground truth (≥ ~100 labeled
queries), not this saturated set.
