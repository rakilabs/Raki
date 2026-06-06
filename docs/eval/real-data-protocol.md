# Real-data eval protocol (local, private)

Machinery: `cargo run -p raki-eval --bin real-eval` (real model; reads `eval-data/real/`).
Setup: put `.md` notes in `eval-data/real/notes/`, write `eval-data/real/queries.json`
(`[{ "query", "relevant_ids": ["note-slug"], "primary"?: "slug", "category"?: "..." }]`).

## Labeling discipline (D6) — highest-leverage first
1. **Query like a vague future self** — half-remembered, approximate terms, NEVER the note's
   exact words. This is the primary anti-bias action; absolutes remain an optimistic ceiling.
2. **Author + label from memory, before running retrieval.**
3. **Short wait** (a few hours) before running — tertiary.
4. **Phase-2 pooling to ~top-20**: after a run, add any *additional* genuinely-correct note you
   missed; never label toward what ranked highly. Incomplete pooling biases metrics *down* (safe).
5. Mark `primary` ONLY when there is an unambiguous single best answer.

## Privacy (D7)
- `eval-data/` is gitignored; notes + queries + per-query/per-category detail are LOCAL ONLY.
- Only `docs/eval/real-data-baseline.md` is committed — aggregate metrics only, no content.
- Risk: `git add -f eval-data/` would leak private data — do not force-add it.

## Cadence (D8)
Re-run **monthly for the first quarter, then quarterly**, updating the baseline. A lapsed
 cadence is the main way this eval rots.

## What the numbers are NOT
Directional, not statistically powered (~20–40 queries); optimistic ceiling (authorship bias);
whole-note plain text, not block-aware. Do not decide the reranker's fate (D-DELETE) on this set.
