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

// Calibrated 2026-06-05 from the first eval-report run. Set ~0.1 below observed
// OVERALL to avoid flakiness; raise when retrieval improves.
const RECALL_FLOOR: f64 = 0.60;
const MAP_FLOOR: f64 = 0.45;

#[tokio::test]
#[ignore = "runs the real bge model (network + native runtime); run with --ignored"]
async fn retrieval_meets_quality_floor() -> Result<(), Box<dyn std::error::Error>> {
    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let report = run_eval(embedder, 5).await?;

    // The gate floors the BEST available single method (vector here; fusion in #2
    // should only raise this). Both recall and MAP are gated so ranking can't rot
    // while recall holds.
    let best_recall = report
        .overall_keyword
        .recall
        .max(report.overall_vector.recall);
    let best_map = report.overall_keyword.map.max(report.overall_vector.map);

    assert!(
        best_recall >= RECALL_FLOOR,
        "overall recall {best_recall:.3} fell below floor {RECALL_FLOOR}"
    );
    assert!(
        best_map >= MAP_FLOOR,
        "overall MAP {best_map:.3} fell below floor {MAP_FLOOR}"
    );
    Ok(())
}
