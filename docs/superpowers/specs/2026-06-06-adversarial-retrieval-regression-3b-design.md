# Adversarial Retrieval Regression Set (Slice 3b) — Design

Date: 2026-06-06

Status: Approved (pending implementation plan). Supersedes `2026-06-05-eval-corpus-realism-design.md`
(the pre-split draft). Revised after an adversarial review that correctly flagged the earlier
framing as overclaiming "realism."

## What this is

A small, **controlled, deterministic adversarial regression set** with two jobs:

1. Give Slice 4 (the cross-encoder reranker) a **measurable bi-encoder-vs-cross-encoder
   comparison** on failure modes a cross-encoder is theorized to fix — so its lift (if any) is
   a number, not a vibe.
2. Act as a **CI safety net** — per-query snapshots that fail loudly if a future change
   silently breaks a known-hard query.

That is the entire claim. It measures a *relative* comparison and guards against *regressions*
on a toy corpus. It runs in seconds, in CI, on hand-authored data.

## What this is NOT (stated up front, because the earlier draft overclaimed)

- **Not "realism," and not a measure of product quality.** ~30 hand-authored notes say nothing
  about whether a real user finds their own note. A clean "recall@3: 0.82 → 0.91" result is
  real but **narrow** — it is about the engineered effect, not generalization. Do not quote
  these numbers as evidence the product retrieves well.
- **No external validity.** The instrument has caught real bugs (the FTS5 implicit-AND bug,
  keyword recall 0.29→0.85; the RRF regression) — that is genuine diagnostic value. But it has
  **zero** inter-annotator agreement with real users and no correlation with downstream task
  success. The eventual ground truth is real user search logs; this is a cheap interim probe,
  not a substitute.
- **The holdout slice is a discipline gesture, not generalization.** With ~1–2 holdout queries,
  per-query variance at k=3 makes dev-vs-holdout nearly noise. It is reported, never gated on,
  and claims nothing statistical.
- **The taxonomy is not exhaustive.** It deliberately omits recency ("the note from last week"),
  task-context, structured/"the note with the espresso-ratio table" queries, code-heavy notes,
  other languages, and the real-user long tail. Those are real churn drivers and are **out of
  scope** — structure/recency is a named separate lever in ADR-0006; the rest await real data.
- **Grades are coarse editorial judgment, not ground truth.** A 3/1/0 scale where 3 = the
  direct answer, 1 = a same-topic sibling, 0 = different topic. The 1-vs-0 boundary is fuzzy by
  nature; nDCG mostly rewards getting the *direct answer* to the top, which is the defensible
  part. We do not pretend the sibling grades are facts.

## What keeps it honest

The 3a protocol: true qrels (no label-bending), grades on ordering categories, a labeling
rubric + second-judge audit, dev/holdout discipline, committed per-query snapshots. The
critical addition this revision makes is **author-once** (D1/D3): the corpus is written once
from a *fixed-in-advance* taxonomy and measured once — it is **not** iterated against the
model's score, which would be tuning.

## Decisions

