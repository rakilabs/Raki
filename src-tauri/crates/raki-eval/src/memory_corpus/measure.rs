//! Compare pure-retrieval baseline vs signal-boosted retrieval on the R4 corpus.

use raki_domain::{
    EmbeddingProvider, KeywordIndex, NoteId, SignalBooster, SignalSource, VectorIndex,
};
use raki_retrieval::{hybrid_candidates_scored, hybrid_search_with_signals, ScoredNote};

use crate::memory_corpus::seed::seed_queries;

pub struct MeasurementResult {
    pub query: String,
    pub expected_note_ids: Vec<NoteId>,
    pub baseline_rank: Option<usize>,
    pub boosted_rank: Option<usize>,
    pub success_at_3_baseline: bool,
    pub success_at_3_boosted: bool,
}

pub async fn measure_lift(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    signal_source: &dyn SignalSource,
    booster: &dyn SignalBooster,
    k: usize,
    now_ms: i64,
) -> Result<Vec<MeasurementResult>, raki_domain::DomainError> {
    let mut results = Vec::new();
    for case in seed_queries() {
        let baseline =
            hybrid_candidates_scored(keyword, vectors, embedder, None, case.query, k).await?;
        let boosted = hybrid_search_with_signals(
            keyword,
            vectors,
            embedder,
            None,
            signal_source,
            booster,
            case.query,
            k,
            now_ms,
        )
        .await?;

        let expected: Vec<NoteId> = case
            .expected_note_ids
            .iter()
            .map(|s| raki_domain::NoteId::parse(s))
            .collect::<Result<Vec<_>, _>>()?;
        let baseline_rank = rank_of_first_relevant(&baseline, &expected);
        let boosted_rank = rank_of_first_relevant_vec(&boosted, &expected);

        results.push(MeasurementResult {
            query: case.query.to_string(),
            expected_note_ids: expected,
            success_at_3_baseline: baseline_rank.map(|r| r < 3).unwrap_or(false),
            success_at_3_boosted: boosted_rank.map(|r| r < 3).unwrap_or(false),
            baseline_rank,
            boosted_rank,
        });
    }
    Ok(results)
}

fn rank_of_first_relevant(scored: &[ScoredNote], expected: &[NoteId]) -> Option<usize> {
    scored.iter().position(|s| expected.contains(&s.note_id))
}

fn rank_of_first_relevant_vec(ranked: &[NoteId], expected: &[NoteId]) -> Option<usize> {
    ranked.iter().position(|id| expected.contains(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_corpus::index::index_seed_corpus;
    use raki_domain::{MixerConfig, NoteId, NoteSignals};
    use raki_memory::DefaultSignalBooster;
    use std::collections::HashMap;

    struct FakeSignalSource(HashMap<NoteId, NoteSignals>);

    #[async_trait::async_trait]
    impl raki_domain::SignalSource for FakeSignalSource {
        async fn get(
            &self,
            ids: &[NoteId],
        ) -> Result<HashMap<NoteId, NoteSignals>, raki_domain::DomainError> {
            Ok(ids
                .iter()
                .map(|id| (*id, self.0.get(id).cloned().unwrap_or_default()))
                .collect())
        }
    }

    #[tokio::test]
    async fn measurement_runs_on_seed_corpus() {
        let (_db, _repo, keyword, vectors, embedder) = index_seed_corpus().await;
        let source = FakeSignalSource(HashMap::new());
        let booster = DefaultSignalBooster::new(MixerConfig::new(7.0, 0.25, 0.15, 2.0).unwrap());
        let results = measure_lift(
            &keyword,
            &vectors,
            embedder.as_ref(),
            &source,
            &booster,
            10,
            1_000_000_000_000i64,
        )
        .await
        .unwrap();
        assert!(!results.is_empty());
    }
}
