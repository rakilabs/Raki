# Eval Protocol Hardening — 3a-i (Measurement Core + Audit) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the retrieval eval a semantically-explicit, deterministic, per-query-visible, audited instrument — true qrels (no relevance cap), graded nDCG for ordering, a coverage metric, a dev/holdout split, keyword tie-break determinism, and a TREC-style labeling protocol + audit of the current labels.

**Architecture:** Almost all change lives in `raki-eval` (fixtures + harness + report) plus one 3-character determinism fix in `raki-storage`'s keyword SQL and two new docs under `docs/eval/`. `run_eval` is refactored to compute **per-query** results first, then aggregate — this is the foundation both the audit (this plan) and the snapshot gate (3a-ii) build on. No retrieval-ranking logic changes; the eval keeps using the production `search`/`vector_search`/`hybrid_search` adapters.

**Tech Stack:** Rust, `raki-eval` (driver crate), `raki-retrieval` metrics (`recall_at_k`, `average_precision_at_k`, `reciprocal_rank`, `ndcg_at_k`), `raki-storage` (FTS5), `serde`/`serde_json`, `tokio`, fastembed (real model only in `--ignored` runs).

**Spec:** `docs/superpowers/specs/2026-06-05-eval-protocol-hardening-3a-design.md` (D1–D12). This plan covers D1, D2 (recall@3/MAP/MRR/nDCG/coverage), D3 (set field), D4 (graded nDCG), D6 (rubric), D7 (judge-log), D11 (keyword determinism + shared path), D12 (audit). 3a-ii covers D5 (snapshots), D8 (per-method floors), D9 (CI), D10 (artifact).

**Current state (verified):** 22 notes, 18 queries. `MethodScores { recall, map, mrr }`; `CategoryReport { category, scored, keyword, vector, hybrid }`; `Report { k, overall_keyword, overall_vector, overall_hybrid, by_category, unscored_categories }`; `EvalQuery { query, category, relevant_ids, grades }` (grades already present, unused). `run_eval(embedder, k) -> Result<Report, DomainError>` scores kw/vec/hyb via `to_fixture(search/vector_search/hybrid_search)` and `push_scores` (recall/ap/rr). Categories: lexical-overlap(3), semantic-paraphrase(3), buried-fact-in-long-note(2), multi-relevant(3), named-entity(2), temporal(1), messy(1), lexical-cluster(2), negative(1).

---

## File Structure

```
raki-eval/src/lib.rs       MODIFY  set field; QueryResult/EvalRun; per-query refactor; nDCG; coverage; invariants
raki-eval/src/main.rs      MODIFY  per-query inspection dump (dev); 3-method + nDCG/coverage columns
raki-eval/fixtures/queries.json  MODIFY  add `set`; grades on lexical-cluster; one coverage query
raki-storage/src/search.rs MODIFY  ORDER BY rank, note_id (keyword tie determinism) + test
docs/eval/labeling-rubric.md  CREATE  the protocol (D6)
docs/eval/judge-log.md        CREATE  disagreement + audit record (D7/D12)
```

---

## Task 1: `set` field + dev/holdout split + loader invariants

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`
- Modify: `src-tauri/crates/raki-eval/fixtures/queries.json`

- [ ] **Step 1: Add the `set` field to `EvalQuery`**

In `raki-eval/src/lib.rs`, extend the struct (keep existing fields/order; add `set`):

```rust
#[derive(Debug, Deserialize)]
pub struct EvalQuery {
    pub query: String,
    pub category: String,
    /// "dev" (used while tuning) or "holdout" (run only by the gate). Defaults to "dev".
    #[serde(default = "default_set")]
    pub set: String,
    #[serde(default)]
    pub relevant_ids: Vec<String>,
    /// Optional graded relevance (fixture id → grade). Absent ⇒ binary; nDCG dormant.
    #[serde(default)]
    pub grades: HashMap<String, f64>,
}

