//! `eval-report`: run the golden-set eval with the real model and print a
//! keyword-vs-vector-vs-hybrid table, broken down per taxonomy category. Read this
//! while tuning; the regression gate (tests/eval_gate.rs) is the automated counterpart.

use std::sync::Arc;

use raki_ai::{FastEmbedProvider, FastEmbedReranker};
use raki_domain::{EmbeddingProvider, Reranker};
use raki_eval::{eval_dir, fixtures_fingerprint, run_eval, EvalRun, MethodScores};

fn fmt_opt(o: Option<f64>) -> String {
    o.map(|v| format!("{v:.2}"))
        .unwrap_or_else(|| "  - ".to_string())
}

fn row(label: &str, kw: MethodScores, vc: MethodScores, hy: MethodScores, rr: MethodScores) {
    println!(
        "{label:<24} | kw R{:.2} M{:.2} N{} Cov{} | vec R{:.2} M{:.2} N{} Cov{} | hyb R{:.2} M{:.2} N{} Cov{} | rr R{:.2} M{:.2} N{} Cov{}",
        kw.recall,
        kw.map,
        fmt_opt(kw.ndcg),
        fmt_opt(kw.recall_cov),
        vc.recall,
        vc.map,
        fmt_opt(vc.ndcg),
        fmt_opt(vc.recall_cov),
        hy.recall,
        hy.map,
        fmt_opt(hy.ndcg),
        fmt_opt(hy.recall_cov),
        rr.recall,
        rr.map,
        fmt_opt(rr.ndcg),
        fmt_opt(rr.recall_cov),
    );
}

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
    let reranker = Arc::new(FastEmbedReranker::try_new()?);
    let model_id = embedder.model_id();
    let reranker_id = reranker.model_id();
    let k = 3;
    let run = run_eval(embedder, reranker, k).await?;
    let report = &run.report;

    println!("Retrieval eval @ k={k}  (R=recall  M=MAP  N=nDCG  Cov=recall@10)\n");
    for c in &report.by_category {
        row(
            &format!("{} (n={})", c.category, c.scored),
            c.keyword,
            c.vector,
            c.hybrid,
            c.reranked,
        );
    }
    println!("{}", "-".repeat(148));
    row(
        "OVERALL",
        report.overall_keyword,
        report.overall_vector,
        report.overall_hybrid,
        report.overall_reranked,
    );

    println!(
        "\nreranked = hybrid + rerank ({reranker_id}). nDCG delta vs hybrid (graded categories):"
    );
    for c in report
        .by_category
        .iter()
        .filter(|c| c.hybrid.ndcg.is_some())
    {
        if let (Some(rr), Some(hy)) = (c.reranked.ndcg, c.hybrid.ndcg) {
            println!("  {:<24} {:+.3}", c.category, rr - hy);
        }
    }

    println!("\nPer-query (dev set only):");
    for q in run.per_query.iter().filter(|q| q.set == "dev") {
        println!("  [{}] {:?}", q.category, q.query);
        println!("    kw  {:?}", q.keyword.ranked);
        println!("    vec {:?}", q.vector.ranked);
        println!("    hyb {:?}", q.hybrid.ranked);
        println!("    rr  {:?}", q.reranked.ranked);
    }
    if !report.unscored_categories.is_empty() {
        println!(
            "\nunscored (need score threshold): {:?}",
            report.unscored_categories
        );
    }

    if write {
        write_artifacts(&run, &model_id, &reranker_id, &date)?;
    }
    Ok(())
}

fn write_artifacts(
    run: &EvalRun,
    model_id: &str,
    reranker_id: &str,
    date: &str,
) -> std::io::Result<()> {
    let dir = eval_dir();
    std::fs::create_dir_all(&dir)?;

    // D5: per-query snapshot the gate reads. Pretty-printed for reviewable diffs.
    let mut snap = serde_json::to_string_pretty(&run.per_query).expect("serialize per_query");
    snap.push('\n');
    std::fs::write(dir.join("snapshot.json"), snap)?;

    // D10: human-readable baseline artifact.
    std::fs::write(
        dir.join("baseline.md"),
        baseline_md(run, model_id, reranker_id, date),
    )?;
    eprintln!(
        "wrote {}/snapshot.json and {}/baseline.md",
        dir.display(),
        dir.display()
    );
    Ok(())
}

fn baseline_md(run: &EvalRun, model_id: &str, reranker_id: &str, date: &str) -> String {
    use std::fmt::Write;
    let r = &run.report;
    let mut s = String::with_capacity(2048);
    writeln!(s, "# Eval baseline artifact\n").unwrap();
    writeln!(s, "Date: {date}\n").unwrap();
    s.push_str("Reproducible baseline for the retrieval eval (D10). The gate floors cite these\n");
    s.push_str("numbers; the per-query lock is `snapshot.json` (D5).\n\n");
    s.push_str("## Environment\n\n");
    writeln!(s, "- Model id: `{model_id}`").unwrap();
    writeln!(s, "- Reranker model id: `{reranker_id}`").unwrap();
    s.push_str("- Embedding dimension: 384 (fixed by bge-small-en-v1.5; pinned by model id)\n");
    writeln!(
        s,
        "- Platform: {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
    .unwrap();
    writeln!(
        s,
        "- Fixture fingerprint (FNV-1a, non-security): `{}`",
        fixtures_fingerprint()
    )
    .unwrap();
    s.push_str("- Pinned library versions: see committed `src-tauri/Cargo.lock` (fastembed, ort/onnxruntime, rusqlite/SQLite bundled, sqlite-vec).\n");
    writeln!(s, "- k = {}; coverage_k = 10.", r.k).unwrap();
    s.push_str("- Command: `cargo run -p raki-eval --bin eval-report -- --write --date=<date>`\n");
    s.push_str(
        "- Deterministic ordering: keyword is id-sorted in SQL (`ORDER BY score, note_id`);\n",
    );
    s.push_str(
        "  vector/hybrid order is deterministic on this pinned environment (see D5/D11).\n\n",
    );
    s.push_str(
        "`coverage_k = 10` rationale: top-10 spans ~45% of the 22-note corpus — a sensible\n",
    );
    s.push_str("\"find most\" horizon. Revisit when the corpus grows (3b).\n\n");
    s.push_str("## Per-category (kw / vec / hyb / rr)\n\n");
    s.push_str("| category | n | kw R/M/N/Cov | vec R/M/N/Cov | hyb R/M/N/Cov | rr R/M/N/Cov |\n");
    s.push_str("|---|---|---|---|---|---|\n");
    for c in &r.by_category {
        writeln!(
            s,
            "| {} | {} | {} | {} | {} | {} |",
            c.category,
            c.scored,
            cell(c.keyword),
            cell(c.vector),
            cell(c.hybrid),
            cell(c.reranked)
        )
        .unwrap();
    }
    writeln!(
        s,
        "| **OVERALL** |  | {} | {} | {} | {} |\n",
        cell(r.overall_keyword),
        cell(r.overall_vector),
        cell(r.overall_hybrid),
        cell(r.overall_reranked)
    )
    .unwrap();
    writeln!(s, "Unscored categories: {:?}", r.unscored_categories).unwrap();
    s
}

fn cell(m: MethodScores) -> String {
    format!(
        "{:.2}/{:.2}/{}/{}",
        m.recall,
        m.map,
        fmt_opt(m.ndcg),
        fmt_opt(m.recall_cov)
    )
}