- **D1 — Author once, report what the instrument shows.** Write the notes/queries once from the
  fixed D2 taxonomy, run the eval once on the real model, and record the result. There is a
  **diagnostic expectation** — if the modes are as hard as theorized, vector overall recall@3
  lands below ~0.85 and fails (recall@3 < 1.0) in ≥3 categories. But that expectation is **not
  a target to author toward**: if the run comes in saturated, the honest output is a *recorded
  finding* ("these modes don't break bge-small as hypothesized — Slice 4 may lack headroom
  here"), **not** a license to keep adding notes until the number appears. No iterating against
  the score. The ~0.85 / ≥3 figures are rough heuristics for "is there visible headroom," not
  principled gates.

- **D2 — Three reranker-relevant failure modes** (where a cross-encoder reading query+note
  jointly is *theorized* to beat a bi-encoder — D9 covers the case where it does not):

  1. **`dense-near-duplicate`** (ordering, **graded**). ~6 near-identical espresso-dialing notes
     (sour, **bitter**, channeling, grind size, dose/ratio, temp/timing) extending the existing
     `n7`. Query *"my espresso tastes bitter and harsh"* → bitter note grade 3, same-topic
     siblings grade 1. Vector pulls the cluster and mis-orders within it.
  2. **`paraphrase-distractor`** (ordering, **graded**). *"why is my bread dense and didn't
     rise"* → true = a sourdough proofing/rise note (grade 3); a coffee "channeling / dense
     puck / tamping" note + a generic baking note are surface-closer distractors (grade 0/1).
  3. **`polysemy`** (binary). *"how do I get rid of rust"* → a new literal-corrosion note
     ("rust on garden tools / a bike chain") is relevant; the Rust-language notes (`n3`, `n10`)
     are the wrong sense. One or two traps suffice.

  Existing categories are retained unchanged as coverage and a fast-smoke baseline.

- **D3 — Fixed scope: ~8–13 new notes, ~5–10 new queries, authored once.** Corpus grows 22 →
  ~30–35. Time-box: ~2 days. Pick the modes, write the notes, run it once. The marginal note
  authored *after* seeing the score is tuning, not coverage — so there isn't one.

- **D4 — Grades on ordering categories (enforced, kept coarse).** `dense-near-duplicate` and
  `paraphrase-distractor` carry 3/1/0 grades (enforced by the 3a-i `ordering_categories_carry_grades`
  invariant). `polysemy` is binary. Grade boundaries are deliberately coarse; we do not
  over-engineer the 1-vs-0 line.

- **D5 — dev/holdout tagging, honestly weak.** Each new query carries `set: "dev" | "holdout"`;
  hold out ~1 per new category. This is a discipline boundary (don't inspect holdout while
  authoring), explicitly **not** a generalization guarantee at this N. Reported separately,
  never gated.

- **D6 — Labeling rubric + author discipline.** Label Phase-1 (corpus-based, one-line rationale)
  then Phase-2 (pooled candidates, document-based). Include genuinely hard cases; never shape a
  note or label so the reranker wins, and never re-label after seeing a score. Provenance
  `judged`.

- **D7 — Second-judge audit of the new labels.** Claude/subagent consistency cross-check + the
  human as final judge; recorded in `docs/eval/judge-log.md`.

- **D8 — One-time, documented, downward re-baseline.** Vector/hybrid recall drops, so regenerate
  `snapshot.json` + `baseline.md` (`eval-report --write`, reviewed; fingerprint change expected)
  and recalibrate the per-method floors **down** to ~0.10 below observed, dated comment, with
  the ordering-nDCG floor extended to the two new graded categories. This is a *test-got-harder*
  recalibration, not a quality regression (ADR-0005 governs *tuning* regressions). It happens
  **exactly once** here. If a later slice finds itself re-baselining downward *again*, that is a
  smell — the response is to question the change, not to keep lowering the ruler. Up-only
  ratcheting resumes; per-query snapshots guard query-level rot across the change.

- **D9 — Falsification clause (no assumed win).** This set is built *before* the reranker and
  must not assume it. If Slice 4 shows **no delta** on 3b, the result is diagnostic, not hidden:
  decide between (a) the cross-encoder is weak at this scale, (b) the corpus is too easy even
  for the bi-encoder, or (c) the failure mode is genuinely not cross-encoder-fixable. The spec
  records this as an expected possible outcome, not a failure of the slice.

## Relationship to Slice 4

3b is the prerequisite for **measuring** the reranker's lift on the 3a instrument — **not** for
building it. The two can overlap: the reranker can be built against the 22-note smoke set while
3b is authored; 3b is what turns "feels better" into a number (or, per D9, into "no measurable
delta here"). This is not a waterfall gate.

## Architecture / data flow

No retrieval code changes. `run_eval` is unchanged; only the *data*, the loader invariants, and
the gate floor constants change.

```
corpus.json  (22 → ~30–35 notes)        ─┐
queries.json (+~5–10 q's, set + grades) ─┼─> run_eval (UNCHANGED) ─> per-query R@3/MAP/MRR/nDCG/Cov
labeling-rubric.md / judge-log.md       ─┘     ├─> eval-report --write ─> snapshot.json + baseline.md (re-baselined once)
                                               └─> gate ─> per-query snapshots + per-method floors (recalibrated once)
```

## Components touched

- `src-tauri/crates/raki-eval/fixtures/corpus.json` — +~8–13 in-persona adversarial notes.
- `src-tauri/crates/raki-eval/fixtures/queries.json` — +~5–10 queries (`set`, grades on the two
  ordering categories, binary polysemy).
- `src-tauri/crates/raki-eval/src/lib.rs` — loader invariants only (new mandatory categories
  present; bumped corpus-size threshold). No `run_eval` logic change.
- `src-tauri/crates/raki-eval/tests/eval_gate.rs` — per-method floors recalibrated down (dated
  comment); ordering-nDCG floor extended to `dense-near-duplicate` + `paraphrase-distractor`.
- `docs/eval/snapshot.json`, `docs/eval/baseline.md` — regenerated once.
- `docs/eval/judge-log.md` — new-label audit record.

## Testing & verification

- **Loader test** (fake embedder): corpus ≥ new threshold; new mandatory categories present;
  ordering categories carry grades; every relevant/grade id resolves.
- **Harness test** (fake embedder): scoring runs over the larger set without panic; nDCG
  computed for the new graded categories; metrics in `[0, 1]`.
- **Real-model report** (`eval-report`): run **once**; record what it shows against the D1
  expectation (met, or the honest under-shot finding). Not re-run-until-pretty.
- **Gate** (real model): recalibrated per-method floors pass; deterministic keyword snapshot
  gate passes against the re-baselined snapshot.
- **Label audit** (D7) recorded in `judge-log.md`.
- Full workspace `cargo test` / `fmt` / `clippy` green; `bun run typecheck && bun run build`
  green (frontend untouched).

## Consequences

- The eval gains a deterministic adversarial regression set + a bi-vs-cross measurement
  substrate — honestly scoped as a safety net and a relative probe, not a product-quality
  verdict.
- The gate floors drop once (documented). Per-query snapshots keep query-level rot guarded.
- **ADR-0006 nuance:** "corpus realism is the first dependency of every retrieval improvement"
  is too absolute. The accurate claim: *corpus headroom is the prerequisite for measuring a
  retrieval improvement's lift via this eval* — not the only path to improving retrieval (model
  swap, metadata/recency boosts, chunking, and user feedback are independent levers). This spec
  treats it as the former.

## Strategic note (the reviewer's deepest point, recorded not buried)

This is evaluation infrastructure built before real users. The genuine long-term ground truth
is real search logs (what users query, what they click), labeled at scale. This slice is a
deliberate, time-boxed interim: cheap, fast, and useful as a regression net + reranker probe —
chosen over "ship-and-log-first" as a conscious bet on getting retrieval architecture right
before users arrive, **not** a claim that synthetic data beats real data. When real notes
exist, the eval should be rebuilt from sampled user data; this set then retires to a smoke test.

## Non-goals (each its own later slice)

- The cross-encoder reranker (Slice 4) — 3b only builds the set it is measured on.
- Chunk-level embedding (Slice 5); broad `buried-fact` / `multi-relevant` / recency / structure
  coverage.
- Any change to `raki-retrieval`, `raki-storage`, or the app; any `run_eval` logic change beyond
  loader invariants and gate floor constants.
