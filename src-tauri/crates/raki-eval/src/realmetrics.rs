//! Binary "did I find it" metrics for the real-data tier. Recall/MRR reuse the
//! `raki-retrieval` primitives; these two are the additions (Success@k, Primary-Success@1).

use std::collections::HashSet;

/// 1.0 if any relevant id appears in the top-`k`, else 0.0.
pub fn success_at_k(ranked: &[String], relevant: &HashSet<String>, k: usize) -> f64 {
    if ranked.iter().take(k).any(|id| relevant.contains(id)) {
        1.0
    } else {
        0.0
    }
}

/// 1.0 if the (unambiguous) `primary` note is ranked #1; `None` when the query marks no
/// primary (so it is excluded from the Primary-Success@1 aggregate + its denominator).
pub fn primary_success_at_1(ranked: &[String], primary: Option<&str>) -> Option<f64> {
    let p = primary?;
    Some(if ranked.first().map(|s| s.as_str()) == Some(p) {
        1.0
    } else {
        0.0
    })
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
    fn success_at_k_single_and_multi_relevant() {
        let ranked = ids(&["x", "a", "y"]);
        assert_eq!(success_at_k(&ranked, &rel(&["a"]), 3), 1.0);
        assert_eq!(success_at_k(&ranked, &rel(&["a"]), 1), 0.0); // a is at rank 2
                                                                 // multi-relevant: any one in top-k counts.
        assert_eq!(success_at_k(&ranked, &rel(&["a", "b"]), 3), 1.0);
        assert_eq!(success_at_k(&ranked, &rel(&["z"]), 3), 0.0);
    }

    #[test]
    fn primary_success_is_top1_only_and_opt_out() {
        let ranked = ids(&["a", "b"]);
        assert_eq!(primary_success_at_1(&ranked, Some("a")), Some(1.0));
        assert_eq!(primary_success_at_1(&ranked, Some("b")), Some(0.0));
        assert_eq!(primary_success_at_1(&ranked, None), None); // excluded from the metric
    }
}
