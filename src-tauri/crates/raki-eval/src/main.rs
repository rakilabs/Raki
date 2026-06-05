//! `eval-report`: run the golden-set eval with the real model and print a
//! keyword-vs-vector-vs-hybrid table, broken down per taxonomy category. Read this
//! while tuning; the regression gate (tests/eval_gate.rs) is the automated counterpart.

use std::sync::Arc;

use raki_ai::FastEmbedProvider;
use raki_eval::{run_eval, MethodScores};

fn fmt_opt(o: Option<f64>) -> String {
    o.map(|v| format!("{v:.2}"))
        .unwrap_or_else(|| "  - ".to_string())
}

fn row(label: &str, kw: MethodScores, vc: MethodScores, hy: MethodScores) {
    println!(
        "{label:<24} | kw R{:.2} M{:.2} N{} | vec R{:.2} M{:.2} N{} | hyb R{:.2} M{:.2} N{}",
        kw.recall,
        kw.map,
        fmt_opt(kw.ndcg),
        vc.recall,
        vc.map,
        fmt_opt(vc.ndcg),
        hy.recall,
        hy.map,
        fmt_opt(hy.ndcg),
    );
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let k = 3;
    let run = run_eval(embedder, k).await?;
    let report = &run.report;

    println!("Retrieval eval @ k={k}  (R=recall  M=MAP  N=nDCG  Cov=recall@10)\n");
    for c in &report.by_category {
        row(
            &format!("{} (n={})", c.category, c.scored),
            c.keyword,
            c.vector,
            c.hybrid,
        );
    }
    println!("{}", "-".repeat(96));
    row(
        "OVERALL",
        report.overall_keyword,
        report.overall_vector,
        report.overall_hybrid,
    );

    println!("\nPer-query (dev set only):");
    for q in run.per_query.iter().filter(|q| q.set == "dev") {
        println!("  [{}] {:?}", q.category, q.query);
        println!("    kw  {:?}", q.keyword.ranked);
        println!("    vec {:?}", q.vector.ranked);
        println!("    hyb {:?}", q.hybrid.ranked);
    }
    if !report.unscored_categories.is_empty() {
        println!(
            "\nunscored (need score threshold): {:?}",
            report.unscored_categories
        );
    }
    Ok(())
}
