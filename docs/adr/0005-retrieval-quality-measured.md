# 5. Retrieval quality is measured, not vibed

Date: 2026-06-05

## Status

Accepted

## Context

Raki's differentiator is retrieval and memory quality. "It returns results" and "it
returns the right results, ranked well" are different claims; only the second is the
product. Tuning embeddings, k, fusion, or chunking without measurement is guessing.

## Decision

Retrieval quality is a first-class, versioned artifact:

- A **taxonomy-tagged golden set** (`raki-eval/fixtures/`) — queries labeled by
  category (lexical-overlap, semantic-paraphrase, buried-fact-in-long-note,
  multi-relevant, named-entity, temporal, messy, negative). The taxonomy — not the
  size — gives a small set teeth via per-category breakdown.
- **Metrics**: recall@k and MAP@k are the gated bar; MRR is reported; nDCG is
  computed only where graded labels exist (never faked over binary labels).
- A **regression gate** (`raki-eval/tests/eval_gate.rs`) using the real model,
  flooring recall@k AND MAP@k. It is a coarse tripwire, not a statistically
  meaningful benchmark — floors ratchet up, never silently down.
- **Label provenance is tiered**: *judged labels* (hand-curated now; synthetic-
  verified later) are trusted ground truth and kept strictly separate from
  *behavioral signals* (opened-result telemetry — biased, position/UI-dependent),
  which may seed candidates but never count as equal-trust labels.

## Consequences

- Every retrieval change is gated by a measured delta, not a vibe.
- v1 numbers are a bootstrap; the set must grow (synthetic expansion) to earn
  statistical meaning. The format and metrics are the durable contract; label
  sources are pluggable and additive.
- True-negative precision is deferred until a score threshold exists; negative-
  category queries are tracked but unscored in v1 (documented, not silently dropped).
