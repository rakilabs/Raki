# Adversarial Retrieval Regression Set (Slice 3b) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Author ~8 in-persona adversarial notes + 6 queries (dense-near-duplicate, paraphrase-distractor, polysemy) under the 3a protocol, so the cross-encoder reranker (Slice 4) has a measurable bi-vs-cross comparison and the eval gains a CI regression net — then do the one-time documented downward re-baseline.

**Architecture:** Pure **data + thresholds** — fixtures, loader invariants, and gate floor constants only. `run_eval` logic is untouched. Because the committed `snapshot.json` drives the always-on deterministic keyword gate, **each authoring task regenerates the snapshot** so every commit stays green; only the **per-method floor constants** are recalibrated once, at the end, after the author-once measurement.

**Tech Stack:** Rust, `raki-eval` (driver crate), `serde_json` fixtures, `tokio`, fastembed (real model for `eval-report --write` and the `--ignored` gate).

**Spec:** `docs/superpowers/specs/2026-06-06-adversarial-retrieval-regression-3b-design.md` (D1–D9). This plan implements all of it.

**Current state (verified):** 22 notes (n1–n22; `n7` = "Dialing in espresso", the sour-shot note). 19 queries. Loader invariants in `src/lib.rs`: `fixtures_parse_and_reference_real_corpus_ids` (asserts `corpus.len() >= 20`), `every_query_has_a_valid_set_and_resolvable_ids`, `coverage_queries_have_many_relevant`, `ordering_categories_carry_grades` (lists `lexical-cluster`, `dense-near-duplicate`, `paraphrase-distractor`). Gate `tests/eval_gate.rs`: `keyword_snapshot_is_deterministic` (NOT ignored — runs in `cargo test`), `real_model_gate` (`#[ignore]`); floors `VEC/HYB_RECALL/MAP = 0.90`, `KW_* = 0.75`, `COVERAGE_RECALL10_FLOOR = 0.85`, `ORDERING_NDCG_FLOOR = 0.80`; the nDCG floor loop filters `category == "lexical-cluster"`. `run_eval` assigns deterministic note ids by corpus position, so new notes are stable automatically.

