# Real-Data Eval Substrate (Local) — Design

Date: 2026-06-06

Status: Approved (pending implementation plan). Supersedes the shelved SciFact tier
(`2026-06-06-scifact-measurement-tier-design.md`). Measures retrieval on **real personal notes
with real labeled queries** — Raki's actual content distribution and utility function. Hardened
after an adversarial review: the numbers here are an honest *directional* signal and an
*optimistic ceiling*, **not** a statistically-powered or decision-grade measurement (see
Limitations).

## What this is

A **local-only measurement tier**: a loader reads a gitignored directory of the user's exported
Markdown notes plus a hand-authored labeled query set, feeds them through the existing `run_eval`
core (keyword / vector / hybrid / reranked over a real in-memory index), and reports binary
"did I find it" metrics (**Success@3** headline, plus Recall@3/@10, MRR, Success@1,
Primary-Success@1). It runs locally and manually (real model + private data); only *aggregate,
content-free* numbers are committed.

## What this is NOT

- **Not statistically powered, and not the reranker keep/kill decider.** At ~20–40 queries one
  query flipping ranks swings MRR by ~0.02; this cannot detect modest effects with confidence. It
  is a **directional diagnostic**, not significance testing. The reranker's D-DELETE decision is
  **not** settled here (small corpus → tiny rerank pool → tells you nothing about scaled behavior
  or future signals like recency/link-graph). This tier *informs* that decision; it does not make it.
- **Not a realistic-performance number — an optimistic ceiling.** You wrote the notes, so your
  queries are unnaturally aligned with their vocabulary (authorship bias). The measured absolutes
  *overstate* what a cold, half-remembering future user would experience. (The `reranked − hybrid`
  *relative* delta is far more bias-robust — the bias inflates all methods alike — so trust the
  comparison more than the absolutes.)
- **Not committed data.** Real notes/queries are private — gitignored, never in git. Only *code*
  and a *content-free aggregate record* are committed.
- **Not a CI gate or regression snapshot.** The synthetic 30-note per-query snapshot tier remains
  the deterministic, required-CI regression net. This tier measures absolute quality on real data;
  no committed snapshot, not in CI.
- **Not production-faithful at the content layer, but faithful to *current* retrieval.** Current
  retrieval is document-level whole-note embedding — which is exactly what plain-text extraction
  feeds it, so this tier is faithful to the system *as it exists today*. The mismatch is with
  *future* ProseMirror block-aware chunking (ADR-0004/0006), a later slice; that gap is flagged,
  not hidden.
- **Not the persistent import/capture command** (next slice). This slice only *reads* notes.

## Why real data, despite the bias

For personal notes the **user is the authoritative judge of *labels*** — it is their note and
their query, so the correct answer is known with certainty, eliminating the qrel noise that makes
synthetic and crowdworker labels unreliable. The cost is **query bias** (authorship), handled
honestly as a stated limitation, not pretended away. Reliable labels + biased-but-real queries is
still a better substrate than precise labels on the wrong distribution (SciFact) — directional
truth over calibrated fantasy.

## Decisions

- **D1 — Reuse the `run_eval` core.** Extract `run_eval_over(corpus, queries, embedder, reranker,
  k)`; `run_eval` becomes a thin wrapper calling it with the fixture loaders. The synthetic tier,
  snapshot, and gate stay byte-for-byte unchanged — and the **deterministic keyword snapshot gate
  guards the extraction** (it pins exact per-query keyword rankings; any behavior change breaks it).

- **D2 — Local, gitignored data layout.**
  ```
  eval-data/real/              (GITIGNORED — never committed)
    notes/*.md                 exported Markdown (filename-slug = stable note id)
    queries.json               [{ "query", "relevant_ids": ["slug",...], "primary"?: "slug", "category"?: "..." }]
  ```
  `.gitignore` gains `eval-data/`. Invariant: every `relevant_id` and `primary` resolves to a real
  note slug.

- **D3 — Markdown → plain text via `pulldown-cmark`.** Strip YAML frontmatter; filename-slug → id,
  first `# H1` (or filename) → title, body → plain text. Adds `pulldown-cmark` as a `raki-eval`
  dep. Extraction fidelity (frontmatter, wikilinks, HTML, callouts can degrade) is a *tested
  variable*: a fixture with tricky Markdown asserts the extraction does not leak syntax/HTML into
  the embedded text.

