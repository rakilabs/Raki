# Eval Corpus Realism (Slice 3b) — Design [DRAFT — superseded by the 3a/3b split]

Date: 2026-06-05

Status: SUPERSEDED. Adversarial review found this slice conflated "build a trustworthy
measuring instrument" with "expand the data," and led with the data — risking a benchmark
authored by the same person who knows the fixes it must prove. The work was split:

- **Slice 3a — Evaluator & Protocol Hardening** (`2026-06-05-eval-protocol-hardening-3a-design.md`):
  build and lock the instrument first (qrel semantics, graded nDCG, labeling rubric,
  dev/locked split, committed baseline artifact, per-method gate, label audit).
- **Slice 3b — REWRITTEN under 3a's protocol** at
  `2026-06-06-adversarial-retrieval-regression-3b-design.md` (the approved spec; retitled from
  "corpus realism" after an adversarial review). Implement from that file, **not** the text
  below, which is retained for history only.

The original (pre-split) text is retained below for history only.

---

## Purpose

The retrieval foundation (keyword → vector → hybrid) is complete and measured, but the
golden set is too easy: on the current 22-note corpus the embedding model scores
recall@3 = MAP@3 = 1.00 in every category. A saturated test has **no headroom** — the
cross-encoder reranker (Slice 4) and chunking (Slice 5) cannot demonstrate measurable
lift against a test that already reads "perfect."

This slice hand-authors adversarial notes into the golden set so vector retrieval
**measurably fails** in realistic ways, creating the gap the next slices are measured
against. It is the prerequisite named in ADR-0006: *"corpus realism is the first
dependency of every retrieval improvement."*

This is almost entirely a fixtures + recalibration slice. The harness already scores
keyword / vector / hybrid per taxonomy category at k=3; no retrieval or harness logic
changes.

## Scope

In scope: expand `raki-eval/fixtures/corpus.json` and `queries.json`; add failure-mode
categories; recalibrate the regression gate; loader-test update.

Out of scope (each its own later slice): the cross-encoder reranker (Slice 4); chunk-level
embedding (Slice 5); any change to `raki-retrieval`, `raki-storage`, or the app.

## Decisions

- **D1 — Hand-authored adversarial.** Author ~33 new notes by hand with deliberate,
  *realistic* failure traps. Fully-trusted "judged" labels (ADR-0005). No synthetic-
  generation or benchmark-import infra is built now (YAGNI); those remain available as a
  future scaling path once we know which failure modes matter most.

- **D2 — Augment in place.** Grow the existing fixtures (`corpus.json` 22 → ~55 notes;
  `queries.json` 14 → ~28 queries), keeping every existing entry. One corpus, one eval
  run. The per-category taxonomy already provides the separation a second fixture would;
  a separate "hard" fixture is rejected as duplicate maintenance + harness plumbing.

- **D3 — Failure-mode-driven with a sanity floor (the done-criterion).** Author toward
  realistic failure *modes*, not a target number. The slice is "done" when, on the real
  model at k=3:
  - vector **overall recall@3 < ~0.85**, AND
  - **≥ 3 taxonomy categories** score vector recall@3 **< 1.0**.

  If the set is still saturated after authoring, it is not realistically hard enough — add
  more genuine hard cases; never tune notes to hit a number. The floor exists only to
  guarantee the reranker has *something* to improve; it is a minimum, not a target band.

- **D4 — Failure-mode taxonomy.** New / expanded categories, each capturing a real way
  embeddings struggle:
  1. **dense-near-duplicate** — a cluster of 6–8 notes on near-identical sub-topics where
     only fine detail distinguishes the right one. With > 3 siblings, vector pulls the
     cluster but mis-orders within it, so recall@3 drops.
  2. **buried-fact-in-long-note** (expand existing) — a one-sentence answer diluted across
     a long, multi-topic note; whole-note embedding averages it away. Also the chunking
     (Slice 5) motivation, kept measurable here.
  3. **paraphrase-distractor** — a paraphrase query where a semantically closer-*looking*
     wrong note outranks the true (paraphrased) note.
  4. **polysemy** — a query term with multiple senses (e.g. "swift" bird/language,
     "python" snake/language, "java" island/coffee/language) where vector picks the wrong
     sense.
  5. **multi-relevant** (expand existing) — queries with 3–4 relevant notes scattered so
     no single method fits them all into the top-3.

  Existing easy categories (lexical-overlap, named-entity, temporal, messy, negative) are
  retained unchanged for coverage and as a fast-smoke baseline.

