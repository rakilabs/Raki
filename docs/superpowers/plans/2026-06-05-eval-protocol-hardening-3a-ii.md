# Eval Protocol Hardening — 3a-ii (Snapshots, Floors, Artifact, CI) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the eval its *teeth*: a committed per-query regression snapshot the gate checks exactly (D5), per-method floors that stop diluting coverage into precision (D8), a reproducible baseline artifact (D10), and a CI workflow that runs the deterministic gate always and the real-model gate where the model is available (D9).

**Architecture:** Everything lives in `raki-eval` plus two new committed files under `docs/eval/` and one `.github/workflows/eval.yml`. No retrieval/storage/app code changes. The substrate already exists: `run_eval` returns `EvalRun { report, per_query }` where `per_query: Vec<QueryResult>` carries each method's ranked top-k ids and metrics. 3a-ii (a) makes those types serde-serializable, (b) serializes `per_query` to a committed `snapshot.json`, (c) adds a pure comparison function the gate uses to assert no query regresses, (d) replaces the single `overall_hybrid` floor with per-method floors that floor coverage on recall@10 (not recall@3), and (e) writes a human-readable baseline artifact.

**Tech Stack:** Rust, `raki-eval` (driver crate), `serde`/`serde_json`, `tokio`, fastembed (real model only in `--ignored` runs and the non-blocking CI job), GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-06-05-eval-protocol-hardening-3a-design.md` (D1–D12). This plan covers **D5** (per-query snapshots), **D8** (per-method floors), **D9** (CI), **D10** (reproducible artifact). 3a-i delivered D1–D4, D6, D7, D11, D12.

**Current state (verified):**
- `run_eval(embedder, k) -> Result<EvalRun, DomainError>`; `EvalRun { report: Report, per_query: Vec<QueryResult> }`.
- `QueryResult { query, category, set, keyword: MethodResult, vector: MethodResult, hybrid: MethodResult }`.
- `MethodResult { ranked: Vec<String>, scores: MethodScores }`; `MethodScores { recall, map, mrr, ndcg: Option<f64>, recall_cov: Option<f64> }` — derives `Debug, Clone, Copy, Default` (no serde yet).
- `eval-report` prints a per-category table (R/M/N/Cov) + dev-only per-query dump; no file output.
- `eval_gate.rs`: one `#[ignore]`d test flooring `report.overall_hybrid.recall`/`.map` at `0.90` (this includes the coverage query at recall@3 0.43 — the dilution D8 fixes).
- Real-model OVERALL (k=3): kw R0.82 M0.82 N0.73 Cov0.29 · vec/hyb R0.97 M0.97 N0.92 Cov1.00. Coverage query alone: kw Cov0.29, vec/hyb Cov1.00.
- 22 notes, 19 queries (18 scored, 1 coverage, `negative` unscored). `docs/eval/{labeling-rubric,judge-log}.md` exist. No `.github/`. No `sha2` dep (house change-detector is FNV-1a in `raki-storage/src/hash.rs`).

---

## File Structure

```
raki-eval/src/lib.rs       MODIFY  serde derives on snapshot types; Method enum + accessor;
                                   snapshot_regressions(); load_snapshot(); fixtures_fingerprint()
raki-eval/src/main.rs      MODIFY  --write mode: emit docs/eval/snapshot.json + docs/eval/baseline.md
raki-eval/tests/eval_gate.rs  MODIFY  deterministic keyword-snapshot gate (not ignored) +
                                      real-model gate (ignored): vec/hyb snapshots + per-method floors
docs/eval/snapshot.json    CREATE  committed per-query baseline (gate reads this) — canonical, undated
docs/eval/baseline.md      CREATE  human-readable D10 artifact — canonical, undated
.github/workflows/eval.yml CREATE  D9 CI: deterministic job (required) + real-model job (non-blocking)
```

### Two documented deviations from the spec (same spirit, more practical)

1. **Canonical filenames, not dated.** D5/D10 name files `snapshot-<date>.json` / `baseline-<date>.md`. A gate cannot hard-code a date, so this plan uses canonical `docs/eval/snapshot.json` and `docs/eval/baseline.md`, **overwritten on an explicit reviewed re-baseline**. Git history provides the dated versions; the re-baseline shows up as a reviewable diff — exactly D5's "changes only via an explicit, reviewed re-baseline." The artifact still records its generation date in a `Date:` field.
2. **FNV-1a fixture fingerprint, not sha256.** D10 says "sha256 of corpus.json + queries.json." The codebase deliberately avoids a crypto-hash dependency and uses FNV-1a as a deterministic change-detector (`hash.rs` says so explicitly). The fingerprint's job here is identical — "does this baseline match the fixtures it was generated from" — so this plan computes FNV-1a over the embedded fixture text rather than adding `sha2`. Recorded as a non-security fingerprint in the artifact.

