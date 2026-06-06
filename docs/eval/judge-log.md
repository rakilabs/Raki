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

## 2026-06-06 — Claude second-judge pass (notes)

A second, independent LLM judge (Claude) re-audited all 19 queries against the 22-note
corpus from note content alone, per the D6 Phase-1 rubric. This complements — does not
replace — the human judge (D7).

**Outcome:** the labels are sound. 17 of 19 queries matched with no disagreement, including
both graded ordering clusters (E0599→n21:3, E0502→n20:3, siblings:1) and the coverage set.

**Resolved** — `"rust borrow checker"`: the human judge ruled **Option 1** (keep `[n3]`
only), preserving `lexical-overlap` as single-best-match. `n20` (the E0502 borrow-conflict
note) is a defensible secondary under D1 but was not added. No label changed.
