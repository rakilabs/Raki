//! Coarse pipeline-sanity floors for the SciFact tier — NOT a quality-regression gate.
//! `#[ignore]`: runs the real bge model + downloads the dataset. Run with `--ignored`.

use std::sync::Arc;

use raki_ai::{FastEmbedProvider, FastEmbedReranker};
use raki_eval::benchmark::{
    ensure_scifact, parse_corpus, parse_qrels, parse_queries, run_benchmark, BeirData,
};

// Committed floor: ~0.10 below published bge-small nDCG@10 ≈ 0.65 (review M2).
const VECTOR_NDCG_FLOOR: f64 = 0.55;

#[tokio::test]
#[ignore = "downloads the SciFact dataset + runs the real bge model; run with --ignored"]
async fn scifact_pipeline_is_calibrated_and_reranker_is_plausible(
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = ensure_scifact()?;
    let data = BeirData {
        corpus: parse_corpus(&std::fs::read_to_string(dir.join("corpus.jsonl"))?)?,
        queries: parse_queries(&std::fs::read_to_string(dir.join("queries.jsonl"))?)?,
        qrels: parse_qrels(&std::fs::read_to_string(dir.join("qrels/test.tsv"))?)?,
    };
    let embedder = Arc::new(FastEmbedProvider::try_new()?);
    let reranker = Arc::new(FastEmbedReranker::try_new()?);
    let rep = run_benchmark(&data, embedder, reranker, 10).await?;

    // M2: vector calibration — catches a broken bi-encoder / index wiring.
    assert!(
        rep.vector.ndcg >= VECTOR_NDCG_FLOOR,
        "vector nDCG@10 {:.4} below floor {VECTOR_NDCG_FLOOR} (pipeline likely broken, not the model)",
        rep.vector.ndcg
    );
    // M7: reranker plausibility — a garbage/misconfigured cross-encoder must not pass and feed
    // R1 a false delta. (run_benchmark already errors out if the reranker path fails.)
    let delta = rep.reranked_minus_hybrid_ndcg;
    assert!(delta.is_finite(), "reranked−hybrid delta not finite");
    assert!(
        (-0.10..=0.20).contains(&delta),
        "reranked−hybrid nDCG@10 {delta:+.4} outside the plausible band [-0.10, +0.20]"
    );
    assert!(
        rep.reranked.ndcg >= 0.5 * rep.hybrid.ndcg,
        "reranked nDCG {:.4} collapsed vs hybrid {:.4} — reranker likely broken",
        rep.reranked.ndcg,
        rep.hybrid.ndcg
    );
    Ok(())
}
