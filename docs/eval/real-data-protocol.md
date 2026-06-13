# Real-data eval protocol (local, private)

Machinery: `cargo run -p raki-eval --bin real-eval` (real model; reads `eval-data/real/`).
Setup: put `.md` notes in `eval-data/real/notes/`, write `eval-data/real/queries.json`.
If you are dogfooding the Raki app itself, use the **"Export for eval"** button in the Notes view
(or the `export_notes_for_eval` Tauri command) to dump live notes to `eval-data/real/notes/*.md`.
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

### Next-due tracker
Tick a box after each run and set the matching OS/calendar reminder (a multi-month cadence
can't live in an ephemeral scheduler — this checklist + a calendar entry is the durable form).
Re-running early is fine; the dates are the *latest* a run should slip to.

- [ ] 2026-07-06 — monthly (Q1)
- [ ] 2026-08-06 — monthly (Q1)
- [ ] 2026-09-06 — monthly (Q1, last monthly)
- [ ] 2026-12-06 — quarterly
- [ ] 2027-03-06 — quarterly
- [ ] 2027-06-06 — quarterly (then continue quarterly)

## Chunking measurement (added for the chunk-eval slice)
- Run `cargo run -p raki-eval --bin chunk-eval -- --with-real`; it reads `eval-data/real/` via the
  raw-markdown loader (preserves paragraph/heading structure for chunking) and prints whole-vs-chunked
  deltas. The default run (`chunk-eval` without `--with-real`) evaluates only the committed synthetic
  fixtures and is much faster (~2.5 min vs ~20 min).
- Record the synthetic baseline with `cargo run -p raki-eval --bin chunk-eval -- --write`.
- Aggregate real-notes chunking results can be committed (content-free) to
  `docs/eval/chunking-real-notes-summary.md`.
- **Sample the messiest notes**, not just the longest: long multi-section notes, list-heavy notes,
  code-heavy notes, and mixed-language notes — these are where structural chunking and
  prefix↔tokenization interactions break. The promotion gate reads the **long-note stratum**.
- The chunked-vs-whole delta is computed within a single run over the identically-loaded set, so
  attribution is exact; cross-run drift (a living corpus) is inherent and acceptable.

## What the numbers are NOT
Directional, not statistically powered (~20–40 queries); optimistic ceiling (authorship bias);
whole-note plain text, not block-aware. Do not decide the reranker's fate (D-DELETE) on this set.
