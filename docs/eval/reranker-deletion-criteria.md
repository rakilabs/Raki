# Reranker deletion criterion (Slice 4, D-DELETE)

Status: OPEN — decided once real-notes ground truth exists.

The cross-encoder reranker (Slice 4) was built as an eval-substrate integration test on a
synthetic 30-note corpus that **cannot see its primary value** (recall-rescue): vector
recall@3 ≈ 1.0, so the relevant note is already in the top-k and there is nothing to rescue.
A nil delta on the *synthetic* set is therefore an expected, acceptable finding (D-FALSIFY)
and does **not** trigger deletion.

This ticket is the kill-switch, committed before attachment so the experiment cannot quietly
become permanent architecture.

## Tripwire

When real-notes ground truth exists (≥ ~100 labeled real queries sampled from actual use):

- Re-measure `reranked` vs `hybrid` on that ground truth.
- If `reranked` does NOT beat `hybrid` on nDCG by a stable, meaningful margin
  (**default +0.03**, re-set once the real query distribution is known) across the real set,
  then **remove**: the `Reranker` port (`raki-domain`), `FastEmbedReranker` + `FakeReranker`
  (`raki-ai`), the pure `rerank` fn (`raki-retrieval`), and the `reranked` eval method
  (`Method::Reranked`, the struct fields, `run_eval`'s reranker arg, the gate floors, the
  report column, the snapshot block).
- `hybrid_candidates` stays regardless — it is a clean recall primitive independent of rerank.

## Why a fixed tripwire now

D-FALSIFY (record the result honestly) is only a virtue if acted on. Writing the deletion
criterion before the result is known prevents the sunk-cost fallacy: the reranker survives by
earning a measured win on real data, not by already existing.