fn default_set() -> String {
    "dev".to_string()
}
```

- [ ] **Step 2: Tag the 18 queries with `set` (≈⅓ holdout, spread across categories)**

In `raki-eval/fixtures/queries.json`, add `"set"` to each query. Hold out one query from each of several categories so the holdout exercises varied behavior (the rest are `dev`). Use these holdout queries: `"rust borrow checker"` (lexical-overlap), `"keeping tomato plants alive in the heat"` (semantic-paraphrase), `"how should I pay when I arrive at the inn"` (buried-fact), `"core Rust language concepts for beginners"` (multi-relevant), `"E0502"` (lexical-cluster), `"when are quarterly estimated taxes due"` (temporal). All others get `"set": "dev"`. Example (apply the pattern to every entry):

```json
  { "query": "sourdough starter feeding", "category": "lexical-overlap", "set": "dev", "relevant_ids": ["n1"] },
  { "query": "rust borrow checker", "category": "lexical-overlap", "set": "holdout", "relevant_ids": ["n3"] },
```

- [ ] **Step 3: Write the loader invariant test**

In `raki-eval/src/lib.rs` `#[cfg(test)] mod tests`, extend `fixtures_parse_and_reference_real_corpus_ids` (or add a new test) asserting the invariants. Add:

```rust
    #[test]
    fn every_query_has_a_valid_set_and_resolvable_ids() {
        let corpus = load_corpus();
        let queries = load_queries();
        let ids: std::collections::HashSet<&str> = corpus.iter().map(|n| n.id.as_str()).collect();
        for q in &queries {
            assert!(
                q.set == "dev" || q.set == "holdout",
                "query {:?} has invalid set {:?}",
                q.query, q.set
            );
            for r in &q.relevant_ids {
                assert!(ids.contains(r.as_str()), "{:?} references unknown id {r}", q.query);
            }
            for gid in q.grades.keys() {
                assert!(ids.contains(gid.as_str()), "{:?} grades unknown id {gid}", q.query);
            }
        }
        assert!(queries.iter().any(|q| q.set == "holdout"), "need a holdout set");
        assert!(queries.iter().any(|q| q.set == "dev"), "need a dev set");
    }
```

- [ ] **Step 4: Run the loader tests**

Run: `cd src-tauri && cargo test -p raki-eval fixtures_parse every_query_has`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/src/lib.rs src-tauri/crates/raki-eval/fixtures/queries.json
git commit -m "Add dev/holdout set field to eval queries with loader invariants"
```

---

## Task 2: Keyword tie-break determinism (`raki-storage`)

**Files:**
- Modify: `src-tauri/crates/raki-storage/src/search.rs`

Rationale: snapshots must not flap on bm25 ties. The harness receives ordered ids without scores, so it cannot re-sort; the deterministic fix belongs in the SQL. This also gives eval/production parity for keyword ties (the D11 follow-up, done here for keyword since it's free).

- [ ] **Step 1: Write the failing test**

In `raki-storage/src/search.rs` tests, add a test that two notes with identical keyword relevance return in a stable, id-ordered sequence:

```rust
    #[tokio::test]
    async fn ties_break_by_note_id_deterministically() {
        let db = Database::open_in_memory().unwrap();
        let repo = SqliteNoteRepository::new(db.clone());
        let index = SqliteKeywordIndex::new(db);
        // Two notes that match "apple" identically (same single term, same length).
        let a = Note { id: NoteId::parse("00000000-0000-7000-8000-000000000001").unwrap(),
            title: "apple".into(), body: "x".into(), created_at: 1, updated_at: 1, deleted_at: None, version: 1 };
        let b = Note { id: NoteId::parse("00000000-0000-7000-8000-000000000002").unwrap(),
            title: "apple".into(), body: "x".into(), created_at: 1, updated_at: 1, deleted_at: None, version: 1 };
        repo.upsert(&b).await.unwrap();
        repo.upsert(&a).await.unwrap();
        let hits = index.query("apple", 10).await.unwrap();
        let ids: Vec<String> = hits.into_iter().map(|h| h.source_id).collect();
        assert_eq!(ids, vec![a.id.to_string(), b.id.to_string()], "ties ordered by note_id");
    }
```

(Use `raki_domain::{Note, NoteId}` in the test imports if not already present.)

- [ ] **Step 2: Run to verify it fails (or is order-undefined)**

Run: `cd src-tauri && cargo test -p raki-storage ties_break_by_note_id`
Expected: FAIL or flaky — order not guaranteed without the secondary key.

- [ ] **Step 3: Add the secondary sort key**

In `raki-storage/src/search.rs`, change the keyword query's `ORDER BY` to add `note_id`:

```rust
                    "SELECT note_id, bm25(notes_fts) AS score
                     FROM notes_fts
                     WHERE notes_fts MATCH ?1
                     ORDER BY score, note_id
                     LIMIT ?2",
