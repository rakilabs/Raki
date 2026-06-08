# ADR-0007: Retrieval quality is measurement-gated; benchmark-first failing corpus

- **Status:** Accepted
- **Date:** 2026-06-08
- **Deciders:** Jayden
- **Tags:** retrieval, ai, eval, process

## Context

Retrieval/memory quality is Raki's core differentiator — the product value the whole
architecture exists to serve. After Slices 1–3 the foundation is built and notes are
end-to-end, so retrieval quality is the natural focus for completing Phase 1.

But the measurements say quality work cannot proceed *honestly* yet:

- On the synthetic golden set, **vector/hybrid recall ≈ 1.00, MAP ≈ 0.98** — saturated. The
  bi-encoder never fails; the relevant note is already top-k.
- The cross-encoder reranker (built as an eval-substrate experiment) scores **≈ 0.98 — not
  better than hybrid**, exactly as expected on a corpus with nothing to rescue. Its committed
  kill-switch (`docs/eval/reranker-deletion-criteria.md`) says: attach only if it beats hybrid
  by **+0.03 nDCG** on real ground truth, else delete.
- ADR-0006 already named the constraint: *"Until vector fails, neither fusion, reranking, nor
  chunking can demonstrate measurable lift — so corpus realism is the first dependency of every
  retrieval improvement, not an afterthought."*

So the bottleneck on "best retrieval" is **not a better model — it is a harder, honest
measuring stick.** Optimizing on a saturated corpus would mean shipping guesswork and claiming
wins the eval cannot see — the precise failure ADR-0005 exists to prevent.

A real-data eval tier was designed (`2026-06-06-real-data-eval-substrate-design.md`) and a
SciFact public-benchmark tier was designed and then **shelved** in favor of it
(`2026-06-06-scifact-measurement-tier-design.md`), on the reasoning that personal-notes
distribution is what ultimately matters. That shelving is now the blocker: the real-data tier
needs the user to supply and label ≥~100 real queries, which do not exist yet — so *no*
retrieval lever is measurable today.

## Decision

We make **measurement realism the explicit gate on all retrieval quality work**, and we go
**benchmark-first** to obtain a failing corpus without waiting on private data.

1. **Stand up a public IR benchmark tier (SciFact/BEIR subset)** — reproducible, statistically
   powered, CI-gateable, requiring no private notes — chosen specifically because the bi-encoder
   *fails* on it. This un-shelves the SciFact decision: a benchmark whose only job is to let
   reranking/chunking/query-understanding lift become *visible and provable*.
2. **No retrieval lever (reranker attach, chunk-level embeddings, query understanding) is
   driven or shipped until a corpus that can show its failure exists and gates it in CI.** Lift
   is measured on that corpus, in the ADR-0006 stage order (rerank → chunk → generate-stage).
3. **The real-data tier matures in parallel**, fed by dogfooding (enabled by the privacy/
   data-ownership slice, roadmap P1). The reranker's *final* attach-or-delete verdict is taken
   against real ground truth per its kill-switch; the benchmark unblocks the *engineering and
   directional* measurement immediately.

## Consequences

**Positive**
- Retrieval quality work becomes immediately *measurable* instead of blocked on private data —
  R1 (reranker) and R2 (chunking) can demonstrate or refute lift on a powered corpus now.
- Wins and nil-deltas are honest and reproducible by anyone (no private corpus required to
  rerun the gate). Sunk-cost attachment is prevented: levers earn their place by measured lift.

**Negative / costs**
- The benchmark is **domain-shifted** — scientific-claim retrieval is not personal-notes
  retrieval. A lift on SciFact is *directional* evidence, not proof for Raki's distribution;
  the real-data tier remains the faithful judge for final verdicts.
- Adds a benchmark dataset loader + tier to `raki-eval` and a CI gate to maintain.

**Neutral / follow-ups**
- Supersedes the shelving of the SciFact tier (revisits `2026-06-06-scifact-measurement-tier-
  design.md`); the real-data tier is retained, not replaced — the two are complementary
  (powered+reproducible vs faithful).
- Revisit dataset choice (SciFact vs a BEIR subset) during R0 brainstorming based on size,
  licence, and where the bi-encoder fails hardest.
- The reranker kill-switch verdict still requires real-data ground truth (P1 dogfooding).

## Alternatives considered

- **Real-data-first** — most faithful to the actual distribution and demanded by the reranker
  kill-switch, but needs the user to supply + label ≥~100 real queries and is statistically
  weak at small N. Rejected as the *starting* move because it leaves retrieval work blocked on
  private-data effort; retained as the parallel faithfulness track.
- **Harden the synthetic golden set until vector fails** — cheaper, but synthetic adversarial
  authoring tends to overfit the failure modes we already imagine, and is neither powered nor
  externally reproducible. Weaker than a real benchmark for proving lift.
- **Ship the reranker now anyway** — rejected outright: it scores no better than hybrid on the
  only corpus we have, so this is the unmeasured guesswork ADR-0005/0006 forbid.

## References

- ADR-0005 (retrieval quality is measured), ADR-0006 (staged recall → rerank → generate).
- `docs/eval/reranker-deletion-criteria.md` (the committed kill-switch).
- `docs/superpowers/specs/2026-06-06-scifact-measurement-tier-design.md` (un-shelved here).
- `docs/superpowers/specs/2026-06-06-real-data-eval-substrate-design.md` (parallel tier).
- `docs/ROADMAP.md` — Track A (R0 → R4) sequencing.
- `AGENTS.md §1` — Phase-1 completion definition.