### One design choice worth stating up front

D5 wants ordering regressions caught ("the direct-answer rank must not increase"). This plan enforces that via **nDCG@3 non-decrease** on graded (ordering) categories: demoting the grade-3 direct answer below a grade-1 sibling *strictly lowers* nDCG, so the gated-metric-non-decrease check already catches exactly the meaningful ordering regressions — while two equal-grade siblings swapping (not a real regression) correctly does not trip it. This keeps the comparison numeric and exact, and avoids threading `grades` into `QueryResult`. Snapshot exactness is **environment-pinned** (D5/D10/D11): keyword is model-independent and deterministically id-sorted (3a-i), so its snapshot is checked in the always-on deterministic gate; vector/hybrid exactness holds on a pinned environment and is checked in the real-model gate (non-blocking in CI where the environment isn't guaranteed).

---

## Task 1: Serde-serializable snapshot types + `Method` accessor + helpers

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`

- [ ] **Step 1: Import `Serialize` alongside `Deserialize`**

At the top of `lib.rs`, change the serde import:

```rust
use serde::{Deserialize, Serialize};
```

- [ ] **Step 2: Add serde derives to the three snapshot types**

`snapshot.json` is `per_query` serialized, so `QueryResult`, `MethodResult`, and `MethodScores` must round-trip. Add `Serialize, Deserialize` to each (keep existing derives):

```rust
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct MethodScores {
    pub recall: f64,
    pub map: f64,
    pub mrr: f64,
    /// Mean nDCG@k over graded queries only; None when none are graded.
    pub ndcg: Option<f64>,
    /// Mean recall@K_cov over coverage queries only; None when none are coverage.
    pub recall_cov: Option<f64>,
}
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodResult {
    pub ranked: Vec<String>, // fixture ids, best-first, truncated to k
    pub scores: MethodScores,
}
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub query: String,
    pub category: String,
    pub set: String,
    pub keyword: MethodResult,
    pub vector: MethodResult,
    pub hybrid: MethodResult,
}
```

- [ ] **Step 3: Add the `Method` enum and a `QueryResult::method` accessor**

Below `QueryResult`, add:

```rust
/// Which retrieval method a snapshot check targets. The deterministic gate checks
/// `Keyword` only (model-independent); the real-model gate checks all three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Keyword,
    Vector,
    Hybrid,
}

impl QueryResult {
    pub fn method(&self, m: Method) -> &MethodResult {
        match m {
            Method::Keyword => &self.keyword,
            Method::Vector => &self.vector,
            Method::Hybrid => &self.hybrid,
        }
    }
}
```

- [ ] **Step 4: Add `fixtures_fingerprint` (FNV-1a over the embedded fixtures)**

Near the `load_corpus`/`load_queries` functions (which reference `CORPUS_JSON`/`QUERIES_JSON`), add:

```rust
/// FNV-1a 64-bit over the embedded fixture text — a deterministic change-detector
/// (house style, see raki-storage/src/hash.rs; not a security hash). Lets a reviewer
/// confirm a committed baseline matches the fixtures it was generated from.
pub fn fixtures_fingerprint() -> String {
    let mut h: u64 = 0xcbf29ce484222325; // FNV offset basis
    for b in CORPUS_JSON.bytes().chain(QUERIES_JSON.bytes()) {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3); // FNV prime
    }
    format!("{h:016x}")
}
```

- [ ] **Step 5: Build**

Run: `cd src-tauri && cargo build -p raki-eval`
Expected: builds (serde derives resolve; `Method` unused-warning is fine until Task 2).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-eval/src/lib.rs
git commit -m "Make eval snapshot types serializable; add Method accessor and fixture fingerprint"
```

---

## Task 2: `snapshot_regressions` comparison + `load_snapshot` + unit tests

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`

This is the heart of D5: a pure function comparing a fresh run's `per_query` against a committed baseline, returning one message per regression. Unit-tested with hand-built results (no model, fast).

- [ ] **Step 1: Write the failing tests first**

In `raki-eval/src/lib.rs` `#[cfg(test)] mod tests`, add (these reference `snapshot_regressions`, which does not exist yet):