```

- [ ] **Step 4: Run to verify it passes**

Run: `cd src-tauri && cargo test -p raki-storage`
Expected: PASS (including the new test and all existing).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-storage/src/search.rs
git commit -m "Deterministic keyword tie-break: ORDER BY score, note_id"
```

---

## Task 3: Per-query detail refactor (`run_eval` → `EvalRun`)

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`

This is the foundation: compute per-query, per-method results first, then aggregate. `run_eval` returns both the aggregate `Report` and the per-query detail. Metrics (nDCG, coverage) and the audit/snapshot build on `per_query`.

- [ ] **Step 1: Add the per-query result types**

In `raki-eval/src/lib.rs`, near the other public structs, add:

```rust
/// One method's outcome for one query: the ranked fixture ids (top-k) and the metrics.
#[derive(Debug, Clone)]
pub struct MethodResult {
    pub ranked: Vec<String>, // fixture ids, best-first, truncated to k
    pub scores: MethodScores,
}

/// Per-query detail for every method — the substrate for the audit (3a-i) and the
/// per-query snapshot gate (3a-ii).
#[derive(Debug, Clone)]
pub struct QueryResult {
    pub query: String,
    pub category: String,
    pub set: String,
    pub keyword: MethodResult,
    pub vector: MethodResult,
    pub hybrid: MethodResult,
}

/// Everything one eval run produces.
#[derive(Debug, Clone)]
pub struct EvalRun {
    pub report: Report,
    pub per_query: Vec<QueryResult>,
}
```

- [ ] **Step 2: Make `MethodScores` carry nDCG (used in Task 4) and stay `Copy`**

Change `MethodScores` to include nDCG as an `Option<f64>` aggregate (None when no graded queries):

```rust
#[derive(Debug, Clone, Copy, Default)]
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

- [ ] **Step 3: Refactor `run_eval` to build per-query results, then aggregate**

Replace the body of `run_eval` so it returns `EvalRun`. Keep index construction identical (same adapters). Compute each method's ranked ids once, build a `QueryResult`, then aggregate. Constants and full body:

```rust
const COVERAGE_K: usize = 10;

pub async fn run_eval(embedder: Arc<dyn EmbeddingProvider>, k: usize) -> Result<EvalRun, DomainError> {
    let corpus = load_corpus();
    let queries = load_queries();

    let db = Database::open_in_memory()?;
    let repo = SqliteNoteRepository::new(db.clone());
    let keyword = SqliteKeywordIndex::new(db.clone());
    let vectors = SqliteVectorIndex::new(db.clone());

    let mut fixture_of: HashMap<String, String> = HashMap::new();
    for cn in &corpus {
        const DUMMY_EPOCH_MS: i64 = 1000;
        let note = Note::new(cn.title.clone(), cn.body.clone(), DUMMY_EPOCH_MS);
        let uuid = note.id.to_string();
        repo.upsert(&note).await?;
        let doc = format!("{}\n\n{}", cn.title, cn.body);
        let emb = embedder.embed(std::slice::from_ref(&doc)).await?;
        let emb = emb.first().ok_or_else(|| {
            DomainError::Provider("embedder returned empty batch for single doc".to_string())
        })?;
        vectors.upsert(&uuid, emb).await?;
        fixture_of.insert(uuid, cn.id.clone());
    }

    let mut per_query: Vec<QueryResult> = Vec::new();
    let mut unscored: HashSet<String> = HashSet::new();

    for q in &queries {
        if q.relevant_ids.is_empty() {
            unscored.insert(q.category.clone());
            continue;
        }
        let relevant: HashSet<String> = q.relevant_ids.iter().cloned().collect();
        let cov_k = if q.category == "coverage" { COVERAGE_K } else { k };

        let kw = to_fixture(&search(&keyword, &q.query, cov_k.max(k)).await?, &fixture_of);
        let vc = to_fixture(
            &vector_search(&vectors, embedder.as_ref(), &q.query, cov_k.max(k)).await?,
            &fixture_of,
        );
        let hy = to_fixture(
            &hybrid_search(&keyword, &vectors, embedder.as_ref(), &q.query, cov_k.max(k)).await?,
            &fixture_of,
        );

        per_query.push(QueryResult {
            query: q.query.clone(),
            category: q.category.clone(),
            set: q.set.clone(),
            keyword: MethodResult { scores: score_one(&kw, &relevant, &q.grades, k, q), ranked: truncate(&kw, k) },
            vector: MethodResult { scores: score_one(&vc, &relevant, &q.grades, k, q), ranked: truncate(&vc, k) },
            hybrid: MethodResult { scores: score_one(&hy, &relevant, &q.grades, k, q), ranked: truncate(&hy, k) },
        });
    }

    let report = aggregate(&per_query, &mut unscored, k);
    Ok(EvalRun { report, per_query })
}

fn truncate(ids: &[String], k: usize) -> Vec<String> {
    ids.iter().take(k).cloned().collect()
}
```

