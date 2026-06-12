//! Record a pure-retrieval baseline for the R4 memory corpus.

use raki_domain::{EmbeddingProvider, KeywordIndex, NoteId, VectorIndex};
use raki_retrieval::hybrid_candidates_scored;
use serde::{Deserialize, Serialize};

use crate::memory_corpus::seed::seed_queries;

/// One query's ranked baseline output, self-describing for downstream diffing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaselineResult {
    pub query: String,
    pub expected_note_ids: Vec<NoteId>,
    pub ranked_ids: Vec<NoteId>,
    pub ranked_scores: Vec<f64>,
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
        let (ranked_ids, ranked_scores): (Vec<NoteId>, Vec<f64>) = scored
            .into_iter()
            .map(|s| (s.note_id, s.retrieval_score))
            .unzip();
        results.push(BaselineResult {
            query: case.query.to_string(),
            expected_note_ids: case
                .expected_note_ids
                .iter()
                .map(|id| NoteId::parse(id))
                .collect::<Result<Vec<_>, _>>()?,
            ranked_ids,
            ranked_scores,
        });
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_corpus::index::index_seed_corpus;
    use crate::memory_corpus::seed::seed_notes;

    #[tokio::test]
    async fn record_baseline_with_fake_adapters_produces_self_describing_results() {
        let (_db, _repo, keyword, vectors, embedder) = index_seed_corpus().await;

        let results = record_baseline(&keyword, &vectors, embedder.as_ref(), seed_notes().len())
            .await
            .unwrap();

        assert_eq!(results.len(), seed_queries().len(), "one result per query");

        for result in &results {
            assert!(!result.ranked_ids.is_empty(), "every query returns results");
            assert_eq!(
                result.ranked_ids.len(),
                result.ranked_scores.len(),
                "ids and scores are paired"
            );
            for expected in &result.expected_note_ids {
                assert!(
                    result.ranked_ids.contains(expected),
                    "query {:?} expected {} in ranked output",
                    result.query,
                    expected
                );
            }
        }
    }
}