```rust
    fn mk(query: &str, category: &str, kw_recall: f64, ndcg: Option<f64>) -> QueryResult {
        let scores = MethodScores { recall: kw_recall, map: kw_recall, mrr: kw_recall, ndcg, recall_cov: None };
        let mr = MethodResult { ranked: vec!["n1".into()], scores };
        QueryResult {
            query: query.into(), category: category.into(), set: "dev".into(),
            keyword: mr.clone(), vector: mr.clone(), hybrid: mr,
        }
    }

    #[test]
    fn identical_runs_have_no_regression() {
        let base = vec![mk("q1", "lexical-overlap", 1.0, None)];
        let cur = base.clone();
        assert!(snapshot_regressions(&cur, &base, &[Method::Keyword]).is_empty());
    }

    #[test]
    fn a_metric_drop_is_a_regression() {
        let base = vec![mk("q1", "lexical-overlap", 1.0, None)];
        let cur = vec![mk("q1", "lexical-overlap", 0.5, None)];
        let r = snapshot_regressions(&cur, &base, &[Method::Keyword]);
        assert_eq!(r.len(), 1, "recall drop must be reported once");
        assert!(r[0].contains("recall@3"));
    }

    #[test]
    fn an_ndcg_drop_on_ordering_is_a_regression() {
        let base = vec![mk("E0599", "lexical-cluster", 1.0, Some(0.92))];
        let cur = vec![mk("E0599", "lexical-cluster", 1.0, Some(0.73))];
        let r = snapshot_regressions(&cur, &base, &[Method::Keyword]);
        assert!(r.iter().any(|m| m.contains("nDCG@3")), "demoted direct answer must trip nDCG");
    }

    #[test]
    fn an_improvement_is_not_a_regression() {
        let base = vec![mk("q1", "lexical-overlap", 0.5, None)];
        let cur = vec![mk("q1", "lexical-overlap", 1.0, None)];
        assert!(snapshot_regressions(&cur, &base, &[Method::Keyword]).is_empty());
    }

    #[test]
    fn a_missing_or_new_query_demands_rebaseline() {
        let base = vec![mk("q1", "lexical-overlap", 1.0, None)];
        let cur = vec![mk("q2", "lexical-overlap", 1.0, None)];
        let r = snapshot_regressions(&cur, &base, &[Method::Keyword]);
        assert_eq!(r.len(), 2, "q1 absent + q2 new ⇒ two re-baseline messages");
        assert!(r.iter().any(|m| m.contains("absent")));
        assert!(r.iter().any(|m| m.contains("not in baseline")));
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cd src-tauri && cargo test -p raki-eval snapshot 2>&1 | tail -5`
Expected: compile error — `snapshot_regressions` not found.

- [ ] **Step 3: Implement `snapshot_regressions`**

Add to `lib.rs` (non-test scope), below the `Method` impl:

```rust
/// Per-metric float tolerance — these metrics are deterministic functions of rank
/// positions, so any real drop exceeds this; the epsilon only absorbs float noise.
const METRIC_EPS: f64 = 1e-9;

/// Compare a fresh run's per-query results against a committed baseline snapshot.
/// Returns one human-readable message per regression; empty ⇒ no regression.
/// A gated metric (recall@3, MAP@3, MRR, and — where both runs have them — nDCG@3 and
/// recall@10) dropping below baseline is a regression. A query present in one run but
/// not the other demands an explicit re-baseline. `methods` selects which retrieval
/// methods to check: `[Keyword]` for the deterministic gate, all three for real-model.
pub fn snapshot_regressions(
    current: &[QueryResult],
    baseline: &[QueryResult],
    methods: &[Method],
) -> Vec<String> {
    let base: HashMap<&str, &QueryResult> =
        baseline.iter().map(|q| (q.query.as_str(), q)).collect();
    let cur: HashMap<&str, &QueryResult> =
        current.iter().map(|q| (q.query.as_str(), q)).collect();

    let mut out = Vec::new();
    for b in baseline {
        if !cur.contains_key(b.query.as_str()) {
            out.push(format!(
                "query {:?} in baseline but absent from current run (re-baseline)",
                b.query
            ));
        }
    }
    for c in current {
        let Some(b) = base.get(c.query.as_str()) else {
            out.push(format!("query {:?} not in baseline (new query — re-baseline)", c.query));
            continue;
        };
        for &m in methods {
            let (cm, bm) = (c.method(m), b.method(m));
            let mut drop = |name: &str, cv: f64, bv: f64| {
                if cv + METRIC_EPS < bv {
                    out.push(format!(
                        "{:?} [{:?}] {name} {cv:.4} < baseline {bv:.4}",
                        c.query, m
                    ));
                }
            };
            drop("recall@3", cm.scores.recall, bm.scores.recall);
            drop("MAP@3", cm.scores.map, bm.scores.map);
            drop("MRR", cm.scores.mrr, bm.scores.mrr);
            if let (Some(cv), Some(bv)) = (cm.scores.ndcg, bm.scores.ndcg) {
                drop("nDCG@3", cv, bv);
            }
            if let (Some(cv), Some(bv)) = (cm.scores.recall_cov, bm.scores.recall_cov) {
                drop("recall@10", cv, bv);
            }
        }
    }
    out
}
```

