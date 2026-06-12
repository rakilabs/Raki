# ADR-0009: Memory-Lifecycle Ranking Signals

- **Status:** Accepted
- **Date:** 2026-06-12
- **Deciders:** Raki agent (implementing `docs/superpowers/plans/2026-06-12-r4-memory-lifecycle-signals.md`)
- **Tags:** retrieval, memory, ranking, signals, privacy

## Context

Raki's retrieval layer (`raki-retrieval`) treats every note as equally fresh. In a personal second brain this is wrong: a note the user pinned, opened ten times this week, or edited yesterday is usually more useful than an old draft that happens to contain the same keyword. We need a small, auditable set of **memory-lifecycle signals** that can re-rank retrieval results without adding a second source of truth or leaking user behavior to a cloud provider.

The forces at play:

- **Local-first / privacy:** signals must live in the same SQLite file as notes; cloud providers must not see them.
- **User owns the data:** signal history must be exportable and deletable.
- **Reliability over features:** we will not attach a ranking lever to production search until it proves measurable lift on a corpus where today's retrieval fails (ADR-0007).
- **Provider-agnostic AI:** the signal math cannot depend on a particular embedding model or LLM.
- **Surgical changes:** the mixer must sit behind a narrow domain port so storage, retrieval, and UI can evolve independently.

## Decision

We will introduce three memory-lifecycle signals — **recency**, **pin**, and **salience** — behind domain ports in `raki-domain`, implement a multiplicative `SignalBooster` in `raki-memory`, persist aggregated signals in SQLite via migration V8, and expose only a single `record_note_view` Tauri command. The signal-boosted search path (`hybrid_search_with_signals`) will **not** be wired into production `search_notes` until a real-model evaluation on the R4 seed corpus shows a meaningful lift.

## Consequences

**Positive**

- Retrieval can now be rescued by behavioral context when the user reopens, pins, or frequently references a note.
- All signal math is pure, testable, and model-agnostic; it depends only on timestamps, view counts, and a pinned flag.
- Storage lives in one SQLite file (`note_signals` table, migration V8), so vectors, FTS, and signals move together in one transaction.
- The production code path is protected by a measurement gate; if signals do not help, the default hybrid path is unchanged.

**Negative / costs**

- Adds a new table and migration to maintain; switching embedding models does not automatically recompute signals, only vectors.
- View aggregation is a coarse proxy for importance; a user may open a note many times without it being the "best" answer.
- The initial measurement on the synthetic corpus with a deterministic fake embedder showed **no measurable Success@3 lift** (3/26 queries at k=10 for baseline, all-signals, and each single-signal ablation). The mixer therefore remains off the production path until a real embedding model evaluation is run.

**Neutral / follow-ups**

- Re-run `measure_memory_signals` with the real local embedding provider (`fastembed` / `bge-small`) to get the binding attach/tune/delete verdict.
- If the real-model gate also fails after two tuning iterations, remove the signal-boosted path from the codebase per the kill-switch discipline.
- Link-density and tag-affinity signals are deferred until Phase 2, when the link graph and tag system exist.

## Alternatives considered

- **Additive scoring** (`retrieval_score + signal_bonus`) — rejected because it decouples signal magnitude from retrieval confidence and can push irrelevant notes above relevant ones. Multiplicative boosting preserves the relative ordering produced by retrieval and only amplifies it.
- **Per-provider signal models** (e.g., ask an LLM to re-rank) — rejected because it would make ranking behavior depend on cloud egress and model drift. The chosen math is deterministic and runs locally.
- **Store raw events instead of aggregates** — rejected because the product does not need an event stream today; aggregated `view_count` and `last_accessed_at` are sufficient, smaller, and simpler to export. Raw events can be added later behind the same port if needed.
- **Attach signals immediately to production search** — rejected per ADR-0007. Ranking changes must prove lift before becoming the default.

## Signal design details

### Multiplicative mixer

The booster computes:

```text
recency = 2^(-elapsed_days / half_life_days)   (0 if never accessed)
pin     = pin_boost * pinned
salience = salience_weight * ln(1 + view_count) / ln(10)   (clamped to [0, 1])
raw_boost = 1.0 + recency + pin + salience
capped_boost = min(max_boost, raw_boost)
boosted_score = retrieval_score * capped_boost
```

- A `max_boost >= 1.0` cap prevents any single extreme signal from dominating.
- All four hyperparameters live in `MixerConfig`, validated in `raki-domain`.

### Why views are aggregated

We store `view_count` and `last_accessed_at` rather than a raw event log because:

1. The ranking function only needs these two aggregates.
2. Aggregates are small, private, and trivially exportable.
3. They degrade gracefully: a missing row means "no known interaction," which maps to the default signal vector.

### Why signal math is a `raki-domain` port

`SignalSource`, `SignalStore`, and `SignalBooster` are traits in `raki-domain` so that:

- `raki-memory` and `raki-retrieval` can be unit-tested with fakes.
- `raki-storage` provides one SQLite adapter without leaking SQL upward.
- The frontend and commands depend only on contracts, not implementations.

### R4 split

The work was split into three honest slices:

1. **R4.1** — build the signal model, ports, SQLite storage, and app wiring.
2. **R4-corpus** — create a synthetic real-notes corpus where pure retrieval demonstrably fails.
3. **R4.2** — measure the mixer against the corpus and decide attach/tune/delete.

This split prevents shipping an unmeasured ranking change and keeps the corpus, harness, and production path independent.

## Privacy and retention policy

- Signals are derived from local user behavior; they never leave the device.
- The `record_note_view` command is explicit — views are not inferred from editor focus alone. A future UI may batch or rate-limit calls.
- `note_signals` rows follow the same soft-delete / version / change-log conventions as other user tables (ADR-0002), so they are covered by export and backup.
- A "clear memory signals" setting can truncate `note_signals` without touching authored notes.

## Measurement outcome

- **Corpus:** 26 synthetic real-notes seed notes + 26 queries in `raki-eval/src/memory_corpus/seed.rs`.
- **Baseline:** committed to `docs/eval/r4-memory-baseline.json`.
- **Method:** Success@3 on the top-10 hybrid pool, plus per-signal ablation (baseline, all-signals, recency-only, pin-only, salience-only).
- **Initial result (deterministic fake embedder):**
  - Baseline Success@3: 3/26
  - All-signals Success@3: 3/26
  - Single-signal ablations: 3/26 each
- **Gate:** not met. Success@3 lift is 0, far below the +0.05 absolute threshold.
- **Decision:** **do not attach** `hybrid_search_with_signals` to production search. Keep the ports, storage, booster, and `record_note_view` command in the codebase; the binding verdict is pending a real-model evaluation.

## References

- Plan: `docs/superpowers/plans/2026-06-12-r4-memory-lifecycle-signals.md`
- Design spec: `docs/superpowers/specs/2026-06-10-r4-memory-lifecycle-signals-design.md`
- ADR-0002 (sync-ready data model)
- ADR-0007 (benchmark-first / kill-switch)
- `docs/eval/memory-corpus-DATA_PROVENANCE.md`
- `docs/eval/r4-memory-baseline.json`
