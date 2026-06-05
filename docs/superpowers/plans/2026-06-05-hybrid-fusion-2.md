# Hybrid Retrieval (RRF Fusion, Slice #2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fuse keyword (FTS5) and vector (sqlite-vec) rankings via Reciprocal Rank Fusion into a single `hybrid_search`, make it the production `search_notes` path, and harden the eval so the three methods are measurably distinct.

**Architecture:** A `hybrid_search` seam in `raki-retrieval` retrieves a candidate pool from each of `search` (keyword) and `vector_search`, fuses with the existing `reciprocal_rank_fusion`, and returns top-k. The eval harness gains a third method (hybrid) and a hardened, more discriminating corpus (~18 notes, k=3, confusable pairs). `search_notes` is rewired to hybrid; the gate floors hybrid against recalibrated numbers.

**Tech Stack:** `raki-retrieval` (`reciprocal_rank_fusion`, already present) · `raki-eval` · `fastembed` (real model in report/gate).

---

## Spec & Decisions

Extends the approved design `docs/superpowers/specs/2026-06-04-vector-retrieval-eval-design.md` (fusion named there as slice #2). Decisions for this slice:

- **Candidate pool:** hybrid retrieves `pool = max(k, 20)` from each method before fusing (RRF needs depth; the eval corpus is ~18 docs, so 20 covers it).
- **Honest framing:** fusion typically *equals or slightly beats* the best single method; it does not always strictly win. The slice makes the relationship **measurable and the production path**; it does NOT hard-assert `hybrid > max(kw, vec)` (that would be a flaky/gamed gate). The gate floors **hybrid** (recall@k AND MAP@k); the report shows all three per category; a DoD step *observes* fusion vs. max.
- **Corpus hardening (prerequisite):** at 8 docs/k=5 vector scored 1.00 everywhere — no headroom to see fusion. Grow to ~18 notes (incl. confusable pairs that split the methods) and drop eval k to 3, so recall@3 discriminates.
- **Graceful degradation:** if no vectors are indexed yet, `vector_search` returns `[]` and RRF falls back to keyword order naturally. (The fake-fallback-embedder edge — only on offline model-init failure — can inject low-quality vector ranks; acceptable and noted, not handled in v1.)
- **No new ADR:** fusion is mechanism under the existing retrieval design.

## File Structure

```
src-tauri/crates/raki-retrieval/src/search.rs   MODIFY  + hybrid_search seam
src-tauri/crates/raki-retrieval/src/lib.rs       MODIFY  export hybrid_search
src-tauri/crates/raki-eval/fixtures/corpus.json  MODIFY  8 → ~18 notes (confusable pairs)
src-tauri/crates/raki-eval/fixtures/queries.json MODIFY  + keyword-wins / vector-wins queries
src-tauri/crates/raki-eval/src/lib.rs            MODIFY  + hybrid method in run_eval/Report; loader threshold
src-tauri/crates/raki-eval/src/main.rs           MODIFY  print 3 methods; k=3
src-tauri/crates/raki-eval/tests/eval_gate.rs    MODIFY  floor hybrid; k=3; recalibrate
src-tauri/src/state.rs                            MODIFY  AppState += vectors, embedder
src-tauri/src/lib.rs                              MODIFY  pass vectors+embedder to AppState
src-tauri/src/commands/notes.rs                   MODIFY  search_notes → hybrid_search
```

---

## Task 1: `hybrid_search` seam (RRF fusion of keyword + vector)

**Files:**
- Modify: `src-tauri/crates/raki-retrieval/src/search.rs`, `src-tauri/crates/raki-retrieval/src/lib.rs`

- [ ] **Step 1: Write the failing test**

In `src-tauri/crates/raki-retrieval/src/search.rs`, add to the `#[cfg(test)] mod tests` block (reuses the existing `FakeKeyword`, `FakeEmbed`, `FakeVectors`):

```rust
    #[tokio::test]
    async fn hybrid_ranks_shared_item_first() {
        // keyword: [a, b]   vector: [b, c]   → b appears in both, RRF lifts it to #1.
        let keyword = FakeKeyword(vec!["a", "b"]);
        let vectors = FakeVectors(vec!["b", "c"]);
        let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, "q", 3).await.unwrap();
        assert_eq!(ids[0], "b", "item ranked by both methods fuses to the top");
        assert_eq!(ids.len(), 3);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p raki-retrieval hybrid_ranks_shared`
Expected: FAIL — `hybrid_search` not found.

- [ ] **Step 3: Implement the seam**

In `src-tauri/crates/raki-retrieval/src/search.rs`, add the import for the fusion helper and the function (above the test module). Update the top `use` to include `KeywordIndex` (already there) and add the fusion import:

```rust
use crate::fusion::{reciprocal_rank_fusion, DEFAULT_RRF_K};
```

```rust
/// Candidate depth pulled from each retriever before fusing. RRF needs depth to find
/// overlap; the final result is still truncated to `k`.
const HYBRID_CANDIDATE_POOL: usize = 20;

/// Hybrid retrieval: fuse keyword and vector rankings via Reciprocal Rank Fusion and
/// return the top-`k` source ids. Degrades gracefully — if either retriever returns
/// nothing (e.g. no vectors indexed yet), RRF uses whatever ranking is present.
pub async fn hybrid_search(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    query: &str,
    k: usize,
) -> Result<Vec<String>, DomainError> {
    let pool = k.max(HYBRID_CANDIDATE_POOL);
    let kw = search(keyword, query, pool).await?;
    let vec = vector_search(vectors, embedder, query, pool).await?;
    let fused = reciprocal_rank_fusion(&[kw, vec], DEFAULT_RRF_K);
    Ok(fused.into_iter().take(k).map(|(id, _score)| id).collect())
}
```

- [ ] **Step 4: Export it**

In `src-tauri/crates/raki-retrieval/src/lib.rs`, update the search export:

```rust
pub use search::{hybrid_search, search, vector_search};
```

- [ ] **Step 5: Run to verify it passes**

Run: `cd src-tauri && cargo test -p raki-retrieval`
Expected: PASS — including `hybrid_ranks_shared_item_first`.

- [ ] **Step 6: Lint + commit**

Run: `cd src-tauri && cargo clippy -p raki-retrieval --all-targets -- -D warnings`
```bash
git add src-tauri/crates/raki-retrieval/src/search.rs src-tauri/crates/raki-retrieval/src/lib.rs
git commit -m "Add hybrid_search seam fusing keyword and vector via RRF"
```

---

## Task 2: Harden the eval corpus (≈18 notes, confusable pairs)

**Files:**
- Modify: `src-tauri/crates/raki-eval/fixtures/corpus.json`
- Modify: `src-tauri/crates/raki-eval/fixtures/queries.json`
- Modify: `src-tauri/crates/raki-eval/src/lib.rs` (loader-test threshold only)

- [ ] **Step 1: Replace the corpus with the hardened set**

Overwrite `src-tauri/crates/raki-eval/fixtures/corpus.json` (n1–n8 unchanged; n9–n18 added — n9/n10 are a keyword-vs-vector confusable pair, n11 is a vector-wins paraphrase target):

```json
[
  { "id": "n1", "title": "Sourdough starter schedule",
    "body": "Feed the starter once every 24 hours. Discard half, then add equal parts flour and water by weight (1:1:1). It is ready to bake when it doubles within four to six hours." },
  { "id": "n2", "title": "Estimated tax due dates 2026",
    "body": "Federal income tax filing is due April 15. Quarterly estimated payments are due April 15, June 15, September 15, and January 15 of the following year." },
  { "id": "n3", "title": "Rust ownership notes",
    "body": "Every value has one owner. Moves transfer ownership; the borrow checker enforces that references never outlive their referent. Use lifetimes to relate the validity of references." },
  { "id": "n4", "title": "Follow-up with Dr. Patel",
    "body": "Reviewed the knee MRI results. Next appointment in six weeks to decide on physical therapy versus a referral to orthopedics." },
  { "id": "n5", "title": "Summer garden watering",
    "body": "Tomatoes want a deep soak twice a week rather than daily shallow watering. Mulch to keep the roots cool and reduce evaporation in the heat." },
  { "id": "n6", "title": "Japan trip planning 2026",
    "body": "Flights land at Haneda in the evening; take the monorail to the hotel in Shinagawa. Spend three days in Tokyo: Asakusa, the teamLab museum, and a day trip to Nikko. Then the shinkansen to Kyoto for temples and the bamboo grove. Important: the ryokan in Hakone only accepts payment in cash on arrival, so withdraw yen beforehand. Budget for the Hakone free pass and a luggage-forwarding service between cities. End the trip with two relaxed days in Osaka for food." },
  { "id": "n7", "title": "Dialing in espresso",
    "body": "If the shot tastes sour and thin, the grind is too coarse and the extraction too fast; grind finer. Aim for a 1:2 ratio of coffee to liquid over twenty-five to thirty seconds." },
  { "id": "n8", "title": "Password manager migration",
    "body": "Exported the old vault as an encrypted CSV, imported it into Bitwarden, verified a few logins, then securely deleted the export file." },
  { "id": "n9", "title": "Build error E0433 unresolved crate",
    "body": "Cargo reported error E0433: failed to resolve, use of undeclared crate serde. The fix was adding serde to the dependencies table in Cargo.toml, then it compiled." },
  { "id": "n10", "title": "Rust module paths and visibility",
    "body": "Organize code with mod, pub, and use. Paths are relative to the current module; crate:: is the absolute path from the crate root. Re-export with pub use." },
  { "id": "n11", "title": "Trouble sleeping",
    "body": "Avoid screens for an hour before bed, try chamomile tea, and keep the bedroom cool and dark. A consistent wake time helps the most over a few weeks." },
  { "id": "n12", "title": "Tokyo subway tips",
    "body": "Get a Suica card for tap-in travel, avoid the Yamanote line at rush hour, and use a maps app for exact platform numbers and exits." },
  { "id": "n13", "title": "Postgres connection pooling",
    "body": "Use PgBouncer in transaction mode for many short-lived connections. Size the pool near the number of CPU cores; oversizing causes contention, not throughput." },
  { "id": "n14", "title": "Docker compose for local dev",
    "body": "Define each service in docker-compose.yml, use a named volume for the database so data survives restarts, and bring it up with docker compose up in detached mode." },
  { "id": "n15", "title": "Weekly meal prep",
    "body": "Cook grains and roast vegetables on Sunday, portion into containers, and keep sauces separate so nothing turns soggy before midweek." },
  { "id": "n16", "title": "Backpacking gear checklist",
    "body": "Tent, a sleeping bag rated to the season, a water filter, layered clothing for changing weather, and a stove with enough fuel for every meal." },
  { "id": "n17", "title": "Mortgage refinance notes",
    "body": "Compare the new rate against total closing costs; the break-even point is costs divided by monthly savings. Lock the rate before underwriting starts." },
  { "id": "n18", "title": "Learning Spanish with spaced repetition",
    "body": "Review flashcards on an expanding schedule. The algorithm surfaces each card just before you would forget it, which makes review time far more efficient." }
]
```

- [ ] **Step 2: Replace the queries (adds keyword-wins n9 and vector-wins n11 splitters)**

Overwrite `src-tauri/crates/raki-eval/fixtures/queries.json`:

```json
[
  { "query": "sourdough starter feeding", "category": "lexical-overlap", "relevant_ids": ["n1"] },
  { "query": "rust borrow checker", "category": "lexical-overlap", "relevant_ids": ["n3"] },
  { "query": "my espresso is too acidic, what should I change", "category": "semantic-paraphrase", "relevant_ids": ["n7"] },
  { "query": "keeping tomato plants alive in the heat", "category": "semantic-paraphrase", "relevant_ids": ["n5"] },
  { "query": "do I need cash for the ryokan in Hakone", "category": "buried-fact-in-long-note", "relevant_ids": ["n6"] },
  { "query": "how should I pay when I arrive at the inn", "category": "buried-fact-in-long-note", "relevant_ids": ["n6"] },
  { "query": "what upcoming deadlines and appointments do I have", "category": "multi-relevant", "relevant_ids": ["n2", "n4"] },
  { "query": "Dr. Patel", "category": "named-entity", "relevant_ids": ["n4"] },
  { "query": "when are quarterly estimated taxes due", "category": "temporal", "relevant_ids": ["n2"] },
  { "query": "bitwardn migrat export vualt", "category": "messy", "relevant_ids": ["n8"] },
  { "query": "E0433", "category": "named-entity", "relevant_ids": ["n9"] },
  { "query": "I can't fall asleep at night, what helps", "category": "semantic-paraphrase", "relevant_ids": ["n11"] },
  { "query": "pgbouncer connection pool sizing", "category": "lexical-overlap", "relevant_ids": ["n13"] },
  { "query": "how do I change a flat car tire", "category": "negative", "relevant_ids": [] }
]
```

- [ ] **Step 3: Raise the loader-test corpus threshold**

In `src-tauri/crates/raki-eval/src/lib.rs`, in `fixtures_parse_and_reference_real_corpus_ids`, change the corpus assertion:

```rust
        assert!(corpus.len() >= 16, "need a non-trivial corpus");
```

- [ ] **Step 4: Run the loader test**

Run: `cd src-tauri && cargo test -p raki-eval fixtures_parse`
Expected: PASS (18 notes, 14 queries, all relevant ids resolve, buried-fact present).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/fixtures src-tauri/crates/raki-eval/src/lib.rs
git commit -m "Harden eval corpus to 18 notes with keyword/vector confusable pairs"
```

---

## Task 3: Add hybrid as a third method in the harness + report

**Files:**
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`, `src-tauri/crates/raki-eval/src/main.rs`

- [ ] **Step 1: Update the wiring test to expect a hybrid score**

In `src-tauri/crates/raki-eval/src/lib.rs`, in `harness_scores_every_category_with_fake_embedder`, add after the existing in-range checks:

```rust
        // Hybrid is computed and in range for every scored category.
        for c in &report.by_category {
            assert!(c.hybrid.recall >= 0.0 && c.hybrid.recall <= 1.0);
        }
        assert!(report.overall_hybrid.recall >= 0.0 && report.overall_hybrid.recall <= 1.0);
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p raki-eval harness_scores`
Expected: FAIL — no `hybrid` field on `CategoryReport` / `overall_hybrid` on `Report`.

- [ ] **Step 3: Add hybrid to the structs**

In `src-tauri/crates/raki-eval/src/lib.rs`, add a `hybrid` field to `CategoryReport` (after `vector`):

```rust
    pub hybrid: MethodScores,
```

and to `Report` (after `overall_vector`):

```rust
    pub overall_hybrid: MethodScores,
```

- [ ] **Step 4: Compute hybrid in `run_eval`**

In `src-tauri/crates/raki-eval/src/lib.rs`, add `hybrid_search` to the retrieval import:

```rust
use raki_retrieval::{average_precision_at_k, hybrid_search, recall_at_k, reciprocal_rank, search, vector_search};
```

Add a third accumulator next to `cat_kw`/`cat_vec`:

```rust
    let mut cat_hyb: HashMap<String, ScoreAcc> = Default::default();
```

Inside the per-query loop, after the `vec_ids` line, add:

```rust
        let hyb_ids = to_fixture(
            &hybrid_search(&keyword, &vectors, embedder.as_ref(), &q.query, k).await?,
            &fixture_of,
        );
        push_scores(cat_hyb.entry(q.category.clone()).or_default(), &hyb_ids, &relevant, k);
```

In the `by_category` build loop, fetch the hybrid accumulator and set the field:

```rust
    for (cat, kw) in &cat_kw {
        let vc = cat_vec.get(cat).ok_or_else(|| {
            DomainError::Provider(format!("category {cat} missing from vector scores"))
        })?;
        let hy = cat_hyb.get(cat).ok_or_else(|| {
            DomainError::Provider(format!("category {cat} missing from hybrid scores"))
        })?;
        by_category.push(CategoryReport {
            category: cat.clone(),
            scored: kw.0.len(),
            keyword: MethodScores { recall: mean(&kw.0), map: mean(&kw.1), mrr: mean(&kw.2) },
            vector: MethodScores { recall: mean(&vc.0), map: mean(&vc.1), mrr: mean(&vc.2) },
            hybrid: MethodScores { recall: mean(&hy.0), map: mean(&hy.1), mrr: mean(&hy.2) },
        });
    }
```

Set the overall after the existing two:

```rust
    let overall_hybrid = overall(&cat_hyb);
```

and add `overall_hybrid` to the returned `Report { ... }`.

- [ ] **Step 5: Run to verify it passes**

Run: `cd src-tauri && cargo test -p raki-eval`
Expected: PASS.

- [ ] **Step 6: Print three methods in the report binary**

In `src-tauri/crates/raki-eval/src/main.rs`, replace `row` and its callers, and set `k = 3`:

```rust
fn row(label: &str, kw: MethodScores, vc: MethodScores, hy: MethodScores) {
    println!(
        "{label:<26} | kw R{:.2} M{:.2} | vec R{:.2} M{:.2} | hyb R{:.2} M{:.2}",
        kw.recall, kw.map, vc.recall, vc.map, hy.recall, hy.map
    );
}
```

```rust
    let k = 3;
    let report = run_eval(embedder, k).await?;

    println!("Retrieval eval @ k={k}  (R=recall  M=MAP)\n");
    for c in &report.by_category {
        row(&format!("{} (n={})", c.category, c.scored), c.keyword, c.vector, c.hybrid);
    }
    println!("{}", "-".repeat(86));
    row("OVERALL", report.overall_keyword, report.overall_vector, report.overall_hybrid);
```

- [ ] **Step 7: Lint + commit**

Run: `cd src-tauri && cargo clippy -p raki-eval --all-targets -- -D warnings`
```bash
git add src-tauri/crates/raki-eval/src/lib.rs src-tauri/crates/raki-eval/src/main.rs
git commit -m "Add hybrid method to the eval harness and report"
```

---

## Task 4: Wire `search_notes` to hybrid retrieval

**Files:**
- Modify: `src-tauri/src/state.rs`, `src-tauri/src/lib.rs`, `src-tauri/src/commands/notes.rs`

- [ ] **Step 1: Expose vectors + embedder on `AppState`**

In `src-tauri/src/state.rs`, replace the file:

```rust
//! Application state: the injected ports the command layer delegates to.

use std::sync::Arc;

use raki_ai::EgressPolicy;
use raki_domain::{Clock, EmbeddingProvider, KeywordIndex, NoteRepository, VectorIndex};

use crate::indexing::IndexingService;

#[allow(dead_code)]
pub struct AppState {
    pub notes: Arc<dyn NoteRepository>,
    pub keyword: Arc<dyn KeywordIndex>,
    pub vectors: Arc<dyn VectorIndex>,
    pub embedder: Arc<dyn EmbeddingProvider>,
    pub clock: Arc<dyn Clock>,
    pub egress: EgressPolicy,
    pub index: Arc<IndexingService>,
}
```

- [ ] **Step 2: Populate them in the composition root**

In `src-tauri/src/lib.rs`, inside `setup`, clone the Arcs into both the service and the state. Replace the `index`/`app.manage` block:

```rust
            let index = Arc::new(IndexingService::new(store, embedder.clone(), vectors.clone()));
            index.trigger(); // startup catch-up pass (backfill + drain), single-flight

            app.manage(AppState {
                notes,
                keyword,
                vectors,
                embedder,
                clock: Arc::new(SystemClock),
                egress: EgressPolicy::LocalOnly,
                index,
            });
            Ok(())
```

- [ ] **Step 3: Rewrite `search_notes` to use hybrid retrieval**

In `src-tauri/src/commands/notes.rs`, replace the `search_notes` function:

```rust
/// Hybrid search: fuse FTS5 keyword + sqlite-vec vector rankings (RRF), then hydrate
/// the ranked ids to DTOs. (Hydration is one `get` per hit; fine at k = 20.)
#[tauri::command]
pub async fn search_notes(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<NoteDto>, AppError> {
    let ids = raki_retrieval::hybrid_search(
        state.keyword.as_ref(),
        state.vectors.as_ref(),
        state.embedder.as_ref(),
        &query,
        20,
    )
    .await?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let note_id = NoteId::parse(&id)?;
        if let Some(note) = state.notes.get(&note_id).await? {
            out.push(NoteDto::from(note));
        }
    }
    Ok(out)
}
```

- [ ] **Step 4: Build, test, lint, fmt the workspace**

Run: `cd src-tauri && cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --check`
Expected: all green (real-model tests stay ignored).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/state.rs src-tauri/src/lib.rs src-tauri/src/commands/notes.rs
git commit -m "Wire search_notes to hybrid keyword+vector retrieval"
```

---

## Task 5: Recalibrate the gate to floor hybrid (real model)

**Files:**
- Modify: `src-tauri/crates/raki-eval/tests/eval_gate.rs`

- [ ] **Step 1: Observe the real numbers**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report`
Expected: a 3-method table at k=3. **Record OVERALL hybrid recall and MAP.** Sanity: keyword should win the `E0433` named-entity row (exact rare token) and lose the sleep paraphrase row; vector the reverse; hybrid should be at least as good as the better of the two overall. If offline, report as deferred.

- [ ] **Step 2: Update the gate to floor hybrid, k=3, calibrated**

In `src-tauri/crates/raki-eval/tests/eval_gate.rs`, replace the floors + assertions (set the two consts to ~0.10 below the OVERALL hybrid values observed in Step 1; the values below are a starting point — adjust to your observed numbers and record them in the comment):

```rust
// Calibrated 2026-06-05 at k=3 on the 18-note corpus from the first 3-method
// eval-report run. Floors are ~0.10 below observed OVERALL hybrid. Ratchet UP as the
// corpus and retrieval improve — never silently down.
const RECALL_FLOOR: f64 = 0.75;
const MAP_FLOOR: f64 = 0.70;

#[tokio::test]
#[ignore = "runs the real bge model (network + native runtime); run with --ignored"]
async fn retrieval_meets_quality_floor() -> Result<(), Box<dyn std::error::Error>> {
    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let report = run_eval(embedder, 3).await?;

    // Floor the PRODUCTION method (hybrid — what search_notes uses). Both recall and
    // MAP are gated so ranking can't rot while recall holds.
    let recall = report.overall_hybrid.recall;
    let map = report.overall_hybrid.map;

    assert!(recall >= RECALL_FLOOR, "hybrid recall {recall:.3} below floor {RECALL_FLOOR}");
    assert!(map >= MAP_FLOOR, "hybrid MAP {map:.3} below floor {MAP_FLOOR}");
    Ok(())
}
```

- [ ] **Step 3: Run the gate, adjust floors if needed**

Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate -- --ignored`
Expected: PASS. If it fails because observed hybrid is below the starting consts, lower the consts to ~0.10 below the Step 1 observed values (never below a value the system clears) and re-run. If offline, report as deferred.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-eval/tests/eval_gate.rs
git commit -m "Recalibrate retrieval gate to floor hybrid at k=3"
```

---

## Task 6: Slice #2 verification + Definition of Done

- [ ] **Step 1: Full fast sweep**

Run: `cd src-tauri && cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: all pass (ignored real-model tests excluded), fmt clean, no warnings.

- [ ] **Step 2: Real report + gate**

Run: `cd src-tauri && cargo run -q -p raki-eval --bin eval-report`
Run: `cd src-tauri && cargo test -p raki-eval --test eval_gate -- --ignored`
Expected: report prints 3 methods; gate passes. If offline, report both as deferred.

- [ ] **Step 3: Observe — and record — the fusion relationship**

From the Step 2 report, write down whether OVERALL hybrid recall/MAP is **≥ max(keyword, vector)**. This is the measured fusion claim. If hybrid merely ties the best single method on this corpus, that is an honest, reportable result — not a failure (fusion's value grows with harder, larger corpora and is what slice #2's seam unlocks). Also confirm the per-method split shows on the confusable rows: keyword ahead on `E0433`, vector ahead on the sleep paraphrase.

- [ ] **Step 4: Frontend untouched — confirm still green**

Run (repo root): `bun run typecheck && bun run test && bun run build`
Expected: all green.

- [ ] **Step 5: Manual end-to-end smoke (user-performed)**

Run: `bun run tauri dev`
Expected: create notes, search; results now reflect hybrid (semantic matches surface even without exact keywords). Defer to the user; report as deferred rather than claiming success.

- [ ] **Step 6: Final commit (only if Step 1 required a fmt pass)**

```bash
git add -A
git commit -m "Hybrid fusion (slice #2): final verification"
```

---

## Self-Review

**Spec coverage (slice #2):**
- RRF fusion seam → Task 1 (`hybrid_search`, reuses `reciprocal_rank_fusion`).
- Production search uses hybrid → Task 4 (`search_notes` + AppState exposure).
- Eval proves the relationship → Tasks 2 (hardened corpus), 3 (hybrid method + report), 5 (gate floors hybrid).
- Honest framing (observe, don't force, fusion>max) → Task 6 Step 3 + gate floors the production method, not a strict-improvement assertion.
- Graceful degradation (empty vectors → keyword order) → Task 1 (RRF over whatever is present).

**Deferred, named:** larger/synthetic corpus + graded labels (future); fake-fallback-embedder noise suppression (edge case); keyword stemming/OR-of-terms tuning (a future keyword slice the eval can now measure).

**Placeholder scan:** none — fixtures are real content; gate floors are concrete starting consts with an explicit observe-and-adjust calibration step (Task 5), not "TBD".

**Type consistency:**
- `hybrid_search(&dyn KeywordIndex, &dyn VectorIndex, &dyn EmbeddingProvider, &str, usize) -> Result<Vec<String>, DomainError>` defined Task 1, consumed by the harness (Task 3) and the command (Task 4).
- `CategoryReport.hybrid` + `Report.overall_hybrid: MethodScores` defined Task 3, consumed by the bin (Task 3 Step 6) and the gate (Task 5).
- `AppState { vectors, embedder }` added Task 4 Step 1, populated Task 4 Step 2, read by `search_notes` Task 4 Step 3.

---

## Execution Handoff

(Presented to the user after saving.)
