//! The retrieval regression gate. Two layers (spec D5 + D8):
//!  - `keyword_snapshot_is_deterministic` runs the fake embedder and checks the KEYWORD
//!    per-query snapshot. Keyword retrieval is real FTS5 and model-independent, so this is
//!    exact and runs in ordinary `cargo test` (and as the required CI job).
//!  - `real_model_gate` (#[ignore]) runs the real bge model and checks the vector/hybrid
//!    per-query snapshots plus per-method average floors. Run explicitly:
//!    cargo test -p raki-eval --test eval_gate -- --ignored
//!
//! The snapshots (D5) are the teeth; the floors (D8) are a coarse smoke alarm. Coverage is
//! floored on recall@10 (its proper metric), never averaged into the recall@3 floor.

use std::sync::Arc;

use raki_ai::{FakeEmbeddingProvider, FakeReranker, FastEmbedProvider, FastEmbedReranker};
use raki_eval::{load_snapshot, run_eval, snapshot_regressions, Method, MethodScores, QueryResult};

// Re-baselined for hardened corpus 3b, 2026-06-06: floors ~0.10 below the observed
// OVERALL on the 30-note / 25-query corpus. One-time downward recalibration (the test got
// harder, ADR-0005 §ratchet); up-only ratcheting resumes. Per-query snapshots guard rot.
// Observed non-coverage: kw recall~0.89 MAP~0.83 | vec/hyb recall~1.00 MAP~0.98.
// Observed coverage recall@10 = 1.00. Observed ordering nDCG min = 0.91 (paraphrase-distractor).
const KW_RECALL_FLOOR: f64 = 0.75; // ~0.14 below observed kw non-coverage recall
const KW_MAP_FLOOR: f64 = 0.70; // ~0.13 below observed kw non-coverage MAP
const VEC_RECALL_FLOOR: f64 = 0.90; // ~0.10 below observed vec non-coverage recall
const VEC_MAP_FLOOR: f64 = 0.90; // ~0.08 below observed vec non-coverage MAP
const HYB_RECALL_FLOOR: f64 = 0.90; // ~0.10 below observed hyb non-coverage recall
const HYB_MAP_FLOOR: f64 = 0.90; // ~0.08 below observed hyb non-coverage MAP
const COVERAGE_RECALL10_FLOOR: f64 = 0.85; // ~0.15 below observed coverage recall@10
const ORDERING_NDCG_FLOOR: f64 = 0.80; // ~0.11 below the MIN observed nDCG across the 3 ordering cats

// Slice 4 (additive): reranked = hybrid + rerank. Floors ~0.10 below observed; existing
// floors are unchanged (this is not a downward re-baseline of the others). Measure-then-floor.
const RR_RECALL_FLOOR: f64 = 0.90; // ~0.08 below observed reranked non-coverage recall (~0.98)
const RR_MAP_FLOOR: f64 = 0.90; // ~0.06 below observed reranked non-coverage MAP (~0.96)

fn mean(it: impl Iterator<Item = f64>) -> f64 {
    let (sum, n) = it.fold((0.0, 0usize), |(s, n), v| (s + v, n + 1));
    if n == 0 {
        0.0
    } else {
        sum / n as f64
    }
}

fn noncov_mean(per_query: &[QueryResult], m: Method, f: fn(&MethodScores) -> f64) -> f64 {
    mean(
        per_query
            .iter()
            .filter(|q| q.category != "coverage")
            .map(|q| f(&q.method(m).scores)),
    )
}

#[tokio::test]
async fn keyword_snapshot_is_deterministic() -> Result<(), Box<dyn std::error::Error>> {
    let run = run_eval(
        Arc::new(FakeEmbeddingProvider::new(384)),
        Arc::new(FakeReranker),
        3,
    )
    .await?;
    let baseline = load_snapshot()?;
    let regressions = snapshot_regressions(&run.per_query, &baseline, &[Method::Keyword]);
    assert!(
        regressions.is_empty(),
        "keyword regressions:\n{}",
        regressions.join("\n")
    );
    Ok(())
}

#[tokio::test]
#[ignore = "runs the real bge model (network + native runtime); run with --ignored"]
async fn real_model_gate() -> Result<(), Box<dyn std::error::Error>> {
    let run = run_eval(
        Arc::new(FastEmbedProvider::try_new()?),
        Arc::new(FastEmbedReranker::try_new()?),
        3,
    )
    .await?;
    let baseline = load_snapshot()?;

    // D5: no vector/hybrid/reranked query regresses (keyword already covered deterministically).
    let regressions = snapshot_regressions(
        &run.per_query,
        &baseline,
        &[Method::Vector, Method::Hybrid, Method::Reranked],
    );
    assert!(
        regressions.is_empty(),
        "vec/hyb regressions:\n{}",
        regressions.join("\n")
    );

    // D8: per-method floors over non-coverage queries.
    let pq = &run.per_query;
    for (m, rf, mf) in [
        (Method::Keyword, KW_RECALL_FLOOR, KW_MAP_FLOOR),
        (Method::Vector, VEC_RECALL_FLOOR, VEC_MAP_FLOOR),
        (Method::Hybrid, HYB_RECALL_FLOOR, HYB_MAP_FLOOR),
        (Method::Reranked, RR_RECALL_FLOOR, RR_MAP_FLOOR),
    ] {
        let r = noncov_mean(pq, m, |s| s.recall);
        let mp = noncov_mean(pq, m, |s| s.map);
        assert!(r >= rf, "{m:?} non-coverage recall {r:.3} below floor {rf}");
        assert!(mp >= mf, "{m:?} non-coverage MAP {mp:.3} below floor {mf}");
    }

    // D8: coverage floored on recall@10 (vec + hyb — the production-facing methods).
    let cov = pq
        .iter()
        .find(|q| q.category == "coverage")
        .expect("a coverage query");
    for m in [Method::Vector, Method::Hybrid] {
        let c = cov
            .method(m)
            .scores
            .recall_cov
            .expect("coverage recall@10 present");
        assert!(
            c >= COVERAGE_RECALL10_FLOOR,
            "{m:?} coverage recall@10 {c:.3} below floor {COVERAGE_RECALL10_FLOOR}"
        );
    }

    // D8: ordering categories floored on nDCG@3 (vec + hyb + rr).
    const ORDERING: &[&str] = &[
        "lexical-cluster",
        "dense-near-duplicate",
        "paraphrase-distractor",
    ];
    for q in pq
        .iter()
        .filter(|q| ORDERING.contains(&q.category.as_str()))
    {
        for m in [Method::Vector, Method::Hybrid, Method::Reranked] {
            let n = q.method(m).scores.ndcg.expect("ordering nDCG present");
            assert!(
                n >= ORDERING_NDCG_FLOOR,
                "{:?} {m:?} nDCG {n:.3} below floor {ORDERING_NDCG_FLOOR}",
                q.query
            );
        }
    }
    Ok(())
}