- [ ] **Step 4: Add `load_snapshot` (runtime read, CARGO_MANIFEST_DIR-relative)**

Reading at runtime (not `include_str!`) avoids coupling compilation to the committed file. Add to `lib.rs`:

```rust
/// Path to the committed per-query baseline, relative to this crate. `raki-eval` lives at
/// `src-tauri/crates/raki-eval`, so the repo root is three parents up.
pub fn snapshot_path() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/eval/snapshot.json")
}

/// Load the committed baseline snapshot. Panics with a clear message if missing —
/// the gate cannot run without a committed baseline (generate it with `eval-report --write`).
pub fn load_snapshot() -> Vec<QueryResult> {
    let path = snapshot_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("read snapshot {}: {e} — run `eval-report --write` first", path.display()));
    serde_json::from_str(&text).expect("snapshot.json is valid")
}
```

- [ ] **Step 5: Run to verify the comparison tests pass**

Run: `cd src-tauri && cargo test -p raki-eval snapshot regression rebaseline improvement metric 2>&1 | tail -8`
Expected: the five new tests PASS. (`load_snapshot` is untested here — exercised end-to-end in Task 5 after the file exists.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-eval/src/lib.rs
git commit -m "Add snapshot_regressions comparison, load_snapshot, and unit tests"
```

---

## Task 3: `eval-report --write` emits `snapshot.json` + `baseline.md` (D10)

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/main.rs`

`eval-report` keeps printing to stdout; with `--write` it also serializes `per_query` to `snapshot.json` and writes the human-readable artifact. An optional `--date=YYYY-MM-DD` stamps the artifact (the binary cannot invent a stable date).

- [ ] **Step 1: Rewrite `main.rs` to add write mode**

Replace the current `main` (keep `fmt_opt`/`row` as-is from 3a-i). Full new `main` plus a `write_artifacts` helper:

```rust
use std::sync::Arc;

use raki_ai::FastEmbedProvider;
use raki_eval::{fixtures_fingerprint, run_eval, EvalRun, MethodScores};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let write = args.iter().any(|a| a == "--write");
    let date = args
        .iter()
        .find_map(|a| a.strip_prefix("--date="))
        .unwrap_or("undated")
        .to_string();

    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let model_id = embedder.model_id();
    let k = 3;
    let run = run_eval(embedder, k).await?;
    let report = &run.report;

    println!("Retrieval eval @ k={k}  (R=recall  M=MAP  N=nDCG  Cov=recall@10)\n");
    for c in &report.by_category {
        row(&format!("{} (n={})", c.category, c.scored), c.keyword, c.vector, c.hybrid);
    }
    println!("{}", "-".repeat(120));
    row("OVERALL", report.overall_keyword, report.overall_vector, report.overall_hybrid);

    println!("\nPer-query (dev set only):");
    for q in run.per_query.iter().filter(|q| q.set == "dev") {
        println!("  [{}] {:?}", q.category, q.query);
        println!("    kw  {:?}", q.keyword.ranked);
        println!("    vec {:?}", q.vector.ranked);
        println!("    hyb {:?}", q.hybrid.ranked);
    }
    if !report.unscored_categories.is_empty() {
        println!("\nunscored (need score threshold): {:?}", report.unscored_categories);
    }

    if write {
        write_artifacts(&run, &model_id, &date)?;
    }
    Ok(())
}

/// Repo-root `docs/eval` dir, relative to this crate (src-tauri/crates/raki-eval).
fn eval_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/eval")
}

fn write_artifacts(run: &EvalRun, model_id: &str, date: &str) -> std::io::Result<()> {
    let dir = eval_dir();
    std::fs::create_dir_all(&dir)?;

    // D5: per-query snapshot the gate reads. Pretty-printed for reviewable diffs.
    let snap = serde_json::to_string_pretty(&run.per_query).expect("serialize per_query");
    std::fs::write(dir.join("snapshot.json"), snap + "\n")?;

    // D10: human-readable baseline artifact.
    std::fs::write(dir.join("baseline.md"), baseline_md(run, model_id, date))?;
    eprintln!("wrote {}/snapshot.json and {}/baseline.md", dir.display(), dir.display());
    Ok(())
}

fn baseline_md(run: &EvalRun, model_id: &str, date: &str) -> String {
    let r = &run.report;
    let mut s = String::new();
    s.push_str("# Eval baseline artifact\n\n");
    s.push_str(&format!("Date: {date}\n\n"));
    s.push_str("Reproducible baseline for the retrieval eval (D10). The gate floors cite these\n");
    s.push_str("numbers; the per-query lock is `snapshot.json` (D5).\n\n");
    s.push_str("## Environment\n\n");
    s.push_str(&format!("- Model id: `{model_id}`\n"));
    s.push_str("- Embedding dimension: 384 (fixed by bge-small-en-v1.5; pinned by model id)\n");
    s.push_str(&format!("- Platform: {} / {}\n", std::env::consts::OS, std::env::consts::ARCH));
    s.push_str(&format!("- Fixture fingerprint (FNV-1a, non-security): `{}`\n", fixtures_fingerprint()));
    s.push_str("- Pinned library versions: see committed `src-tauri/Cargo.lock` (fastembed, ort/onnxruntime, rusqlite/SQLite bundled, sqlite-vec).\n");
    s.push_str(&format!("- k = {}; coverage_k = 10.\n", r.k));
    s.push_str("- Command: `cargo run -p raki-eval --bin eval-report -- --write --date=<date>`\n");
    s.push_str("- Deterministic ordering: keyword is id-sorted in SQL (`ORDER BY score, note_id`);\n");
    s.push_str("  vector/hybrid order is deterministic on this pinned environment (see D5/D11).\n\n");
    s.push_str("`coverage_k = 10` rationale: top-10 spans ~45% of the 22-note corpus — a sensible\n");
    s.push_str("\"find most\" horizon. Revisit when the corpus grows (3b).\n\n");
    s.push_str("## Per-category (kw / vec / hyb)\n\n");
    s.push_str("| category | n | kw R/M/N/Cov | vec R/M/N/Cov | hyb R/M/N/Cov |\n");
    s.push_str("|---|---|---|---|---|\n");
    for c in &r.by_category {
        s.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            c.category, c.scored, cell(c.keyword), cell(c.vector), cell(c.hybrid)
        ));
    }
    s.push_str(&format!(
        "| **OVERALL** |  | {} | {} | {} |\n\n",
        cell(r.overall_keyword), cell(r.overall_vector), cell(r.overall_hybrid)
    ));
    s.push_str(&format!("Unscored categories: {:?}\n", r.unscored_categories));
    s
}

fn cell(m: MethodScores) -> String {
    format!("{:.2}/{:.2}/{}/{}", m.recall, m.map, fmt_opt(m.ndcg), fmt_opt(m.recall_cov))
}
```

Note: the embedding dimension is recorded as 384 (bge-small-en-v1.5's fixed width — the model id pins it); keep the literal to avoid an extra probe embed. The `Report` import is used by `baseline_md`'s signature via `run.report`; ensure `Report` stays imported (remove it if clippy flags it unused — it is referenced only through `EvalRun`, so the `use ... Report` may be dropped; let clippy guide).

- [ ] **Step 2: Build + clippy**

Run: `cd src-tauri && cargo build -p raki-eval --bin eval-report && cargo clippy -p raki-eval --all-targets -- -D warnings`
Expected: builds, clean. Remove any unused import clippy flags.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/raki-eval/src/main.rs
git commit -m "eval-report --write: emit snapshot.json and baseline.md artifacts"
```

---

## Task 4: Generate and commit the baseline + snapshot (real model, one-time)

**Files:**
- Create: `docs/eval/snapshot.json`
- Create: `docs/eval/baseline.md`

This is a one-time generation step the developer runs and reviews. It requires the real model (network on first run; cached after).

- [ ] **Step 1: Generate the artifacts**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report -- --write --date=2026-06-05`
Expected: prints the table and `wrote .../docs/eval/snapshot.json and .../docs/eval/baseline.md`.

- [ ] **Step 2: Review the artifacts by eye**

Open `docs/eval/snapshot.json` — confirm 18 scored queries each have `keyword`/`vector`/`hybrid` with `ranked` (≤3 ids — coverage included; its recall@10 lives in `scores.recall_cov`, not in `ranked` length) and `scores`. Open `docs/eval/baseline.md` — confirm the environment block, the fixture fingerprint, and the per-category table read sanely (lexical-cluster carries nDCG; coverage carries Cov). The `negative` query is correctly absent from the snapshot (unscored).

- [ ] **Step 3: Commit the baseline**

```bash
git add docs/eval/snapshot.json docs/eval/baseline.md
git commit -m "Commit eval baseline snapshot and artifact (2026-06-05)"
```

---

## Task 5: Gate rework — deterministic keyword snapshot + real-model snapshots + per-method floors (D5/D8)

**Files:**
- Modify: `src-tauri/crates/raki-eval/tests/eval_gate.rs`

Two tests. The **deterministic** one (NOT ignored) runs under the fake embedder and asserts the keyword snapshot doesn't regress — keyword is model-independent, so this is exact and runs in normal `cargo test`. The **real-model** one (ignored) asserts the vector/hybrid snapshots don't regress and that per-method floors hold, with coverage floored on recall@10 (resolving the dilution).

- [ ] **Step 1: Replace `eval_gate.rs` wholesale**

```rust
//! The retrieval regression gate. Two layers (spec D5 + D8):
//!  - `keyword_snapshot_is_deterministic` runs the fake embedder and checks the KEYWORD
//!    per-query snapshot. Keyword retrieval is real FTS5 and model-independent, so this is
//!    exact and runs in ordinary `cargo test` (and as the required CI job).
//!  - `real_model_gate` (#[ignore]) runs the real bge model and checks the vector/hybrid
//!    per-query snapshots plus per-method average floors. Run explicitly:
//!      cargo test -p raki-eval --test eval_gate -- --ignored
//!
//! The snapshots (D5) are the teeth; the floors (D8) are a coarse smoke alarm. Coverage is
//! floored on recall@10 (its proper metric), never averaged into the recall@3 floor.

use std::sync::Arc;

use raki_ai::{FakeEmbeddingProvider, FastEmbedProvider};
use raki_eval::{load_snapshot, run_eval, snapshot_regressions, Method, QueryResult};

// Per-method recall@3 / MAP@3 floors over NON-COVERAGE scored queries, calibrated
// 2026-06-05 ~0.10 below observed (kw ~0.85, vec/hyb ~1.00 non-coverage). Ratchet UP only.
const KW_RECALL_FLOOR: f64 = 0.75;
const KW_MAP_FLOOR: f64 = 0.75;
const VEC_RECALL_FLOOR: f64 = 0.90;
const VEC_MAP_FLOOR: f64 = 0.90;
const HYB_RECALL_FLOOR: f64 = 0.90;
const HYB_MAP_FLOOR: f64 = 0.90;
// Coverage recall@10 floor (vec/hyb observed 1.00). nDCG@3 floor for ordering (observed 0.92 vec/hyb).
const COVERAGE_RECALL10_FLOOR: f64 = 0.85;
const ORDERING_NDCG_FLOOR: f64 = 0.80;

fn mean(it: impl Iterator<Item = f64>) -> f64 {
    let v: Vec<f64> = it.collect();
    v.iter().sum::<f64>() / v.len().max(1) as f64
}

/// recall@3 mean for `method` over scored, non-coverage queries.
fn noncov_recall(per_query: &[QueryResult], m: Method) -> f64 {
    mean(per_query.iter().filter(|q| q.category != "coverage").map(|q| q.method(m).scores.recall))
}
fn noncov_map(per_query: &[QueryResult], m: Method) -> f64 {
    mean(per_query.iter().filter(|q| q.category != "coverage").map(|q| q.method(m).scores.map))
}

#[tokio::test]
async fn keyword_snapshot_is_deterministic() -> Result<(), Box<dyn std::error::Error>> {
    let run = run_eval(Arc::new(FakeEmbeddingProvider::new(384)), 3).await?;
    let baseline = load_snapshot();
    let regressions = snapshot_regressions(&run.per_query, &baseline, &[Method::Keyword]);
    assert!(regressions.is_empty(), "keyword regressions:\n{}", regressions.join("\n"));
    Ok(())
}

#[tokio::test]
#[ignore = "runs the real bge model (network + native runtime); run with --ignored"]
async fn real_model_gate() -> Result<(), Box<dyn std::error::Error>> {
    let run = run_eval(Arc::new(FastEmbedProvider::try_new()?), 3).await?;
    let baseline = load_snapshot();

    // D5: no vector/hybrid query regresses (keyword already covered deterministically).
    let regressions = snapshot_regressions(&run.per_query, &baseline, &[Method::Vector, Method::Hybrid]);
    assert!(regressions.is_empty(), "vec/hyb regressions:\n{}", regressions.join("\n"));

    // D8: per-method floors over non-coverage queries.
    let pq = &run.per_query;
    for (m, rf, mf) in [
        (Method::Keyword, KW_RECALL_FLOOR, KW_MAP_FLOOR),
        (Method::Vector, VEC_RECALL_FLOOR, VEC_MAP_FLOOR),
        (Method::Hybrid, HYB_RECALL_FLOOR, HYB_MAP_FLOOR),
    ] {
        let (r, mp) = (noncov_recall(pq, m), noncov_map(pq, m));
        assert!(r >= rf, "{m:?} non-coverage recall {r:.3} below floor {rf}");
        assert!(mp >= mf, "{m:?} non-coverage MAP {mp:.3} below floor {mf}");
    }

    // D8: coverage floored on recall@10 (vec + hyb — the production-facing methods).
    let cov = pq.iter().find(|q| q.category == "coverage").expect("a coverage query");
    for m in [Method::Vector, Method::Hybrid] {
        let c = cov.method(m).scores.recall_cov.expect("coverage recall@10 present");
        assert!(c >= COVERAGE_RECALL10_FLOOR, "{m:?} coverage recall@10 {c:.3} below floor {COVERAGE_RECALL10_FLOOR}");
    }

    // D8: ordering categories floored on nDCG@3 (vec + hyb).
    for q in pq.iter().filter(|q| q.category == "lexical-cluster") {
        for m in [Method::Vector, Method::Hybrid] {
            let n = q.method(m).scores.ndcg.expect("ordering nDCG present");
            assert!(n >= ORDERING_NDCG_FLOOR, "{:?} {m:?} nDCG {n:.3} below floor {ORDERING_NDCG_FLOOR}", q.query);
        }
    }
    Ok(())
}
```

- [ ] **Step 2: Verify `FakeEmbeddingProvider` is exported from `raki_ai`**

Run: `cd src-tauri && cargo build -p raki-eval --tests 2>&1 | tail -5`
Expected: builds. (3a-i already used `FakeEmbeddingProvider` in `lib.rs` tests, so the export exists.)

- [ ] **Step 3: Run the deterministic gate (no model)**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate keyword_snapshot_is_deterministic`
Expected: PASS — the fake-run keyword results match the committed snapshot's keyword entries exactly.

- [ ] **Step 4: Run the real-model gate**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate -- --ignored`
Expected: PASS — vec/hyb snapshots match and all floors hold. If a floor is *above* observed, the floor is miscalibrated (lower it to ~0.10 below the baseline value and note it); if a snapshot regresses on first run, the snapshot was generated by a different build — regenerate (Task 4) and re-commit.

- [ ] **Step 5: Red-green check the deterministic gate**

Temporarily edit `docs/eval/snapshot.json`, raising one keyword `recall` above its real value (e.g. a `1.0` where the run yields `0.5`), then run `cargo test -p raki-eval --test eval_gate keyword_snapshot_is_deterministic`.
Expected: FAIL naming that query + `recall@3`. Then `git checkout docs/eval/snapshot.json` to restore, and re-run to confirm PASS. (Proves the gate actually bites.)

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-eval/tests/eval_gate.rs
git commit -m "Gate: deterministic keyword snapshot + real-model snapshots and per-method floors"
```

---

## Task 6: CI workflow (D9)

**Files:**
- Create: `.github/workflows/eval.yml`

Two jobs. **Deterministic** is required and always meaningful (no model). **Real-model** is non-blocking (`continue-on-error`) because GitHub-hosted runners don't guarantee the fastembed cache — honest about where the gate is enforced.

- [ ] **Step 1: Write the workflow**

Create `.github/workflows/eval.yml`:

```yaml
name: eval

on:
  push:
  pull_request:

jobs:
  deterministic:
    name: deterministic gate (required, no model)
    runs-on: ubuntu-latest
    defaults:
      run:
        working-directory: src-tauri
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
      - run: cargo fmt --check
      - run: cargo clippy --workspace --all-targets -- -D warnings
      # Runs loader invariants, the fake-embedder harness, and the keyword per-query
      # snapshot gate — all model-independent, so this job always means something.
      - run: cargo test --workspace

  real-model:
    name: real-model eval (non-blocking)
    runs-on: ubuntu-latest
    continue-on-error: true # GitHub runners don't guarantee the model cache (spec D9)
    defaults:
      run:
        working-directory: src-tauri
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
      # fastembed caches the model under the working dir on first download; cache it
      # across runs. Adjust the path if the cache location differs in CI logs.
      - name: Cache fastembed model
        uses: actions/cache@v4
        with:
          path: src-tauri/.fastembed_cache
          key: fastembed-bge-small-en-v1.5
      - run: cargo test -p raki-eval --test eval_gate -- --ignored
```

- [ ] **Step 2: Lint the YAML locally (syntax only)**

Run: `cd /Users/jayden/Projects/Raki/Raki && python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/eval.yml')); print('yaml ok')"`
Expected: `yaml ok`. (This validates syntax; the workflow itself only runs once pushed to GitHub.)

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/eval.yml
git commit -m "CI: eval workflow — required deterministic gate + non-blocking real-model gate"
```

---

## Task 7: Verification + Definition of Done

- [ ] **Step 1: Full deterministic sweep**

Run: `cd src-tauri && cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass, clean. Confirms the keyword snapshot gate now runs as part of `cargo test --workspace` (it is NOT `#[ignore]`d).

- [ ] **Step 2: Real-model gate green**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate -- --ignored`
Expected: PASS (snapshots + floors). If offline, record as deferred.

- [ ] **Step 3: Artifacts are committed and consistent**

Run: `cd /Users/jayden/Projects/Raki/Raki && git status --short docs/eval`
Expected: clean (no uncommitted snapshot/baseline). Confirm `docs/eval/snapshot.json` and `docs/eval/baseline.md` exist and the baseline's fixture fingerprint matches a fresh `cargo run -p raki-eval --bin eval-report -- --write` (diff should be empty on the same environment).

- [ ] **Step 4: DoD against the spec (3a-ii portion)**

D5 (per-query snapshots — committed `snapshot.json`, gate asserts no query regresses incl. nDCG for ordering) ✓ Tasks 2–5 · D8 (per-method floors; coverage on recall@10, not diluted into recall@3; ordering on nDCG@3; ratchet-up) ✓ Task 5 · D9 (CI: required deterministic job incl. keyword snapshot + non-blocking real-model job) ✓ Task 6 · D10 (reproducible artifact with env, fixture fingerprint, tables, coverage_k rationale) ✓ Tasks 3–4. Documented deviations: canonical (undated) filenames; FNV-1a fingerprint instead of sha256; ordering enforced via nDCG-non-decrease.

- [ ] **Step 5: Frontend untouched**

Run (repo root): `bun run typecheck && bun run build`
Expected: green (sanity only; no frontend change).

---

## Self-Review

**Spec coverage (3a-ii):** D5 → Tasks 1–5 (serializable types, `snapshot_regressions`, committed `snapshot.json`, deterministic + real-model snapshot gates). D8 → Task 5 (per-method floors; coverage floored on recall@10, ordering on nDCG@3 — resolves the 3a-i dilution finding). D9 → Task 6 (`eval.yml`, required deterministic + non-blocking real-model). D10 → Tasks 3–4 (`baseline.md` with environment, fixture fingerprint, tables, coverage_k rationale). D1–D4/D6/D7/D11/D12 were 3a-i.

**Placeholder scan:** none — every code step has complete code. Floor constants are concrete starting values calibrated from the observed 2026-06-05 run, with explicit instructions (Task 5 Step 4) to lower any floor that sits above its freshly-generated baseline.

**Type consistency:** `Method`/`QueryResult::method` (Task 1) used by `snapshot_regressions` (Task 2) and the gate (Task 5). `MethodScores`/`MethodResult`/`QueryResult` gain serde derives (Task 1) consumed by `to_string_pretty(&run.per_query)` (Task 3) and `load_snapshot` (Task 2). `fixtures_fingerprint` (Task 1) used in `baseline_md` (Task 3). `snapshot.json` written in Task 3, committed in Task 4, read by `load_snapshot` in Task 5. Non-coverage floor helpers operate on `per_query` (public since 3a-i).

**Known deviations from spec, documented (in File Structure):** canonical filenames vs dated; FNV-1a fixture fingerprint vs sha256; ordering regressions enforced via nDCG-non-decrease rather than a raw direct-answer-rank comparison. All three preserve the decision's intent.

---

## Execution Handoff

(Presented to the user after saving.)