- [ ] **Step 4: Add `score_one` (per-query, per-method metrics)**

Add below `run_eval`. It computes recall@3/MAP@3/MRR always, nDCG@k only when grades exist, recall@K_cov only for coverage queries:

```rust
fn score_one(
    ranked: &[String],
    relevant: &HashSet<String>,
    grades: &HashMap<String, f64>,
    k: usize,
    q: &EvalQuery,
) -> MethodScores {
    MethodScores {
        recall: recall_at_k(ranked, relevant, k).unwrap_or(0.0),
        map: average_precision_at_k(ranked, relevant, k).unwrap_or(0.0),
        mrr: reciprocal_rank(ranked, relevant).unwrap_or(0.0),
        ndcg: if grades.is_empty() { None } else { ndcg_at_k(ranked, grades, k) },
        recall_cov: if q.category == "coverage" {
            recall_at_k(ranked, relevant, COVERAGE_K)
        } else {
            None
        },
    }
}
```

Add `ndcg_at_k` to the `raki_retrieval` import line:

```rust
use raki_retrieval::{
    average_precision_at_k, hybrid_search, ndcg_at_k, recall_at_k, reciprocal_rank, search,
    vector_search,
};
```

- [ ] **Step 5: Add `aggregate` (rebuild `Report` from `per_query`)**

Replace the old inline aggregation. Add:

```rust
fn aggregate(per_query: &[QueryResult], unscored: &mut HashSet<String>, k: usize) -> Report {
    use std::collections::BTreeMap;
    let mut cats: BTreeMap<String, Vec<&QueryResult>> = BTreeMap::new();
    for qr in per_query {
        cats.entry(qr.category.clone()).or_default().push(qr);
    }
    let mut by_category = Vec::new();
    for (cat, qrs) in &cats {
        by_category.push(CategoryReport {
            category: cat.clone(),
            scored: qrs.len(),
            keyword: mean_scores(qrs.iter().map(|q| q.keyword.scores)),
            vector: mean_scores(qrs.iter().map(|q| q.vector.scores)),
            hybrid: mean_scores(qrs.iter().map(|q| q.hybrid.scores)),
        });
    }
    let overall_keyword = mean_scores(per_query.iter().map(|q| q.keyword.scores));
    let overall_vector = mean_scores(per_query.iter().map(|q| q.vector.scores));
    let overall_hybrid = mean_scores(per_query.iter().map(|q| q.hybrid.scores));
    let mut unscored_categories: Vec<String> = unscored.drain().collect();
    unscored_categories.sort();
    Report { k, overall_keyword, overall_vector, overall_hybrid, by_category, unscored_categories }
}

fn mean_scores(it: impl Iterator<Item = MethodScores> + Clone) -> MethodScores {
    let v: Vec<MethodScores> = it.collect();
    let n = v.len().max(1) as f64;
    let opt_mean = |f: &dyn Fn(&MethodScores) -> Option<f64>| {
        let present: Vec<f64> = v.iter().filter_map(f).collect();
        if present.is_empty() { None } else { Some(present.iter().sum::<f64>() / present.len() as f64) }
    };
    MethodScores {
        recall: v.iter().map(|s| s.recall).sum::<f64>() / n,
        map: v.iter().map(|s| s.map).sum::<f64>() / n,
        mrr: v.iter().map(|s| s.mrr).sum::<f64>() / n,
        ndcg: opt_mean(&|s| s.ndcg),
        recall_cov: opt_mean(&|s| s.recall_cov),
    }
}
```

Delete the now-unused `push_scores`, `overall`, and `ScoreAcc` helpers. Keep `to_fixture` and `mean` (if `mean` is now unused, delete it too — let clippy guide).

- [ ] **Step 6: Update the harness tests to the new return type**

