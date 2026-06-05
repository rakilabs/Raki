//! `eval-report`: run the golden-set eval with the real model and print a
//! keyword-vs-vector-vs-hybrid table, broken down per taxonomy category. Read this
//! while tuning; the regression gate (tests/eval_gate.rs) is the automated counterpart.

use std::sync::Arc;

use raki_ai::FastEmbedProvider;
use raki_eval::{run_eval, MethodScores};

fn row(label: &str, kw: MethodScores, vc: MethodScores, hy: MethodScores) {
    println!(
        "{label:<26} | kw R{:.2} M{:.2} | vec R{:.2} M{:.2} | hyb R{:.2} M{:.2}",
        kw.recall, kw.map, vc.recall, vc.map, hy.recall, hy.map
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let k = 3;
    let report = run_eval(embedder, k).await?;

    println!("Retrieval eval @ k={k}  (R=recall  M=MAP)\n");
    for c in &report.by_category {
        row(
            &format!("{} (n={})", c.category, c.scored),
            c.keyword,
            c.vector,
            c.hybrid,
        );
    }
    println!("{}", "-".repeat(86));
    row(
        "OVERALL",
        report.overall_keyword,
        report.overall_vector,
        report.overall_hybrid,
    );
    if !report.unscored_categories.is_empty() {
        println!(
            "\nunscored (need score threshold): {:?}",
            report.unscored_categories
        );
    }
    Ok(())
}