- **D4 — Binary relevance; "did I find it" metrics (Success@3 headline).** No grades. For a query
  with relevant set `R` and ranked results:
  - **Success@k** = `1` if `≥1` relevant note in `top_k`, else `0`. **Success@3 is the headline**
    (users scan the top few; finding it at #3 is a win); Success@1 is a secondary "found-it-first".
  - **Recall@k** = `|R ∩ top_k| / |R|`.
  - **MRR** = `1 / rank_of_first_relevant_note`.
  - **Primary-Success@1** = `1` if the query's `primary` note ranks #1 — computed **only over
    queries that mark a `primary`**, and a `primary` is marked **only when there is an unambiguous
    best answer** (if two notes are equally good, mark neither — this avoids measuring labeling
    coin-flips as retrieval failures). **Always reported with its denominator**, e.g.
    `Primary-Success@1: 0.62 (over 8 of 30 queries with an unambiguous primary)` — a naked
    percentage is misleading at small N.

  Reported per method (kw / vec / hyb / reranked): Success@3, Success@1, Recall@3, Recall@10, MRR,
  Primary-Success@1 (with denominator).

- **D5 — Headline outputs (directional).** (a) Per-method Success@3 + Recall@3 + MRR. (b) The
  `reranked − hybrid` delta on Success@3 / MRR — the **bias-robust relative comparison** that is a
  *directional input* to D-DELETE (not the decision; see Limitations). Slice 4's net-negative was
  on a saturated synthetic set; this shows the direction on real notes.

- **D6 — Labeling protocol (honest about its limits).** Author ~20–40 real "find that thing"
  queries. Discipline, highest-leverage first:
  1. **Query like a vague future self, not like the author** — deliberately use half-remembered,
     approximate terms, **never the note's exact vocabulary**. This is the primary anti-bias
     action; it *reduces* (cannot eliminate) structural authorship bias, so the measured absolutes
     remain an optimistic ceiling (stated in Limitations).
  2. **Author + label from memory, before retrieval** (Phase-1 — labels are certain).
  3. **Short wait before running** (a few hours) — tertiary; marginally reduces working-memory
     validation, does **not** fix authorship bias.
  4. **Phase-2 pooling to ~top-20** (sustainable depth, not 50): add any *additional* genuinely-
     correct note found, never labeling toward what ranked highly. Incomplete pooling biases
     metrics *conservatively downward* (a missed relevant under-counts), which is the safe
     direction.
  5. Optional `category` tags (`exact`/`vague`/`paraphrase`/…) for qualitative pattern-spotting
     (kept **local-only**, see D7). Second-judge audit optional (labels are user-certain).

- **D7 — Privacy: commit aggregate-only, content-free.** `docs/eval/real-data-baseline.md` opens
  with an **in-band warning header** so the caveats travel with the numbers:
  `<!-- Directional signal only. Not statistically powered; absolutes are an optimistic ceiling.
  See Limitations in 2026-06-06-real-data-eval-substrate-design.md. -->`. It then contains **only**:
  total query count, per-method aggregate metrics (Success@3/@1, Recall@3/@10, MRR,
  Primary-Success@1 with denominator), date, model ids, platform. **No category tables, no per-query anything,
  no note/query text** — small-count category breakdowns leak the query distribution and failure
  modes, so they live in a **local-only** `eval-data/real/category-breakdown.md`. Per-query detail
  prints to the terminal only. The accidental-commit risk (`git add -f`) is mitigated by the
  `eval-data/` gitignore entry and a note in the protocol doc; a future pre-commit guard is a cheap
  follow-up if wanted.

- **D8 — Accountability cadence (anti-bitrot).** A manual local tier rots if unscheduled. The
  protocol doc records a re-run cadence — **monthly for the first quarter, then quarterly** — and a
  recurring reminder/task is set so the eval stays a living backbone, not a June-2026 curiosity.

## Architecture / data flow

```
eval-data/real/notes/*.md ──strip frontmatter, pulldown-cmark→ (slug, title, plain text) ─┐
eval-data/real/queries.json ──→ EvalQuery (binary relevant_ids, primary?, category?)       ─┤
                                                                                            ▼
        run_eval_over(corpus, queries, embedder, reranker, k)   [reused core]
                                                                                            ▼
   per query: kw / vec / hyb / reranked  ──→ Success@3/@1, Recall@3/@10, MRR, Primary-Success@1
                                                                                            │
   terminal (local): per-query + per-category detail        docs/eval/real-data-baseline.md
                                                             (committed: aggregate-only, content-free)
```

## Components touched

- `raki-eval/src/lib.rs` — extract `run_eval_over(corpus, queries, embedder, reranker, k)`;
  `run_eval` delegates via the fixture loaders (no behavior change; keyword snapshot guards it).
- `raki-eval/src/local_corpus.rs` — CREATE: Markdown-dir + `queries.json` loader (frontmatter
  strip, plain-text extraction, slug ids, `relevant_id`/`primary` resolution invariant).
- `raki-eval/src/bin/real-eval.rs` — CREATE: local binary; loads `eval-data/real/`, runs
  `run_eval_over`, prints per-method + per-query + per-category detail (local), writes the
  aggregate-only `real-data-baseline.md`. Includes `Success@k` / `Primary-Success@1` helpers
  (Recall/MRR reuse `raki-retrieval` primitives). **Missing `eval-data/real/` is a helpful
  onboarding error, not a panic** — it prints the setup steps (export notes to
  `eval-data/real/notes/*.md`, author `queries.json`, see `real-data-protocol.md`) and exits
  cleanly.
- `raki-eval/Cargo.toml` — MODIFY: add `pulldown-cmark`.
- `.gitignore` — MODIFY: add `eval-data/`.
- `docs/eval/real-data-baseline.md` — CREATE: committed aggregate-only record.
- `docs/eval/real-data-protocol.md` — CREATE: D6 discipline, D7 privacy rules, D8 cadence.

## Testing & verification

- **Loader unit test** on a tiny **synthetic committed** `.md` fixture *with tricky Markdown*
  (YAML frontmatter, a wikilink, inline HTML, and a fenced code block): asserts text extraction,
  slug id, `relevant_id`/`primary` resolution, and no syntax/HTML/fence leakage — e.g. a
  ` ```rust\nfn main() { println!("hi"); }\n``` ` block extracts to `fn main() { println!("hi"); }`,
  **not** `rust fn main()...` or backtick noise. Runs in CI; no real data.
- **Metric unit tests** for Success@k / Recall@k / MRR / Primary-Success@1 against hand-built
  rankings (single- and multi-relevant, per D4).
- **Refactor guard:** the existing deterministic keyword snapshot gate stays green after the
  `run_eval_over` extraction (proves behavior preserved on the synthetic tier).
- Real-data eval itself: **local + manual** (real model + private notes), not in CI, no snapshot.
- Full `cargo test --workspace --exclude raki` / fmt / clippy green; required CI path unaffected.
  `bun run typecheck && bun run build` green (frontend untouched).

## Limitations (what these numbers are NOT — quote them with these attached)

1. **No statistical power** at ~20–40 queries — directional, not significant. Do not make a
   keep/delete decision on a delta this size.
2. **Optimistic ceiling** from authorship bias — real cold-recall performance is lower than these
   absolutes; trust the *relative* method comparison more than the absolute scores.
3. **Whole-note, plain-text** — faithful to *current* document-level retrieval, but not to future
   block-aware chunking; intra-note precision (the right paragraph in a long note) is unmeasurable
   until chunk-level retrieval exists (a later slice).
4. **Discipline-dependent** — the protocol's value collapses if queries aren't authored honestly
   or the cadence (D8) lapses. The spec provides machinery; the engineer provides discipline.
5. **Note-length-distribution bias** — whole-note embedding favors short notes. A corpus skewed
   short (< ~300 words) makes this eval *overstate* performance; long notes (thousands of words)
   get compressed into one vector, losing intra-note precision, so the eval *understates*
   performance relative to future block-aware chunking. Read the absolutes against your corpus's
   length profile.

## Consequences

- Retrieval becomes measurable on Raki's real content distribution and utility function — a
  directional feedback loop the team will actually trust and use (the cultural shift from
  benchmark-chasing to product-measurement).
- A reusable **privacy boundary** is established: private data local, only content-free aggregates
  committed.
- The reranker question gets a real-distribution directional read (not a verdict).

## Non-goals (each its own later slice)

- The persistent import/capture command; Markdown → ProseMirror canonical conversion; block/chunk-
  level retrieval and block-aware eval; recency/structure/link/behavioral signals and their eval;
  any CI gating of this tier; committing private data; changes to the synthetic tier or
  `search_notes`.
