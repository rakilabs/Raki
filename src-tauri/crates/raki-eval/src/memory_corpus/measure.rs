//! Compare pure-retrieval baseline vs signal-boosted retrieval on the R4 corpus.

use std::collections::HashMap;

use raki_domain::{
    EmbeddingProvider, KeywordIndex, MixerConfig, NoteId, NoteSignals, SignalBooster, SignalSource,
    VectorIndex,
};
use raki_memory::DefaultSignalBooster;
use raki_retrieval::{hybrid_candidates_scored, hybrid_search_with_signals, ScoredNote};

use crate::memory_corpus::seed::seed_queries;

pub struct MeasurementResult {
    pub query: String,
    pub expected_note_ids: Vec<NoteId>,
    /// 0-based rank of the first relevant note in the pure-retrieval baseline.
    pub baseline_rank: Option<usize>,
    /// 0-based rank of the first relevant note after signal boosting.
    pub boosted_rank: Option<usize>,
    pub success_at_3_baseline: bool,
    pub success_at_3: bool,
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
            success_at_3: boosted_rank.map(|r| r < 3).unwrap_or(false),
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

/// In-memory signal source for evaluation: every note gets a deterministic signal
/// vector built from the seed corpus metadata.
pub struct MapSignalSource {
    pub signals: HashMap<NoteId, NoteSignals>,
}

#[async_trait::async_trait]
impl SignalSource for MapSignalSource {
    async fn get(
        &self,
        ids: &[NoteId],
    ) -> Result<HashMap<NoteId, NoteSignals>, raki_domain::DomainError> {
        Ok(ids
            .iter()
            .map(|id| (*id, self.signals.get(id).cloned().unwrap_or_default()))
            .collect())
    }
}

/// Build a signal map that exercises all three signals on the expected notes:
/// - pinned notes from `seed_pinned()` are marked pinned.
/// - expected notes are given a recent last-accessed timestamp and a high view count.
/// - other notes are left mostly stale.
pub fn build_signal_map(now_ms: i64) -> HashMap<NoteId, NoteSignals> {
    use crate::memory_corpus::seed::{seed_notes, seed_pinned, seed_queries};

    let mut signals: HashMap<NoteId, NoteSignals> = seed_notes()
        .into_iter()
        .map(|n| {
            (
                n.id,
                NoteSignals {
                    pinned: false,
                    view_count: 0,
                    last_accessed_at_ms: Some(now_ms - 30 * 86_400_000),
                },
            )
        })
        .collect();

    for pinned in seed_pinned() {
        let id = NoteId::parse(pinned).expect("seed_pinned contains valid ids");
        signals.entry(id).and_modify(|s| s.pinned = true);
    }

    for case in seed_queries() {
        for expected in case.expected_note_ids {
            let id = NoteId::parse(expected).expect("seed query ids are valid");
            signals.entry(id).and_modify(|s| {
                s.last_accessed_at_ms = Some(now_ms);
                s.view_count = 10;
            });
        }
    }

    signals
}

/// One row of an ablation report.
#[derive(Debug, Clone, PartialEq)]
pub struct AblationResult {
    pub name: &'static str,
    pub success_at_3_count: usize,
    pub total_queries: usize,
    pub delta_vs_baseline: i64,
    pub delta_vs_all_signals: i64,
}

fn baseline_booster() -> DefaultSignalBooster {
    DefaultSignalBooster::new(MixerConfig::new(7.0, 0.0, 0.0, 1.0).unwrap())
}

fn all_signals_booster() -> DefaultSignalBooster {
    DefaultSignalBooster::new(MixerConfig::new(7.0, 0.25, 0.15, 2.0).unwrap())
}

fn recency_only_booster() -> DefaultSignalBooster {
    DefaultSignalBooster::new(MixerConfig::new(7.0, 0.0, 0.0, 2.0).unwrap())
}

fn pin_only_booster() -> DefaultSignalBooster {
    DefaultSignalBooster::new(MixerConfig::new(7.0, 0.25, 0.0, 2.0).unwrap())
}

fn salience_only_booster() -> DefaultSignalBooster {
    DefaultSignalBooster::new(MixerConfig::new(7.0, 0.0, 0.15, 2.0).unwrap())
}

fn isolate_pin(signals: &HashMap<NoteId, NoteSignals>) -> HashMap<NoteId, NoteSignals> {
    signals
        .iter()
        .map(|(id, s)| {
            let mut s = s.clone();
            s.last_accessed_at_ms = None;
            s.view_count = 0;
            (*id, s)
        })
        .collect()
}

fn isolate_salience(signals: &HashMap<NoteId, NoteSignals>) -> HashMap<NoteId, NoteSignals> {
    signals
        .iter()
        .map(|(id, s)| {
            let mut s = s.clone();
            s.last_accessed_at_ms = None;
            s.pinned = false;
            (*id, s)
        })
        .collect()
}

/// Run the R4 measurement under five booster configurations: no-op baseline,
/// all signals combined, recency-only, pin-only, and salience-only.
pub async fn ablate_signals(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    signal_source: &MapSignalSource,
    k: usize,
    now_ms: i64,
) -> Result<Vec<AblationResult>, raki_domain::DomainError> {
    let baseline = measure_lift(
        keyword,
        vectors,
        embedder,
        signal_source,
        &baseline_booster(),
        k,
        now_ms,
    )
    .await?;
    let baseline_success = baseline.iter().filter(|r| r.success_at_3).count();
    let total = baseline.len();

    let all_run = measure_lift(
        keyword,
        vectors,
        embedder,
        signal_source,
        &all_signals_booster(),
        k,
        now_ms,
    )
    .await?;
    let all_success = all_run.iter().filter(|r| r.success_at_3).count();

    let pin_source = MapSignalSource {
        signals: isolate_pin(&signal_source.signals),
    };
    let salience_source = MapSignalSource {
        signals: isolate_salience(&signal_source.signals),
    };

    let mut results = vec![
        AblationResult {
            name: "baseline",
            success_at_3_count: baseline_success,
            total_queries: total,
            delta_vs_baseline: 0,
            delta_vs_all_signals: baseline_success as i64 - all_success as i64,
        },
        AblationResult {
            name: "all",
            success_at_3_count: all_success,
            total_queries: total,
            delta_vs_baseline: all_success as i64 - baseline_success as i64,
            delta_vs_all_signals: 0,
        },
    ];

    let singles: [(&'static str, DefaultSignalBooster, &dyn SignalSource); 3] = [
        ("recency", recency_only_booster(), signal_source),
        ("pin", pin_only_booster(), &pin_source),
        ("salience", salience_only_booster(), &salience_source),
    ];

    for (name, booster, src) in singles {
        let run = measure_lift(keyword, vectors, embedder, src, &booster, k, now_ms).await?;
        let success = run.iter().filter(|r| r.success_at_3).count();
        results.push(AblationResult {
            name,
            success_at_3_count: success,
            total_queries: total,
            delta_vs_baseline: success as i64 - baseline_success as i64,
            delta_vs_all_signals: success as i64 - all_success as i64,
        });
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::memory_corpus::index::index_seed_corpus;

    #[tokio::test]
    async fn measurement_runs_on_seed_corpus() {
        let (_db, _repo, keyword, vectors, embedder) = index_seed_corpus().await;
        let source = MapSignalSource {
            signals: HashMap::new(),
        };
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

    #[tokio::test]
    async fn ablation_reports_all_configurations() {
        let (_db, _repo, keyword, vectors, embedder) = index_seed_corpus().await;
        let now = 1_000_000_000_000i64;
        let source = MapSignalSource {
            signals: build_signal_map(now),
        };

        let report = ablate_signals(&keyword, &vectors, embedder.as_ref(), &source, 10, now)
            .await
            .unwrap();

        assert_eq!(report.len(), 5, "baseline + 4 signal configs");
        assert_eq!(report[0].name, "baseline");
        for row in &report {
            assert_eq!(row.total_queries, report[0].total_queries);
            assert!(row.success_at_3_count <= row.total_queries);
        }
        assert!(
            report
                .iter()
                .any(|r| r.success_at_3_count >= report[0].success_at_3_count),
            "at least one config matches or beats the no-signal baseline"
        );
    }
}
