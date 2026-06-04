//! Reciprocal Rank Fusion: combine multiple ranked id-lists into one ranking
//! without needing to normalize heterogeneous scores. score = Σ 1/(k + rank).

use std::collections::HashMap;

pub const DEFAULT_RRF_K: f64 = 60.0;

/// Fuse rankings (each a list of source ids, best-first) into a descending-score ranking.
/// Ties broken by id for determinism.
pub fn reciprocal_rank_fusion(rankings: &[Vec<String>], k: f64) -> Vec<(String, f64)> {
    let mut scores: HashMap<String, f64> = HashMap::new();
    for ranking in rankings {
        for (rank, id) in ranking.iter().enumerate() {
            *scores.entry(id.clone()).or_insert(0.0) += 1.0 / (k + (rank as f64) + 1.0);
        }
    }
    let mut fused: Vec<(String, f64)> = scores.into_iter().collect();
    fused.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuses_two_rankings_by_rrf() {
        // "b" is ranked high in both lists; "a" and "c" appear once each.
        let keyword = vec!["a".to_string(), "b".to_string()];
        let vector = vec!["b".to_string(), "c".to_string()];
        let fused = reciprocal_rank_fusion(&[keyword, vector], DEFAULT_RRF_K);

        assert_eq!(fused[0].0, "b", "item present in both rankings wins");
        assert_eq!(fused.len(), 3);
    }

    #[test]
    fn empty_input_yields_empty_output() {
        let fused = reciprocal_rank_fusion(&[], DEFAULT_RRF_K);
        assert!(fused.is_empty());
    }
}