**Author-once discipline (D1/D9):** the note/query set below is **fixed by this plan**. Do **not** add notes beyond it based on an observed score. If the real-model run comes in saturated (vector doesn't drop), that is a **recorded finding** (Task 6), not a trigger to author more.

---

## File Structure

```
raki-eval/fixtures/corpus.json    MODIFY  +8 adversarial notes (n23–n30)
raki-eval/fixtures/queries.json   MODIFY  +6 queries (set + grades on ordering, binary polysemy)
raki-eval/src/lib.rs              MODIFY  loader invariants: new-category presence, size threshold; harness nDCG assertions
raki-eval/tests/eval_gate.rs      MODIFY  per-method floors recalibrated down (once); nDCG floor loop → 3 ordering categories
docs/eval/snapshot.json           REGEN   per authoring task (keeps deterministic gate green)
docs/eval/baseline.md             REGEN   per authoring task; final = the re-baseline artifact
docs/eval/judge-log.md            MODIFY  new-label audit + the author-once measurement record
```

---

## Task 1: dense-near-duplicate — espresso cluster + 2 queries

**Files:**
- Modify: `src-tauri/crates/raki-eval/fixtures/corpus.json`
- Modify: `src-tauri/crates/raki-eval/fixtures/queries.json`

- [ ] **Step 1: Add the 5 espresso-cluster notes**

In `corpus.json`, change the last note's closing line `    "body": "...then it compiled." }` (n22) to end with a comma and append the new notes before the closing `]`:

```json
  { "id": "n23", "title": "Bitter espresso fix",
    "body": "If the shot tastes bitter and harsh, the coffee is over-extracted: the grind is too fine or the shot ran too long. Grind a little coarser and pull a shorter shot, aiming for about twenty-five seconds." },
  { "id": "n24", "title": "Espresso channeling",
    "body": "Channeling is when water jets through a crack in the puck, leaving a dense, unevenly packed bed and under-extracted coffee. Distribute the grounds evenly and tamp level to prevent it." },
  { "id": "n25", "title": "Espresso grind size",
    "body": "Grind size is the biggest lever on extraction. A finer grind slows the flow and raises extraction; a coarser grind speeds it up. Change it in small steps and taste between adjustments." },
  { "id": "n26", "title": "Espresso dose and ratio",
    "body": "A double shot is about eighteen grams of coffee in and thirty-six grams of liquid out, a 1:2 ratio. Weigh both the dose and the output so shots stay repeatable." },
  { "id": "n27", "title": "Espresso water temperature",
    "body": "Brew temperature near ninety-three degrees Celsius suits medium roasts. Hotter water extracts more and can push a shot toward bitter; cooler water can leave it sour and weak." }
```

- [ ] **Step 2: Add the 2 dense-near-duplicate queries**

In `queries.json`, add a comma after the `negative` query's `}` is NOT wanted — instead insert these objects **before** the `negative` query line (keep the array valid). Recommended: place them right after the `coverage` query block:

```json
  { "query": "my espresso tastes bitter and harsh", "category": "dense-near-duplicate", "set": "dev", "relevant_ids": ["n23"],
    "grades": { "n23": 3, "n7": 1, "n24": 1, "n25": 1, "n26": 1, "n27": 1 } },
  { "query": "water is jetting through my espresso puck unevenly", "category": "dense-near-duplicate", "set": "holdout", "relevant_ids": ["n24"],
    "grades": { "n24": 3, "n7": 1, "n23": 1, "n25": 1, "n26": 1, "n27": 1 } },
```

Note: `n24` is the direct answer (grade 3) for query 2 and a same-topic sibling (grade 1) for query 1 — that is correct; a note can be the answer to one query and context for another.

- [ ] **Step 3: Run loader + harness tests**

Run: `cd src-tauri && cargo test -p raki-eval --lib`
Expected: PASS. (`ordering_categories_carry_grades` is satisfied because both new queries carry `grades`; ids resolve; corpus is 27 notes.)

- [ ] **Step 4: Regenerate the snapshot so the deterministic gate stays green**

The committed `snapshot.json` now lacks the new queries, so the keyword snapshot gate would report "new query — re-baseline". Regenerate it (real model; cached):

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report -- --write --date=2026-06-06`
Then: `cd src-tauri && cargo test -p raki-eval --test eval_gate keyword_snapshot_is_deterministic`
Expected: PASS (snapshot now contains the new queries; keyword is deterministic).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/fixtures/corpus.json src-tauri/crates/raki-eval/fixtures/queries.json docs/eval/snapshot.json docs/eval/baseline.md
git commit -m "3b: add dense-near-duplicate espresso cluster and queries"
```

---

## Task 2: paraphrase-distractor — sourdough rise + distractors + 2 queries

**Files:**
- Modify: `src-tauri/crates/raki-eval/fixtures/corpus.json`
- Modify: `src-tauri/crates/raki-eval/fixtures/queries.json`

- [ ] **Step 1: Add the true note and the generic-baking distractor**

In `corpus.json`, append after `n27` (add a comma after n27's `}`):

```json
  { "id": "n28", "title": "Sourdough proofing and rise",
    "body": "If the loaf bakes up dense and barely rises, the dough was under-proofed or the starter was weak. Let it ferment longer until it grows by about half and springs back slowly from a poke." },
  { "id": "n29", "title": "Banana bread recipe",
    "body": "Mash three ripe bananas, stir in flour, sugar, an egg, melted butter, and baking soda, then bake at one hundred seventy-five degrees for an hour until a skewer comes out clean." }
```

(The distractor for these queries is whichever note looks lexically closer than the true answer: `n24` "Espresso channeling" mentions a *dense* puck; `n29` is *bread*. Both are grade 0 — unlabeled — so a vector that ranks them above `n28` loses recall/nDCG.)

- [ ] **Step 2: Add the 2 paraphrase-distractor queries**

In `queries.json`, after the dense-near-duplicate queries from Task 1:

```json
  { "query": "why is my bread dense and didn't rise", "category": "paraphrase-distractor", "set": "dev", "relevant_ids": ["n28"],
    "grades": { "n28": 3, "n1": 1 } },
  { "query": "my sourdough came out flat and gummy", "category": "paraphrase-distractor", "set": "holdout", "relevant_ids": ["n28"],
    "grades": { "n28": 3, "n1": 1 } },
```

(`n1` "Sourdough starter schedule" is a grade-1 sibling — a weak starter is a cause of poor rise. `n28` is the direct answer.)

- [ ] **Step 3: Run loader + harness tests**

Run: `cd src-tauri && cargo test -p raki-eval --lib`
Expected: PASS.

- [ ] **Step 4: Regenerate the snapshot**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report -- --write --date=2026-06-06`
Then: `cd src-tauri && cargo test -p raki-eval --test eval_gate keyword_snapshot_is_deterministic`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/fixtures/corpus.json src-tauri/crates/raki-eval/fixtures/queries.json docs/eval/snapshot.json docs/eval/baseline.md
git commit -m "3b: add paraphrase-distractor sourdough notes and queries"
```

---

## Task 3: polysemy — literal rust + 2 queries

**Files:**
- Modify: `src-tauri/crates/raki-eval/fixtures/corpus.json`
- Modify: `src-tauri/crates/raki-eval/fixtures/queries.json`

- [ ] **Step 1: Add the literal-rust note**

In `corpus.json`, append after `n29` (add a comma after n29's `}`); this is the last note, so it ends the array (no trailing comma):

```json
  { "id": "n30", "title": "Removing rust from tools",
    "body": "To get rust off garden tools or a bike chain, soak the metal in white vinegar overnight, scrub it with steel wool, then dry it and wipe on a little oil to stop the corrosion coming back." }
```

- [ ] **Step 2: Add the 2 polysemy queries (binary — no grades)**

In `queries.json`, after the paraphrase-distractor queries:

```json
  { "query": "how do I get rid of rust", "category": "polysemy", "set": "dev", "relevant_ids": ["n30"] },
  { "query": "cleaning rust off garden tools", "category": "polysemy", "set": "holdout", "relevant_ids": ["n30"] },
```

(`polysemy` is NOT in the ordering list, so binary/no-grades is correct. The wrong sense — `n3`/`n10`, the Rust *language* notes — are the trap.)

- [ ] **Step 3: Run loader + harness tests**

Run: `cd src-tauri && cargo test -p raki-eval --lib`
Expected: PASS. Corpus is now 30 notes, 25 queries.

- [ ] **Step 4: Regenerate the snapshot**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report -- --write --date=2026-06-06`
Then: `cd src-tauri && cargo test -p raki-eval --test eval_gate keyword_snapshot_is_deterministic`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/fixtures/corpus.json src-tauri/crates/raki-eval/fixtures/queries.json docs/eval/snapshot.json docs/eval/baseline.md
git commit -m "3b: add polysemy rust note and queries"
```

---

## Task 4: Loader invariants for the new categories

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`

- [ ] **Step 1: Add a new-category presence invariant + bump the corpus threshold**

In `src/lib.rs` `#[cfg(test)] mod tests`, add this test, and change the existing `assert!(corpus.len() >= 20, ...)` line to `>= 28`:

```rust
    #[test]
    fn new_failure_mode_categories_present() {
        let cats: std::collections::HashSet<String> =
            load_queries().into_iter().map(|q| q.category).collect();
        for c in ["dense-near-duplicate", "paraphrase-distractor", "polysemy"] {
            assert!(cats.contains(c), "missing mandatory 3b category {c}");
        }
    }
```

Change (in `fixtures_parse_and_reference_real_corpus_ids`):

```rust
        assert!(corpus.len() >= 28, "need a non-trivial corpus");
```

- [ ] **Step 2: Assert the new ordering categories produce nDCG (harness test)**

In `harness_scores_every_category_with_fake_embedder`, after the existing `lexical-cluster` nDCG assertions, add:

```rust
        for cat in ["dense-near-duplicate", "paraphrase-distractor"] {
            let q = run.per_query.iter().find(|q| q.category == cat)
                .unwrap_or_else(|| panic!("missing {cat}"));
            assert!(q.keyword.scores.ndcg.is_some(), "{cat} must produce nDCG (graded)");
        }
```

- [ ] **Step 3: Run the loader + harness tests**

Run: `cd src-tauri && cargo test -p raki-eval --lib`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-eval/src/lib.rs
git commit -m "3b: loader invariants for new failure-mode categories"
```

---

## Task 5: Author-once real-model measurement (record, don't tune)

**Files:**
- Modify: `docs/eval/judge-log.md` (measurement record only)

This is the single honest read of the real model on the finished corpus. **Do not change the corpus based on what you see here.**

- [ ] **Step 1: Run the real-model report and read the table**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report`
Read the OVERALL line and the per-category rows. Note: vector overall recall@3, and which categories have vector recall@3 < 1.00 and/or low nDCG.

- [ ] **Step 2: Record the measurement against the D1 expectation**

Append to `docs/eval/judge-log.md` (below the existing content) a short record — the actual numbers, and the verdict:

```markdown
## 2026-06-06 — 3b author-once measurement

Real model, k=3, corpus = 30 notes / 25 queries. Vector OVERALL recall@3 = <X.XX>.
Categories where vector recall@3 < 1.00: <list>. New ordering-category nDCG (vec):
dense-near-duplicate <X.XX>, paraphrase-distractor <X.XX>.

D1 expectation (recall@3 < ~0.85 AND ≥3 categories < 1.0): **<met / under-shot>**.
Per D1/D9: the note set is fixed; no notes were added to chase the number. If under-shot,
this is the recorded finding (these modes did not break bge-small as hypothesized — Slice 4
may have limited headroom on this set), not a failure of the slice.
```

Fill `<...>` with the actual observed values.

- [ ] **Step 3: Commit**

```bash
git add docs/eval/judge-log.md
git commit -m "3b: record author-once real-model measurement"
```

---

## Task 6: Label audit of the new labels (D7)

**Files:**
- Modify (if fixes found): `src-tauri/crates/raki-eval/fixtures/queries.json`
- Modify: `docs/eval/judge-log.md`

- [ ] **Step 1: Phase-1 corpus-based review**

Re-read each of the 6 new queries against the note bodies (`corpus.json`). Confirm from content alone: is `n23` the bitter answer? Is `n28` the rise answer and `n29`/`n24` genuinely *not* relevant (grade 0)? Is `n30` the only literal-rust note? Confirm grades are coarse-defensible (3 = direct, 1 = same-topic sibling).

- [ ] **Step 2: Phase-2 pooled-candidate review (uses the per-query dump)**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report` and inspect the dev per-query dump for the new queries. Any surfaced note **not** labeled that is genuinely relevant on its content gets added (flagged `pool-surfaced`); do not add a note merely because retrieval ranked it. (Do **not** re-label to change a score.)

- [ ] **Step 3: Independent second-judge cross-check (blind subagent)**

Dispatch a subagent given ONLY `corpus.json` + `queries.json` (the 6 new queries) + `docs/eval/labeling-rubric.md` to independently propose relevant ids/grades. Reconcile; the human is final judge. Record the outcome in `docs/eval/judge-log.md`:

```markdown
| 2026-06-06 | (6 new 3b queries) | <none / list changes> | blind subagent cross-check of dense-near-duplicate / paraphrase-distractor / polysemy labels; <agreement summary> | judged |
```

- [ ] **Step 4: Apply any agreed fixes and re-sync**

If any label changed, edit `queries.json`, then regenerate the snapshot:
Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report -- --write --date=2026-06-06`
Run: `cd src-tauri && cargo test -p raki-eval`
Expected: PASS (loader invariants + deterministic gate green).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/fixtures/queries.json docs/eval/judge-log.md docs/eval/snapshot.json docs/eval/baseline.md
git commit -m "3b: audit new labels (pooling + blind cross-check)"
```

---

## Task 7: One-time documented downward re-baseline of the floors (D8)

**Files:**
- Modify: `src-tauri/crates/raki-eval/tests/eval_gate.rs`

The snapshot is already current (Tasks 1–3/6). This task recalibrates the **floor constants** once, from the Task 5 measurement, and extends the nDCG floor to the new graded categories.

- [ ] **Step 1: Read the final per-method numbers**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report`
From the OVERALL line read: keyword/vector/hybrid recall@3 and MAP@3; coverage recall@10 (vec/hyb); and the nDCG of `lexical-cluster`, `dense-near-duplicate`, `paraphrase-distractor` (vec/hyb).

- [ ] **Step 2: Recalibrate the floor constants down to ~0.10 below observed**

In `tests/eval_gate.rs`, replace the constants block and its comment. Set each floor to roughly `floor(observed*100 - 10)/100`, never above observed. Example shape (use YOUR observed numbers, not these literals):

```rust
// Re-baselined for hardened corpus 3b, 2026-06-06: floors ~0.10 below the observed
// OVERALL on the 30-note / 25-query corpus. One-time downward recalibration (the test got
// harder, ADR-0005 §ratchet); up-only ratcheting resumes. Per-query snapshots guard rot.
const KW_RECALL_FLOOR: f64 = 0.55;   // observed kw non-coverage recall ~0.65 → floor 0.55
const KW_MAP_FLOOR: f64 = 0.55;
const VEC_RECALL_FLOOR: f64 = 0.75;  // observed vec non-coverage recall ~0.85 → floor 0.75
const VEC_MAP_FLOOR: f64 = 0.75;
const HYB_RECALL_FLOOR: f64 = 0.75;
const HYB_MAP_FLOOR: f64 = 0.75;
const COVERAGE_RECALL10_FLOOR: f64 = 0.85; // adjust to ~0.10 below observed coverage recall@10
const ORDERING_NDCG_FLOOR: f64 = 0.60;     // ~0.10 below the MIN observed nDCG across the 3 ordering cats
```

If a floor would land *above* its observed value, lower it further — a floor must pass on the committed baseline. Do not raise any floor here (that is a later ratchet).

- [ ] **Step 3: Extend the ordering-nDCG floor to the 3 graded categories**

In `tests/eval_gate.rs`, change the nDCG floor loop filter:

```rust
    const ORDERING: &[&str] = &["lexical-cluster", "dense-near-duplicate", "paraphrase-distractor"];
    for q in pq.iter().filter(|q| ORDERING.contains(&q.category.as_str())) {
```

(Replace the existing `for q in pq.iter().filter(|q| q.category == "lexical-cluster")` line.)

- [ ] **Step 4: Run the full gate (real model)**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate -- --ignored`
Expected: PASS. If a floor fails, it was set too high — lower it to ~0.10 below the actual observed value (never tune the corpus to meet a floor).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/tests/eval_gate.rs
git commit -m "3b: one-time downward re-baseline of per-method floors"
```

---

## Task 8: Verification + Definition of Done

- [ ] **Step 1: Full deterministic sweep**

Run: `cd src-tauri && cargo test --workspace --exclude raki && cargo fmt --check && cargo clippy --workspace --exclude raki --all-targets -- -D warnings`
Expected: all pass, clean. (Mirrors the required CI job; `--exclude raki` avoids the GUI crate's GTK deps.)

- [ ] **Step 2: Real-model gate green**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate -- --ignored`
Expected: PASS (snapshots + recalibrated floors).

- [ ] **Step 3: Artifacts consistent & idempotent**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report -- --write --date=2026-06-06` then (repo root) `git status --short docs/eval`.
Expected: clean (regeneration is idempotent on this environment).

- [ ] **Step 4: DoD against the spec**

D1 (author-once, recorded measurement) ✓ Task 5 · D2 (3 modes) ✓ Tasks 1–3 · D3 (fixed scope) ✓ Tasks 1–3 · D4 (grades on ordering, binary polysemy) ✓ Tasks 1–3 · D5 (dev/holdout per category) ✓ Tasks 1–3 · D6 (rubric labeling) ✓ Task 6 · D7 (second-judge audit) ✓ Task 6 · D8 (one-time downward re-baseline) ✓ Task 7 · D9 (falsification recorded if under-shot) ✓ Task 5. Loader invariants ✓ Task 4.

- [ ] **Step 5: Frontend untouched**

Run (repo root): `bun run typecheck && bun run build`
Expected: green (sanity only).

---

## Self-Review

**Spec coverage:** D1 → Tasks 1–3 (fixed set) + Task 5 (one-shot measurement, no tuning). D2 → Tasks 1–3 (dense-near-duplicate, paraphrase-distractor, polysemy). D3 → 8 notes / 6 queries, fixed. D4 → grades on the two ordering categories (enforced by `ordering_categories_carry_grades`), polysemy binary. D5 → each new category has 1 dev + 1 holdout. D6/D7 → Task 6. D8 → Task 7 (floors only, once; snapshot kept current along the way). D9 → Task 5's recorded-finding branch. Loader invariants → Task 4.

**Placeholder scan:** the only intentional placeholders are the `<X.XX>` measurement values in Task 5 (genuinely unknown until the run) and the example floor literals in Task 7 (explicitly "use YOUR observed numbers"). No code step is left vague.

**Type/consistency:** new categories `dense-near-duplicate` / `paraphrase-distractor` / `polysemy` are spelled identically in fixtures (Tasks 1–3), the loader invariant (Task 4), the harness assertion (Task 4), and the gate's `ORDERING` list (Task 7). `n7`/`n1` reused as graded siblings exist in the current corpus. Note ids `n23`–`n30` are sequential and unique. The snapshot-stays-green strategy (regen per authoring task) is consistent with the 3a-ii deterministic gate semantics.

**Known sequencing note:** floors in `eval_gate.rs` remain at the old (0.90) values during Tasks 1–6; the `real_model_gate` is `#[ignore]`d so it does not run in `cargo test`, keeping those commits green. Task 7 is the single point where floors move — the one documented downward re-baseline (D8).

---

## Execution Handoff

(Presented to the user after saving.)