In `raki-eval/src/lib.rs` tests, `run_eval(...)` now returns `EvalRun`. Update `harness_scores_every_category_with_fake_embedder` to read `.report` and add a per-query assertion:

```rust
        let run = run_eval(Arc::new(FakeEmbeddingProvider::new(384)), 3).await.unwrap();
        let report = &run.report;
        assert_eq!(report.k, 3);
        assert!(!run.per_query.is_empty());
        assert!(run.per_query.iter().all(|q| q.keyword.ranked.len() <= 3));
```

(Keep the existing category/range assertions, referencing `report`.)

- [ ] **Step 7: Build + test**

Run: `cd src-tauri && cargo test -p raki-eval && cargo clippy -p raki-eval --all-targets -- -D warnings`
Expected: PASS, clean. Fix any unused-import/dead-code the refactor leaves.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/crates/raki-eval/src/lib.rs
git commit -m "Refactor run_eval to per-query EvalRun; add nDCG/coverage score fields"
```

---

## Task 4: Graded nDCG on the ordering category (`lexical-cluster`)

**Files:**
- Modify: `src-tauri/crates/raki-eval/fixtures/queries.json`
- Modify: `src-tauri/crates/raki-eval/src/lib.rs` (loader invariant)

The `lexical-cluster` queries (`E0599`→n21, `E0502`→n20) sit in a 5-note near-duplicate error cluster (n9, n19, n20, n21, n22) — the existing ordering category. Add grades so nDCG measures "does the exact code outrank its siblings."

- [ ] **Step 1: Add grades to the lexical-cluster queries**

In `queries.json`, for `E0599` and `E0502` add `grades` (direct answer = 3, sibling error notes = 1):

```json
  { "query": "E0599", "category": "lexical-cluster", "set": "dev", "relevant_ids": ["n21"],
    "grades": { "n21": 3, "n9": 1, "n19": 1, "n20": 1, "n22": 1 } },
  { "query": "E0502", "category": "lexical-cluster", "set": "holdout", "relevant_ids": ["n20"],
    "grades": { "n20": 3, "n9": 1, "n19": 1, "n21": 1, "n22": 1 } },
```

- [ ] **Step 2: Add the "ordering categories must carry grades" invariant**

In `raki-eval/src/lib.rs` loader test, add:

```rust
    #[test]
    fn ordering_categories_carry_grades() {
        const ORDERING: &[&str] = &["lexical-cluster", "dense-near-duplicate", "paraphrase-distractor"];
        for q in load_queries() {
            if ORDERING.contains(&q.category.as_str()) {
                assert!(!q.grades.is_empty(), "ordering query {:?} must carry grades", q.query);
            }
        }
    }
```

- [ ] **Step 3: Assert nDCG is computed for graded queries (harness test)**

In the harness test, after building `run`, add:

```rust
        let cluster = run.per_query.iter().find(|q| q.category == "lexical-cluster").unwrap();
        assert!(cluster.keyword.scores.ndcg.is_some(), "graded query must produce nDCG");
        let lex = run.report.by_category.iter().find(|c| c.category == "lexical-cluster").unwrap();
        assert!(lex.keyword.ndcg.is_some(), "lexical-cluster aggregate carries nDCG");
```

- [ ] **Step 4: Test**

Run: `cd src-tauri && cargo test -p raki-eval`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/fixtures/queries.json src-tauri/crates/raki-eval/src/lib.rs
git commit -m "Grade the lexical-cluster ordering category and gate nDCG presence"
```

---

## Task 5: Coverage metric + one coverage query

**Files:**
- Modify: `src-tauri/crates/raki-eval/fixtures/queries.json`
- Modify: `src-tauri/crates/raki-eval/src/lib.rs` (loader invariant)

`recall@K_cov` is already computed in `score_one`/`aggregate` (Task 3). This task exercises it with a genuine "find all my Rust notes" query over the existing corpus.

- [ ] **Step 1: Add the coverage query (labeled per the rubric)**

Rationale (rubric Phase-1, document-based): every note whose primary topic is the Rust language or a Rust compiler error is relevant to "all my rust notes" — n3 (ownership), n10 (module paths), n9/n19/n20/n21/n22 (cargo build errors). Add to `queries.json`:

```json
  { "query": "all my rust programming notes", "category": "coverage", "set": "dev",
    "relevant_ids": ["n3", "n10", "n9", "n19", "n20", "n21", "n22"] },
```

