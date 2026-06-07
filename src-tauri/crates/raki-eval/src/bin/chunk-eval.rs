//! `chunk-eval`: LOCAL whole-note-vs-chunked retrieval comparison. Runs the prefix × rollup arms
//! over the committed synthetic chunking fixtures (and, when present, the real-data set), printing
//! whole-vs-chunked deltas stratified by note length. Vector + reranked are headlined; hybrid is demoted
//! but read as a DEPLOYMENT-RISK signal (it mirrors the first production state). Directional only —
//! see the chunking spec, Limitations.

use std::sync::Arc;

use raki_ai::{FastEmbedProvider, FastEmbedReranker};
use raki_eval::chunk::{ChunkStrategy, PrefixMode, Rollup};
use raki_eval::{load_chunking_corpus, load_chunking_queries, run_eval_over, MethodScores};

const K: usize = 10;

fn stratum(body: &str) -> &'static str {
    // crude token proxy: word count. short < 200 words, long > 500.
    let w = body.split_whitespace().count();
    if w < 200 {
        "short"
    } else if w > 500 {
        "long"
    } else {
        "medium"
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let corpus = load_chunking_corpus();
    let queries = load_chunking_queries();
    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let reranker = Arc::new(FastEmbedReranker::try_new()?);

    let strata: Vec<(&str, usize)> = {
        let mut m = std::collections::BTreeMap::new();
        for cn in &corpus {
            *m.entry(stratum(&cn.body)).or_insert(0usize) += 1;
        }
        m.into_iter().collect()
    };
    println!("# chunk-eval (synthetic, LOCAL). k={K}  notes per stratum: {strata:?}\n");

    // Whole-note baseline (one run).
    let whole = run_eval_over(
        &corpus,
        &queries,
        embedder.clone(),
        reranker.clone(),
        K,
        ChunkStrategy::WholeNote,
        PrefixMode::Title,
        Rollup::MinRank,
    )
    .await?;

    // Chunked arms: prefix × rollup.
    let prefixes = [
        ("bare", PrefixMode::Bare),
        ("title", PrefixMode::Title),
        ("title+head", PrefixMode::TitleHeading),
    ];
    let rollups = [
        ("min-rank", Rollup::MinRank),
        ("score-max", Rollup::ScoreMax),
    ];

    let line = |label: &str, w: MethodScores, c: MethodScores| {
        println!(
            "  {label:<22} whole R{:.2} M{:.2} | chunk R{:.2} M{:.2} | Δrecall {:+.3} Δmap {:+.3}",
            w.recall,
            w.map,
            c.recall,
            c.map,
            c.recall - w.recall,
            c.map - w.map
        );
    };

    for (pl, p) in prefixes {
        for (rl, r) in rollups {
            let chunked = run_eval_over(
                &corpus,
                &queries,
                embedder.clone(),
                reranker.clone(),
                K,
                ChunkStrategy::Blocks,
                p,
                r,
            )
            .await?;
            println!("## prefix={pl}  rollup={rl}");
            line(
                "vector (headline)",
                whole.report.overall_vector,
                chunked.report.overall_vector,
            );
            line(
                "reranked (headline)",
                whole.report.overall_reranked,
                chunked.report.overall_reranked,
            );
            line(
                "hybrid (deploy-risk)",
                whole.report.overall_hybrid,
                chunked.report.overall_hybrid,
            );
            // per-category (the buried-fact / coreference / list controls live here).
            for cat in &chunked.report.by_category {
                let wc = whole
                    .report
                    .by_category
                    .iter()
                    .find(|c| c.category == cat.category);
                if let Some(wc) = wc {
                    println!(
                        "    [{}] vec Δrecall {:+.3} | rr Δrecall {:+.3}",
                        cat.category,
                        cat.vector.recall - wc.vector.recall,
                        cat.reranked.recall - wc.reranked.recall
                    );
                }
            }
            println!();
        }
    }

    let real_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../eval-data/real");
    match raki_eval::local_corpus::load_local_raw(&real_dir) {
        Ok(data) => {
            println!("\n# chunk-eval (REAL notes — LOCAL, never committed). k={K}");
            let rstrata: std::collections::BTreeMap<&str, usize> = {
                let mut m = std::collections::BTreeMap::new();
                for cn in &data.corpus {
                    *m.entry(stratum(&cn.body)).or_insert(0) += 1;
                }
                m
            };
            println!(
                "notes per stratum: {rstrata:?}  (promotion gate reads the LONG stratum — spec D8)"
            );
            let whole = run_eval_over(
                &data.corpus,
                &data.queries,
                embedder.clone(),
                reranker.clone(),
                K,
                ChunkStrategy::WholeNote,
                PrefixMode::Title,
                Rollup::MinRank,
            )
            .await?;
            for (pl, p) in prefixes {
                for (rl, r) in rollups {
                    let chunked = run_eval_over(
                        &data.corpus,
                        &data.queries,
                        embedder.clone(),
                        reranker.clone(),
                        K,
                        ChunkStrategy::Blocks,
                        p,
                        r,
                    )
                    .await?;
                    println!("## REAL prefix={pl} rollup={rl}");
                    line(
                        "vector (headline)",
                        whole.report.overall_vector,
                        chunked.report.overall_vector,
                    );
                    line(
                        "reranked (headline)",
                        whole.report.overall_reranked,
                        chunked.report.overall_reranked,
                    );
                    line(
                        "hybrid (deploy-risk)",
                        whole.report.overall_hybrid,
                        chunked.report.overall_hybrid,
                    );
                }
            }
        }
        Err(e) => eprintln!("\n(real-notes run skipped: {e})"),
    }

    eprintln!("note: synthetic numbers settle DESIGN only; the verdict is the real-notes run (spec D7/D8).");
    Ok(())
}
