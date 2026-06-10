# R2 — Record the Chunking Design Baseline; Gate the Migration on P1 — Design

**Date:** 2026-06-10
**Status:** Design — approved, pending spec review
**Governing prior art:** `docs/superpowers/specs/2026-06-06-chunking-eval-substrate-design.md` (the chunking
substrate + the D8 promotion gate already exist). ADRs: ADR-0005/0006/0007 (measurement-gated retrieval).
**Roadmap:** `docs/ROADMAP.md` Track A — R2.

---

## Honesty clause (read first)

The chunking **substrate, design space, and promotion gate already exist** (prior spec, 2026-06-06):
`chunk-eval` runs WholeNote-vs-Blocks across `prefix ∈ {bare,title,title+heading} × rollup ∈
{min-rank,score-max}`, and **D8** already fixed the binding trigger — migrate iff the best config beats
whole-note by **≥ +0.05 Success@3 on the long-note stratum, on REAL notes, by 2026-09-06**. That verdict
**cannot be taken on public data**: SciFact docs have no structure (0 newlines; structural `Blocks` is
inert, only `cap_split` fires on the longer ~third), and BEIR has no openly-downloadable long+structured
set (Robust04/TREC-NEWS are license-gated). So the only structurally-faithful, Raki-faithful corpus is
the user's real notes — i.e. **P1**. This slice therefore does the one honest, doable thing now: **run
the built synthetic arm, record the design baseline, name the winning arm, and make explicit in the
roadmap that the binding R2 verdict (and R1's) wait on P1.** It ships no production change.

## What this is

Add a `--write` flag to the existing `chunk-eval` binary (mirroring `bench --write`) that persists the
**synthetic** WholeNote-vs-Blocks comparison to `docs/eval/chunking-baseline.md`, then run it once to
record the baseline and point the roadmap at P1.

## What this is NOT

- **Not** the production storage migration (the one-to-many chunk-keyed schema) — that is the prior
  spec's D8 trigger, a separate slice, gated on real notes + the D9 perf spike.
- **Not** a real-notes run (the `eval-data/real` path stays stdout-only and is never committed).
- **Not** a new dataset, loader, or corpus (the SciFact/long-doc-public path was ruled out — see the
  Honesty clause).
- **Not** a new ADR or a change to `run_eval_over`, `chunk.rs`, `raki-retrieval`, `raki-storage`, or
  `raki-domain`. Only the `chunk-eval` binary gains `--write`.
- **Not** a `run_eval`/`eval_gate` change — the deterministic keyword snapshot gate is untouched.

---

## Decisions

### D1 — Add `--write` to `chunk-eval`, mirroring `bench --write`
The synthetic section's output is built into a `String` (via `writeln!`, as `bench.rs` does), always
printed to stdout, and — only when `--write` is passed — written to `docs/eval/chunking-baseline.md`
via a `CARGO_MANIFEST_DIR`-relative path (`../../../docs/eval/chunking-baseline.md`, the `bench.rs`
pattern). Default is stdout-only, matching the existing `bench`/`real-eval` CLI contract (no accidental
working-tree writes).

**Write safety (reviews #2/#4/#5).** Before writing, `create_dir_all` the parent directory; propagate
any `io::Error` with `?` so a failed write exits non-zero (never a silent/partial success). We keep the
**direct write** (no temp-file+rename) and the **fixed overwrite** of a single committed filename
deliberately: the target is **git-tracked**, so git is the version backstop (a bad write shows in
`git diff`; `git checkout` restores the prior baseline) and history lives in `git log` — `--force`,
timestamped filenames, or temp-rename add machinery a tracked dev artifact doesn't need, and would
diverge from the established `bench.rs` pattern. `CARGO_MANIFEST_DIR` is compile-time **absolute**, so
the path is cwd-independent (it is not resolved against the working directory).

### D2 — Only the synthetic section is recorded; the real-notes section stays stdout-only
`chunk-eval` also runs the `eval-data/real` path when present. That data is gitignored and never
committed, so `--write` persists **only** the synthetic comparison. The real-notes section continues to
print to stdout (unchanged), guarded by its existing "skipped: {e}" fallback when absent.
**No-PII guarantee (review #1):** `chunk-eval` prints **aggregate metrics only** — recall/MAP floats
(`line()`), per-stratum *counts*, and arm/category *labels*; it never prints note title or body text
(the real-notes block at lines 156–186 emits only the three overall scores). So no note content reaches
stdout for the real-notes path. This slice neither adds nor widens that path; this clause records the
existing guarantee so the privacy posture is explicit, not assumed.

### D3 — Recorded content (the baseline doc)
`docs/eval/chunking-baseline.md` contains, for the committed synthetic corpus (12 notes, k=10):
- the per-arm WholeNote-vs-Blocks deltas across all `prefix × rollup` arms, with **vector and reranked
  headlined and hybrid demoted as the deployment-risk signal** (prior spec D4);
- the per-category deltas (the buried-fact / coreference / list controls);
- a one-line **named winning arm** (the prefix × rollup with the best buried-fact MAP delta, read from
  the table — the *design* choice this slice settles);
- the **embedding + reranker model ids** and a reproducibility note (review #7): the models are
  deterministic (no sampling/seed), so reruns are stable; the recorded winner is **directional and
  configuration-dependent** (it would be re-measured if the model changes);
- a header restating the **honesty clause**: synthetic settles design only; the binding verdict is
  real-notes-gated (prior spec D8: +0.05 Success@3, long stratum, by 2026-09-06) on a real-notes corpus
  — the enabler for which is roadmap Track B **P1** (see D4; P1 is scoped in its own slice, not here).

### D4 — Roadmap records design-settled + P1 gate
`docs/ROADMAP.md` R2 → status "**design-settled** (winning arm recorded in
`docs/eval/chunking-baseline.md`); production migration + binding verdict **gated on a real-notes
corpus** per the 2026-06-06 chunking spec D8." A short cross-cutting note makes explicit that **both
R1's reranker verdict and R2's chunking migration now wait on real notes**, whose enabler is roadmap
Track B **P1** — making P1 the natural next slice. **This spec does not define P1** (review #3): P1's
scope, consent model, corpus-size, and acceptance criteria are its own brainstorm/spec; the quantitative
long-note stratum is **already** defined upstream (prior spec D7: short <~200 tokens / medium / long).

**Ownership (review #8, solo repo):** the developer generates and approves the recorded baseline, and
owns the P1-readiness call; no role matrix for a single-maintainer project.

### D5 — No new gates; deterministic suite unaffected
`chunk-eval` is a standalone `[[bin]]`; it is not run in CI and has no `#[ignore]` gate added here. The
synthetic numbers are directional (12 notes, recall saturates — the ranking signal lives in MAP, per the
bin's own note), so no assertion is committed against them. The reranker/SciFact `benchmark_gate` and the
30-note `eval_gate` are untouched.

### D6 — Extract the report builder + winner rule into the lib, and unit-test it (reviews #6/#9)
The markdown assembly and the winning-arm selection are **logic**, so they move out of the binary into
`raki-eval` (lib), keeping the bin thin (load fixtures → `run_eval_over` per arm → call the builder →
print/`--write`). Concretely, a pure function in the lib:
`render_chunking_baseline(whole: &Report, arms: &[(String /*arm label*/, Report)], models: &str) -> String`
that emits the recorded markdown **and** picks the winner (the arm whose buried-fact-category MAP delta
vs `whole` is greatest). It is **unit-tested with fabricated `Report`s** (no model): a constructed case
where one arm has the highest buried-fact MAP delta asserts that arm is named the winner, and the output
string contains the per-arm table, the per-category rows, the model line, and the honesty/P1 header. The
file I/O (`--write` flag + `create_dir_all` + `fs::write`) stays thin glue in the binary and is covered
by the manual run, not a unit test (it needs no model, but exercising it adds little over the lib test).

---

## Components touched

```
crates/raki-eval/src/lib.rs (or a new src/chunk_baseline.rs)  MODIFY/CREATE  render_chunking_baseline(whole, arms, models) -> String + winner rule + unit tests
crates/raki-eval/src/bin/chunk-eval.rs   MODIFY  thin: run arms → render_chunking_baseline → print; --write (create_dir_all + fs::write, ? on error)
docs/eval/chunking-baseline.md           GENERATED by `chunk-eval --write` (manual, real model)
docs/ROADMAP.md                          MODIFY  R2 → design-settled; migration + verdict gated on real notes (P1 enabler)
```

Reused unchanged: `run_eval_over`, `chunk.rs`, the synthetic fixtures, `raki-retrieval`, `raki-storage`,
`raki-domain`, `run_eval`, `eval_gate`.

## Data flow

```
load_chunking_corpus / load_chunking_queries        (committed synthetic fixtures)
  → run_eval_over(WholeNote, Title, MinRank)         baseline
  → run_eval_over(Blocks, prefix, rollup) ∀ arms     chunked
  → build report String: per-arm Δ (vec/reranked headlined, hybrid demoted), per-category Δ, winning arm
  → println!(report)  +  if --write: fs::write(docs/eval/chunking-baseline.md, report)
  → (real-notes section, if eval-data/real present)  stdout-only, never written
```

## Testing & verification

- Deterministic (CI path, must stay green): `cargo test --workspace --exclude raki` (includes the new
  `render_chunking_baseline` unit test — winner rule + section presence, on fabricated `Report`s, no
  model), `cargo clippy --workspace --exclude raki --all-targets -- -D warnings`, `cargo fmt --check`.
  The change is a lib function + a thin bin flag, so the workspace build and the keyword snapshot gate
  are unaffected.
- Compile check: `cargo build -p raki-eval --bin chunk-eval`.
- Manual (real model, the actual deliverable): `cargo run -p raki-eval --bin chunk-eval -- --write`
  prints the synthetic + (if present) real tables and writes `docs/eval/chunking-baseline.md`. Confirm
  the file contains all arms + the named winner + the model line + the honesty header. Cannot be claimed
  without running.

## Definition of Done

1. `render_chunking_baseline(whole, arms, models)` lives in the `raki-eval` lib, emits the markdown +
   picks the buried-fact-MAP winner, and has a unit test (fabricated `Report`s, no model) asserting the
   winner rule + section/header presence. The bin is thin glue over it.
2. `chunk-eval` accepts `--write`; default remains stdout-only; under `--write` it `create_dir_all`s the
   parent and `fs::write`s `docs/eval/chunking-baseline.md`, propagating any `io::Error` to a non-zero
   exit (review #2).
3. Deterministic suite (incl. the new unit test) + clippy + fmt green; `run_eval`/`eval_gate`/snapshot
   untouched; the real-notes path remains stdout-only and emits no note text (review #1).
4. `chunk-eval --write` run records `docs/eval/chunking-baseline.md` with all arms, per-category deltas,
   the named winning arm, the **model ids + reproducibility note**, and the honesty header (binding
   verdict real-notes-gated).
5. ROADMAP R2 marked **design-settled**, migration + binding verdict gated on a real-notes corpus; the
   cross-cutting note frames **P1 (Track B) as the enabler / natural next slice** (it unblocks both R1
   and R2 verdicts) — without defining P1 here (review #3).
6. **Regeneration trigger (review #10):** regenerate `docs/eval/chunking-baseline.md` (re-run
   `chunk-eval --write`) in the same change whenever `chunk.rs`, the synthetic fixtures, `run_eval_over`,
   or the report format change, so the recorded design baseline never goes stale.