- [ ] **Step 2: Add the coverage invariant**

In the loader test:

```rust
    #[test]
    fn coverage_queries_have_many_relevant() {
        for q in load_queries() {
            if q.category == "coverage" {
                assert!(q.relevant_ids.len() >= 4, "coverage query {:?} should have a broad answer set", q.query);
            }
        }
    }
```

- [ ] **Step 3: Assert recall_cov is computed for coverage queries (harness test)**

```rust
        let cov = run.per_query.iter().find(|q| q.category == "coverage").unwrap();
        assert!(cov.vector.scores.recall_cov.is_some(), "coverage query must produce recall@K_cov");
```

- [ ] **Step 4: Test**

Run: `cd src-tauri && cargo test -p raki-eval`
Expected: PASS (recall@3 for this query is naturally low — 7 answers, k=3, ceiling 0.43 — which is correct, not a failure).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/fixtures/queries.json src-tauri/crates/raki-eval/src/lib.rs
git commit -m "Add a coverage query and exercise recall@K_cov"
```

---

## Task 6: Per-query inspection dump in `eval-report` (dev set)

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/main.rs`

- [ ] **Step 1: Update `eval-report` to the new return type and add a per-query dump**

In `raki-eval/src/main.rs`, replace `main` so it (a) handles `run_eval` returning `EvalRun`, (b) prints the per-category table including nDCG/coverage columns where present, and (c) prints a per-query dump for the **dev** set only (holdout is not inspected — D3). Full `main`:

```rust
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let k = 3;
    let run = run_eval(embedder, k).await?;
    let report = &run.report;

    println!("Retrieval eval @ k={k}  (R=recall  M=MAP  N=nDCG  Cov=recall@10)\n");
    for c in &report.by_category {
        row(&format!("{} (n={})", c.category, c.scored), c.keyword, c.vector, c.hybrid);
    }
    println!("{}", "-".repeat(96));
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
    Ok(())
}

fn fmt_opt(o: Option<f64>) -> String {
    o.map(|v| format!("{v:.2}")).unwrap_or_else(|| "  - ".to_string())
}

fn row(label: &str, kw: MethodScores, vc: MethodScores, hy: MethodScores) {
    println!(
        "{label:<24} | kw R{:.2} M{:.2} N{} | vec R{:.2} M{:.2} N{} | hyb R{:.2} M{:.2} N{}",
        kw.recall, kw.map, fmt_opt(kw.ndcg),
        vc.recall, vc.map, fmt_opt(vc.ndcg),
        hy.recall, hy.map, fmt_opt(hy.ndcg),
    );
}
```

- [ ] **Step 2: Build the binary**

Run: `cd src-tauri && cargo build -p raki-eval --bin eval-report && cargo clippy -p raki-eval --all-targets -- -D warnings`
Expected: builds, clean.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/raki-eval/src/main.rs
git commit -m "eval-report: per-query dev dump and nDCG/coverage columns"
```

---

## Task 7: Labeling rubric + judge-log docs (D6 / D7)

**Files:**
- Create: `docs/eval/labeling-rubric.md`
- Create: `docs/eval/judge-log.md`

- [ ] **Step 1: Write the rubric**

Create `docs/eval/labeling-rubric.md` capturing D6 verbatim-in-spirit: the relevance definition; binary-vs-graded (3 = direct answer, 1 = genuinely-related sibling); no `|relevant|` cap; the two-phase pooling process (Phase 1 corpus-based before retrieval, Phase 2 pooled candidates judged document-based and flagged `pool-surfaced`); continuous pooling for future methods; author discipline (include realistic hard modes, never bend labels toward a planned fix); provenance `judged`; the dev/holdout meaning; the ordering-category grades requirement; the coverage definition + `coverage_k=10` rationale.

- [ ] **Step 2: Seed the judge-log**

Create `docs/eval/judge-log.md` with a header and an empty table the audit (Task 8) fills:

```markdown
# Eval label judge log

Records second-judge disagreements and pool-surfaced label changes (rubric Phase 2 / D7).
The subagent cross-check is a *consistency check*, not an independent judge; the human is the
final judge.

