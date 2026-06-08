//! Quick smoke test: run the SciFact benchmark pipeline on a small subset with real models.
//! Verifies end-to-end correctness without the full ~20min runtime on CPU.

use std::sync::Arc;

use raki_ai::{FastEmbedProvider, FastEmbedReranker};
use raki_domain::EmbeddingProvider;
use raki_eval::benchmark::{
    ensure_scifact, parse_corpus, parse_qrels, parse_queries, run_benchmark, BeirData,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = ensure_scifact()?;
    let mut data = BeirData {
        corpus: parse_corpus(&std::fs::read_to_string(dir.join("corpus.jsonl"))?)?,
        queries: parse_queries(&std::fs::read_to_string(dir.join("queries.jsonl"))?)?,
        qrels: parse_qrels(&std::fs::read_to_string(dir.join("qrels/test.tsv"))?)?,
    };

    // Subset for speed: first 100 docs, first 20 queries that have relevant docs.
    data.corpus.truncate(100);
    let corpus_ids: std::collections::HashSet<String> =
        data.corpus.iter().map(|d| d.id.clone()).collect();
    data.queries.retain(|(qid, _)| {
        data.qrels
            .get(qid)
            .map(|g| {
                let has_rel = g.values().any(|&s| s > 0.0);
                let ids_in_corpus = g.keys().any(|did| corpus_ids.contains(did));
                has_rel && ids_in_corpus
            })
            .unwrap_or(false)
    });
    data.queries.truncate(20);

    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let reranker = Arc::new(FastEmbedReranker::try_new()?);
    let model = embedder.model_id();
    let rep = run_benchmark(&data, embedder, reranker, 10).await?;

    println!(
        "# SciFact smoke test (subset: {} docs, {} queries)",
        data.corpus.len(),
        rep.queries_scored
    );
    println!("| method    | nDCG@10 | Recall@10 | MAP |");
    println!("|-----------|---------|-----------|-----|");
    println!(
        "| keyword   | {:.4} | {:.4} | {:.4} |",
        rep.keyword.ndcg, rep.keyword.recall, rep.keyword.map
    );
    println!(
        "| vector    | {:.4} | {:.4} | {:.4} |",
        rep.vector.ndcg, rep.vector.recall, rep.vector.map
    );
    println!(
        "| hybrid    | {:.4} | {:.4} | {:.4} |",
        rep.hybrid.ndcg, rep.hybrid.recall, rep.hybrid.map
    );
    println!(
        "| reranked  | {:.4} | {:.4} | {:.4} |",
        rep.reranked.ndcg, rep.reranked.recall, rep.reranked.map
    );
    println!(
        "\nreranked − hybrid nDCG@10 = {:+.4}",
        rep.reranked_minus_hybrid_ndcg
    );
    println!("model: {model}");

    // Coarse sanity: vector should find something, reranker shouldn't collapse.
    assert!(rep.vector.ndcg > 0.0, "vector should find relevant docs");
    assert!(
        rep.reranked.ndcg >= 0.5 * rep.hybrid.ndcg,
        "reranker shouldn't collapse"
    );
    println!("\n✅ Smoke test passed");
    Ok(())
}
