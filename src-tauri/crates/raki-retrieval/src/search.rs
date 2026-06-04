//! The query-time ranking seam. Today it returns keyword hits in order; when a
//! VectorIndex lands, fuse keyword + vector rankings here via `reciprocal_rank_fusion`.

use raki_domain::{DomainError, KeywordIndex};

/// Return up to `k` source ids best-matching `query`, best-first.
pub async fn search(
    keyword: &dyn KeywordIndex,
    query: &str,
    k: usize,
) -> Result<Vec<String>, DomainError> {
    let hits = keyword.query(query, k).await?;
    Ok(hits.into_iter().map(|h| h.source_id).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use raki_domain::{DomainError, KeywordHit, KeywordIndex};

    struct FakeKeyword(Vec<&'static str>);

    #[async_trait]
    impl KeywordIndex for FakeKeyword {
        async fn query(&self, _q: &str, _k: usize) -> Result<Vec<KeywordHit>, DomainError> {
            Ok(self
                .0
                .iter()
                .enumerate()
                .map(|(i, id)| KeywordHit {
                    source_id: id.to_string(),
                    score: i as f32,
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn search_returns_ids_in_index_order() {
        let index = FakeKeyword(vec!["b", "a", "c"]);
        let ids = search(&index, "anything", 10).await.unwrap();
        assert_eq!(ids, vec!["b".to_string(), "a".to_string(), "c".to_string()]);
    }
}
