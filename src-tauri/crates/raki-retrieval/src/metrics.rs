//! Pure retrieval metrics over ranked id lists. Ids are opaque strings; the caller
//! decides their space (this crate stays domain-only). `None` means "undefined for
//! this query" (e.g. no relevant items) — the caller excludes it from means.

use std::collections::{HashMap, HashSet};

/// Fraction of relevant ids appearing in the top-k. `None` if `relevant` is empty.
pub fn recall_at_k(ranked: &[String], relevant: &HashSet<String>, k: usize) -> Option<f64> {
    if relevant.is_empty() {
        return None;
    }
    let hits = ranked
        .iter()
        .take(k)
        .filter(|id| relevant.contains(*id))
        .count();
    Some(hits as f64 / relevant.len() as f64)
}

/// Average precision at k. `None` if `relevant` is empty.
pub fn average_precision_at_k(
    ranked: &[String],
    relevant: &HashSet<String>,
    k: usize,
) -> Option<f64> {
    if relevant.is_empty() {
        return None;
    }
    let mut hits = 0usize;
    let mut sum = 0.0;
    for (i, id) in ranked.iter().take(k).enumerate() {
        if relevant.contains(id) {
            hits += 1;
            sum += hits as f64 / (i + 1) as f64;
        }
    }
    Some(sum / relevant.len() as f64)
}

/// Reciprocal rank of the first relevant hit; `Some(0.0)` if none in `ranked`.
/// `None` if `relevant` is empty.
pub fn reciprocal_rank(ranked: &[String], relevant: &HashSet<String>) -> Option<f64> {
    if relevant.is_empty() {
        return None;
    }
    for (i, id) in ranked.iter().enumerate() {
        if relevant.contains(id) {
            return Some(1.0 / (i + 1) as f64);
        }
    }
    Some(0.0)
}

/// nDCG@k with graded relevance (gain = grade, 0 if absent). `None` if no grades
/// or the ideal DCG is zero. Binary labels intentionally do NOT produce an nDCG —
/// that would be a fake rank-quality signal.
pub fn ndcg_at_k(ranked: &[String], grades: &HashMap<String, f64>, k: usize) -> Option<f64> {
    if grades.is_empty() {
        return None;
    }
    let dcg: f64 = ranked
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, id)| grades.get(id).copied().unwrap_or(0.0) / ((i + 2) as f64).log2())
        .sum();
    let mut ideal: Vec<f64> = grades.values().copied().collect();
    ideal.sort_by(|a, b| b.total_cmp(a));
    let idcg: f64 = ideal
        .iter()
        .take(k)
        .enumerate()
        .map(|(i, g)| g / ((i + 2) as f64).log2())
        .sum();
    if idcg == 0.0 {
        None
    } else {
        Some(dcg / idcg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn rel(v: &[&str]) -> HashSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn recall_counts_hits_in_top_k() {
        let ranked = ids(&["a", "b", "c"]);
        assert_eq!(recall_at_k(&ranked, &rel(&["b", "x"]), 3), Some(0.5));
        assert_eq!(recall_at_k(&ranked, &rel(&["a"]), 1), Some(1.0));
        assert_eq!(recall_at_k(&ranked, &rel(&["c"]), 2), Some(0.0)); // c is rank 3, outside k=2
        assert_eq!(recall_at_k(&ranked, &rel(&[]), 3), None);
    }

    #[test]
    fn average_precision_rewards_earlier_hits() {
        // relevant at ranks 1 and 3 → (1/1 + 2/3)/2
        let ranked = ids(&["a", "x", "b"]);
        let ap = average_precision_at_k(&ranked, &rel(&["a", "b"]), 3).unwrap();
        assert!((ap - ((1.0 + 2.0 / 3.0) / 2.0)).abs() < 1e-9);
    }

    #[test]
    fn reciprocal_rank_uses_first_hit() {
        assert_eq!(reciprocal_rank(&ids(&["x", "a"]), &rel(&["a"])), Some(0.5));
        assert_eq!(reciprocal_rank(&ids(&["x", "y"]), &rel(&["a"])), Some(0.0));
    }

    #[test]
    fn ndcg_is_one_for_ideal_order_and_none_for_binary() {
        let mut grades = HashMap::new();
        grades.insert("a".to_string(), 3.0);
        grades.insert("b".to_string(), 1.0);
        let ideal = ids(&["a", "b"]);
        assert_eq!(ndcg_at_k(&ideal, &grades, 2), Some(1.0));
        // empty grades (binary-only labels) → no nDCG
        assert_eq!(ndcg_at_k(&ideal, &HashMap::new(), 2), None);
    }
}
