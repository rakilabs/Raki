//! Record a pure-retrieval baseline for the R4 memory corpus.

use raki_domain::{EmbeddingProvider, KeywordIndex, NoteId, VectorIndex};
use raki_retrieval::hybrid_candidates_scored;

use crate::memory_corpus::seed::seed_queries;

pub struct BaselineResult {
    pub query: String,
    pub ranked_ids: Vec<NoteId>,
}

pub async fn record_baseline(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    k: usize,
) -> Result<Vec<BaselineResult>, raki_domain::DomainError> {
    let mut results = Vec::new();
    for case in seed_queries() {
        let scored =
            hybrid_candidates_scored(keyword, vectors, embedder, None, case.query, k).await?;
        results.push(BaselineResult {
            query: case.query.to_string(),
            ranked_ids: scored.into_iter().map(|s| s.note_id).collect(),
        });
    }
    Ok(results)
}
