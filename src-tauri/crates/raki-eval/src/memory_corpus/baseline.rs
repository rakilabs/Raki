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
    use std::sync::Arc;

    use raki_ai::FakeEmbeddingProvider;
    use raki_domain::body_to_text;

    use super::*;
    use crate::build_in_memory_index;
    use crate::memory_corpus::seed::seed_notes;
    use raki_domain::NoteRepository;

    #[tokio::test]
    async fn record_baseline_with_fake_adapters_produces_self_describing_results() {
        let (_db, repo, keyword, vectors) = build_in_memory_index().unwrap();
        let embedder = Arc::new(FakeEmbeddingProvider::new(384));

        for note in seed_notes() {
            repo.upsert(&note).await.unwrap();
            let text = format!("{}\n\n{}", note.title, body_to_text(&note.body));
            let emb = embedder.embed(std::slice::from_ref(&text)).await.unwrap();
            let emb = emb
                .into_iter()
                .next()
                .expect("fake embedder returns one embedding per input");
            vectors.upsert(&note.id.to_string(), &emb).await.unwrap();
        }

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