| date | query | change | reason | provenance |
|------|-------|--------|--------|------------|
```

- [ ] **Step 3: Commit**

```bash
git add docs/eval/labeling-rubric.md docs/eval/judge-log.md
git commit -m "Add eval labeling rubric and judge-log"
```

---

## Task 8: Label audit of the current set (D12)

**Files:**
- Modify (if fixes found): `src-tauri/crates/raki-eval/fixtures/queries.json`
- Modify: `docs/eval/judge-log.md`

This task is a *process* task: audit the 18 queries' labels against the 22-note corpus using the rubric, with an independent consistency cross-check.

- [ ] **Step 1: Phase-1 corpus-based review**

Re-read each query and its `relevant_ids`/`grades` against the note bodies in `corpus.json`. For each, confirm the label is justified from note content alone. Note any suspected miss/over-label.

- [ ] **Step 2: Phase-2 pooled candidates (uses the dump, document-based decisions)**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report` (real model; if offline, record as deferred). For each dev query, inspect the per-query dump: any surfaced note **not** already labeled is read in `corpus.json` and judged on its content. If genuinely relevant, add it (flag `pool-surfaced` in the judge-log). Do **not** add a note merely because retrieval ranked it.

- [ ] **Step 3: Independent consistency cross-check (blind subagent)**

Dispatch a subagent given ONLY `corpus.json` + `queries.json` + the rubric (no knowledge of Slices 4–5) to independently propose relevant ids/grades for each query. Reconcile disagreements per the rubric; record each in `docs/eval/judge-log.md` with the resolution. The human is the final judge.

- [ ] **Step 4: Apply fixes + re-run invariants**

Apply any agreed label fixes to `queries.json`. Run: `cd src-tauri && cargo test -p raki-eval`
Expected: PASS (invariants still hold).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/fixtures/queries.json docs/eval/judge-log.md
git commit -m "Audit current eval labels (pooling + consistency cross-check)"
```

---

## Task 9: 3a-i verification + Definition of Done

- [ ] **Step 1: Full sweep**

Run: `cd src-tauri && cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass, clean.

- [ ] **Step 2: Real-model report reads sanely**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report`
Expected: 3-method table with nDCG on `lexical-cluster`, a `coverage` row, the per-query dev dump, and the `negative` category unscored. If offline, record as deferred.

- [ ] **Step 3: Confirm DoD against the spec (3a-i portion)**

D1 (no cap) ✓ Task 3/5 · D2 (recall@3/MAP/MRR/nDCG/coverage) ✓ Tasks 3–5 · D3 (set) ✓ Task 1 · D4 (graded nDCG gated-by-presence) ✓ Task 4 · D6 (rubric) ✓ Task 7 · D7 (judge-log) ✓ Tasks 7–8 · D11 (keyword determinism + shared path) ✓ Task 2 · D12 (audit) ✓ Task 8. Deferred to 3a-ii: D5 (snapshots), D8 (per-method floors), D9 (CI), D10 (artifact).

- [ ] **Step 4: Frontend untouched**

Run (repo root): `bun run typecheck && bun run build`
Expected: green (no frontend change, sanity only).

---

## Self-Review

**Spec coverage (3a-i):** D1 → Task 3/5 (true qrels, no cap). D2 → Tasks 3–5 (metric set incl. coverage). D3 → Task 1 (set). D4 → Task 4 (graded nDCG). D6 → Task 7 (rubric). D7 → Tasks 7–8 (judge-log + cross-check). D11 → Task 2 + Task 3 (keyword determinism; eval keeps production adapters). D12 → Task 8 (audit). D5/D8/D9/D10 are 3a-ii, named in the goal.

**Placeholder scan:** none — every code step has complete code; Task 7 (docs) and Task 8 (audit) are inherently prose/process and say exactly what to produce.

**Type consistency:** `EvalRun { report, per_query }` (Task 3) returned by `run_eval`, consumed by `main.rs` (Task 6) and the audit (Task 8). `MethodScores` gains `ndcg`/`recall_cov: Option<f64>` (Task 3), produced by `score_one`, aggregated by `mean_scores`, read by `row`/tests. `QueryResult.set` (Task 3) from `EvalQuery.set` (Task 1). `COVERAGE_K`/`coverage` category consistent across Tasks 3 and 5.

**Known deviation from spec, documented:** D5's "eval imposes a stable total order (by score)" is infeasible in the harness (it receives ordered ids without scores); implemented instead as the keyword SQL secondary sort (Task 2) + pinned-environment determinism. Vector cross-environment tie determinism remains the tracked retrieval follow-up (3a-ii records the environment in the artifact).

---

## Execution Handoff

(Presented to the user after saving.)
