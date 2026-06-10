//! The precision seam: reorder the recall union by a cross-encoder `Reranker`. Pure —
//! depends only on the port, never on a concrete model.

use raki_domain::{DomainError, Reranker};

/// Reorder `candidates` ((id, text) — the recall union) by reranker score, best-first,
/// and return the top-`k` ids. Equal scores preserve the candidates' incoming order
/// (stable sort), so the recall ranking is the tie-break.
pub async fn rerank(
    reranker: &dyn Reranker,
    query: &str,
    candidates: &[(String, String)],
    k: usize,
) -> Result<Vec<String>, DomainError> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let docs: Vec<String> = candidates.iter().map(|(_, text)| text.clone()).collect();
    let mut scored = reranker.rerank(query, &docs).await?;
    // Stable sort by score descending; NaN treated as lowest (Equal keeps incoming order).
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(scored
        .iter()
        .filter_map(|s| candidates.get(s.index).map(|(id, _)| id.clone()))
        .take(k)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use raki_domain::{Locality, RerankScore};

    /// Scores each doc by its index (higher index = higher score), so it REVERSES the
    /// incoming order — proving rerank actually reorders by score, not position.
    struct ReverseReranker;
    #[async_trait]
    impl Reranker for ReverseReranker {
        fn locality(&self) -> Locality {
            Locality::Local
        }
        fn model_id(&self) -> String {
            "reverse".to_string()
        }
        async fn rerank(&self, _q: &str, docs: &[String]) -> Result<Vec<RerankScore>, DomainError> {
            Ok(docs
                .iter()
                .enumerate()
                .map(|(i, _)| RerankScore {
                    index: i,
                    score: i as f32,
                })
                .collect())
        }
    }

    fn cands(ids: &[&str]) -> Vec<(String, String)> {
        ids.iter()
            .map(|id| (id.to_string(), format!("text-{id}")))
            .collect()
    }

    #[tokio::test]
    async fn rerank_reorders_by_score_desc_and_truncates() {
        let out = rerank(&ReverseReranker, "q", &cands(&["a", "b", "c"]), 2)
            .await
            .unwrap();
        assert_eq!(
            out,
            vec!["c".to_string(), "b".to_string()],
            "highest score first, top-2"
        );
    }

    #[tokio::test]
    async fn rerank_empty_candidates_is_empty() {
        let out = rerank(&ReverseReranker, "q", &[], 3).await.unwrap();
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn rerank_k_larger_than_len_returns_all_reordered() {
        let out = rerank(&ReverseReranker, "q", &cands(&["a", "b"]), 10)
            .await
            .unwrap();
        assert_eq!(out, vec!["b".to_string(), "a".to_string()]);
    }

    /// Returns one in-range score and one OUT-OF-RANGE index, to prove the wrapper skips
    /// the bad index instead of panicking on `candidates[s.index]`.
    struct OobReranker;

    #[async_trait]
    impl Reranker for OobReranker {
        fn locality(&self) -> Locality {
            Locality::Local
        }
        fn model_id(&self) -> String {
            "oob".to_string()
        }
        async fn rerank(
            &self,
            _query: &str,
            _documents: &[String],
        ) -> Result<Vec<RerankScore>, DomainError> {
            Ok(vec![
                RerankScore {
                    index: 0,
                    score: 0.9,
                },
                RerankScore {
                    index: 99,
                    score: 0.8,
                }, // out of range for 1 candidate
            ])
        }
    }

    #[tokio::test]
    async fn rerank_skips_out_of_range_index_without_panicking() {
        let candidates = vec![("id0".to_string(), "doc zero".to_string())];
        let ids = rerank(&OobReranker, "q", &candidates, 10).await.unwrap();
        assert_eq!(
            ids,
            vec!["id0".to_string()],
            "OOB index skipped, in-range kept"
        );
    }
}