- **D5 — Gate re-baseline (one-time, documented, downward).** Vector and therefore hybrid
  recall will drop, so the gate floors (currently 0.90 / 0.90) must be recalibrated **down**
  to ~0.10 below the new observed OVERALL hybrid. ADR-0005's "ratchet up, never silently
  down" governs *regressions from tuning* — making the **test** harder is a legitimate
  re-baseline, not the system getting worse. It is recorded explicitly in the gate comment
  and commit message ("re-baselined for hardened corpus v2, <date>"). Up-only ratcheting
  resumes afterward.

- **D6 — Integrity safeguards.** The honesty spine of a measurement slice:
  - Label by *genuine relevance* — "would a user accept this note as an answer to this
    query?" — **independent of which retriever surfaces it**.
  - Author toward realism, not toward failures imagined in advance, and **not** toward the
    reranker (which does not exist yet and must earn its lift on a set built before it).
  - The sanity floor (D3) is checked by observation, not by editing notes until a number
    appears.

- **D7 — k stays at 3.** The gate and report remain at recall@3 / MAP@3 for comparability
  with Slice 2. Dense clusters (> 3 siblings) and multi-relevant spreads (3–4 answers) are
  what make k=3 discriminating; no need to change k.

## Architecture / data flow

No code paths change. `run_eval` already: builds an in-memory index from the fixtures,
embeds every note, and scores keyword / vector / hybrid per category. This slice changes
only the *data* it runs on plus the gate's floor constants and the loader-test threshold.

```
corpus.json (~55 notes) ─┐
queries.json (~28 q's)  ─┼─> run_eval (unchanged) ─> per-category R/M/MRR for kw/vec/hyb
gate floors (lowered)   ─┘                            ─> eval-report (read) + gate (CI)
```

## Components touched

- `src-tauri/crates/raki-eval/fixtures/corpus.json` — +~33 adversarial notes.
- `src-tauri/crates/raki-eval/fixtures/queries.json` — +~14 queries across the new
  failure-mode categories.
- `src-tauri/crates/raki-eval/src/lib.rs` — loader-test threshold (`corpus.len() >= 50`)
  and a presence assertion for the new mandatory categories. No `run_eval` logic change.
- `src-tauri/crates/raki-eval/tests/eval_gate.rs` — recalibrated `RECALL_FLOOR` /
  `MAP_FLOOR` with a dated re-baseline comment.

## Testing & verification

- **Loader test** (fake-embedder, fast): corpus ≥ 50 notes; every `relevant_id` resolves
  to a real corpus id; the new mandatory categories (`dense-near-duplicate`,
  `paraphrase-distractor`, `polysemy`) are present.
- **Harness smoke** (fake embedder): unchanged 3-method scoring still runs over the larger
  set without panics.
- **Real-model report** (`eval-report`, `#[ignore]`-class manual run): confirms the new
  failure landscape and that the D3 sanity floor is met (vector overall < ~0.85, ≥ 3
  categories < 1.0). Recorded in the commit message.
- **Gate** (real model, `#[ignore]`d): recalibrated floors pass.
- Full workspace `cargo test` / `fmt` / `clippy` green.

## Consequences

- The eval stops reading "everything is perfect" and starts showing where embeddings fail
  — the precondition for Slices 4 and 5 to be measurable rather than vibes.
- The gate floor drops once (documented); this is the expected effect of a harder test,
  not a quality regression.
- The corpus remains hand-authored and small enough to reason about; synthetic expansion
  and benchmark import stay available as later scaling moves, now informed by which failure
  modes proved hardest.
