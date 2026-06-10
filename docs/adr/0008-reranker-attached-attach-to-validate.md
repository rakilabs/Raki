# ADR-0008: Cross-encoder reranker attached to production as attach-to-validate

- **Status:** Accepted
- **Date:** 2026-06-08
- **Deciders:** Jayden
- **Tags:** retrieval, ai, reranker, process

## Context

R0 stood up the SciFact benchmark tier (ADR-0007). On it, the local cross-encoder
(jina-reranker-v1-turbo-en) beats hybrid by **+0.0313 nDCG@10** (also +0.0285 Recall@10,
+0.0319 MAP) — a consistent, multi-metric lift. But SciFact is domain-shifted: the **binding**
keep-or-delete verdict, per `docs/eval/reranker-deletion-criteria.md`, requires **+0.03 nDCG on
≥100 real-labeled personal-notes queries**, which do not exist yet (they arrive via the P1
dogfooding/real-data track).

So we have directional, reproducible evidence the reranker helps, but not the faithful verdict.

## Decision

Attach the reranker to production `search_notes` **as attach-to-validate**:

1. Wire `FastEmbedReranker` into `AppState` as `Option<Arc<dyn Reranker>>`, constructed at startup
   with the embedder's degrade-don't-crash pattern.
2. `search_notes` reranks the 100-candidate hybrid recall union to the top-20, with hard fallbacks
   (missing model, error, 5 s timeout, panic via spawn_blocking's JoinError, out-of-range index) to
   the unchanged hybrid order. Search never breaks; reranking only improves or no-ops.
3. The reranker is local (`Locality::Local`) — no egress, no privacy cost.
4. The kill-switch stays **armed**: the binding verdict is deferred to real-notes ground truth (P1).
   No production telemetry is added; validation is via the eval harness, not metrics.

## Consequences

**Positive**
- Users get the directionally-better ranking now, and dogfooding the reranked experience is how the
  real-notes intuition (and eventually the labeled queries) accrue.
- The hybrid floor is untouched and remains the guaranteed fallback.

**Negative / costs**
- Ships a lever not yet validated on Raki's own distribution — mitigated by the armed kill-switch and
  trivial rollback (the reranker is an `Option`; revert the wiring to return to hybrid-only).
- Adds per-search work (100-note hydration + cross-encoder pass), bounded by `POOL` and a per-candidate
  size cap; latency is watched in dogfooding, with `POOL` as the dial.

## Alternatives considered

- **Don't attach; build the real-notes tier first** — most faithful, but blocks a visible improvement
  on private-data effort (that is the P1 track, pursued in parallel).
- **Attach unconditionally (trust SciFact)** — rejected: SciFact is domain-shifted; the kill-switch
  binds to real data.

## References

- ADR-0006 (staged recall → rerank → generate), ADR-0007 (measurement-gated; benchmark-first).
- `docs/eval/reranker-deletion-criteria.md` (the binding kill-switch).
- `docs/eval/scifact-baseline.md` (the +0.0313 directional basis).
- `docs/superpowers/specs/2026-06-08-r1-reranker-attach-design.md`.
