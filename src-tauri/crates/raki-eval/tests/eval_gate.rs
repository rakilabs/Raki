//! The retrieval regression gate. `#[ignore]`d because it runs the real model
//! (network + native runtime), like the fastembed smoke test. Run explicitly:
//!   cargo test -p raki-eval --test eval_gate -- --ignored
//! and in a dedicated CI job with a warm model cache (keyed on the model id).
//!
//! Floors are calibrated from the first `eval-report` run and set conservatively
//! below the observed values. They are a regression tripwire, not a quality verdict:
//! a tuning change that drops below them goes red. Ratchet them UP as the corpus and
//! retrieval improve — never silently down.

use std::sync::Arc;

use raki_ai::FastEmbedProvider;
use raki_eval::run_eval;

// Calibrated 2026-06-05 at k=3 on the 18-note corpus from the first 3-method
// eval-report run. Floors are ~0.10 below observed OVERALL hybrid. Ratchet UP as the
// corpus and retrieval improve — never silently down.
const RECALL_FLOOR: f64 = 0.90;
const MAP_FLOOR: f64 = 0.90;

#[tokio::test]
#[ignore = "runs the real bge model (network + native runtime); run with --ignored"]
async fn retrieval_meets_quality_floor() -> Result<(), Box<dyn std::error::Error>> {
    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let run = run_eval(embedder, 3).await?;
    let report = &run.report;

    // Floor the PRODUCTION method (hybrid — what search_notes uses). Both recall and
    // MAP are gated so ranking can't rot while recall holds.
    let recall = report.overall_hybrid.recall;
    let map = report.overall_hybrid.map;

    assert!(
        recall >= RECALL_FLOOR,
        "hybrid recall {recall:.3} below floor {RECALL_FLOOR}"
    );
    assert!(
        map >= MAP_FLOOR,
        "hybrid MAP {map:.3} below floor {MAP_FLOOR}"
    );
    Ok(())
}
