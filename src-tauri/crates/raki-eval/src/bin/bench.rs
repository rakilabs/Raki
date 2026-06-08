//! `bench`: run the SciFact benchmark tier with the real bge-small embedder + cross-encoder
//! reranker, print the 4-method aggregate table + the `reranked − hybrid` nDCG@10 delta + a
//! vector-sanity line. Pass `--write` to persist `docs/eval/scifact-baseline.md`. Network +
//! model download on first run (cached under `.beir_cache/`). Directional, domain-shifted
//! evidence — NOT a measure of Raki's own retrieval (see ADR-0007).

use std::sync::Arc;

use raki_ai::{FastEmbedProvider, FastEmbedReranker};
use raki_domain::EmbeddingProvider;
use raki_eval::benchmark::{ensure_scifact, run_benchmark, BeirData, MethodAgg};

const K: usize = 10;

fn row(label: &str, m: MethodAgg) -> String {
    format!(
        "| {label:<9} | {:.4} | {:.4} | {:.4} |",
        m.ndcg, m.recall, m.map
    )
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let write = std::env::args().any(|a| a == "--write");

    let dir = match ensure_scifact() {
        Ok(d) => d,
        Err(e) => {
            eprintln!("SciFact unavailable: {e}");
            eprintln!("(needs network on first run; cached under .beir_cache/ thereafter)");
            std::process::exit(1);
        }
    };
    let data = BeirData {
        corpus: raki_eval::benchmark::parse_corpus(&std::fs::read_to_string(
            dir.join("corpus.jsonl"),
        )?)?,
        queries: raki_eval::benchmark::parse_queries(&std::fs::read_to_string(
            dir.join("queries.jsonl"),
        )?)?,
        qrels: raki_eval::benchmark::parse_qrels(&std::fs::read_to_string(
            dir.join("qrels/test.tsv"),
        )?)?,
    };

    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let reranker = Arc::new(FastEmbedReranker::try_new()?);
    let model = embedder.model_id();
    let rep = run_benchmark(&data, embedder, reranker, K).await?;

    let mut out = String::new();
    use std::fmt::Write;
    writeln!(
        out,
        "# SciFact benchmark (k={K}, queries scored = {})",
        rep.queries_scored
    )?;
    writeln!(out, "\n| method    | nDCG@10 | Recall@10 | MAP |")?;
    writeln!(out, "|-----------|---------|-----------|-----|")?;
    writeln!(out, "{}", row("keyword", rep.keyword))?;
    writeln!(out, "{}", row("vector", rep.vector))?;
    writeln!(out, "{}", row("hybrid", rep.hybrid))?;
    writeln!(out, "{}", row("reranked", rep.reranked))?;
    writeln!(
        out,
        "\n**reranked − hybrid nDCG@10 = {:+.4}** (R1 directional signal)",
        rep.reranked_minus_hybrid_ndcg
    )?;
    writeln!(
        out,
        "vector nDCG@10 = {:.4} (sanity vs published bge-small ≈ 0.65)",
        rep.vector.ndcg
    )?;
    writeln!(
        out,
        "\nmodel: {model} · dataset: BEIR SciFact (CC BY-NC 2.0; downloaded, not redistributed)"
    )?;
    println!("{out}");

    if write {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../docs/eval/scifact-baseline.md");
        std::fs::write(&path, out)?;
        eprintln!("wrote {}", path.display());
    } else {
        eprintln!("(stdout only; pass --write to persist docs/eval/scifact-baseline.md)");
    }
    Ok(())
}
