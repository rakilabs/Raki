//! The query-time ranking seams: `search` (keyword), `vector_search` (semantic), and
//! `hybrid_search`, a vector-primary recall union of the two.

use raki_domain::{DomainError, EmbeddingProvider, KeywordIndex, VectorIndex};

/// Candidate depth pulled from each retriever before merging. The keyword pool needs depth
/// so its backfill ids are available; the merged result is still truncated to `k`.
const HYBRID_CANDIDATE_POOL: usize = 20;

/// Hybrid retrieval, **vector-primary**. The embedding model is the stronger retriever on
/// clean text, so vector's ranking is authoritative and keyword results only *backfill*
/// ids vector did not already return. This guarantees hybrid is never worse than vector
/// alone (the eval showed score-fusion regressing — vector has no headroom to beat here),
/// while keyword still gives graceful degradation (cold start before embeddings finish)
/// and exact-token / out-of-vocab coverage. A future cross-encoder rerank stage will
/// reorder this candidate union for precision; see ADR-0006.
pub async fn hybrid_search(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    query: &str,
    k: usize,
) -> Result<Vec<String>, DomainError> {
    let pool = k.max(HYBRID_CANDIDATE_POOL);
    let mut out = vector_search(vectors, embedder, query, pool).await?;
    for id in search(keyword, query, pool).await? {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    out.truncate(k);
    Ok(out)
}

/// Return up to `k` source ids best-matching `query`, best-first.
pub async fn search(
    keyword: &dyn KeywordIndex,
    query: &str,
    k: usize,
) -> Result<Vec<String>, DomainError> {
    let hits = keyword.query(query, k).await?;
    Ok(hits.into_iter().map(|h| h.source_id).collect())
}

/// Embed `query` (query-side) and return up to `k` nearest source ids, best-first.
pub async fn vector_search(
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    query: &str,
    k: usize,
) -> Result<Vec<String>, DomainError> {
    let mut embedded = embedder.embed_query(&[query.to_string()]).await?;
    let emb = embedded
        .pop()
        .ok_or_else(|| DomainError::Provider("empty query embedding".to_string()))?;
    let hits = vectors.query(&emb, k).await?;
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

    use raki_domain::{Embedding, EmbeddingProvider, Locality, VectorHit, VectorIndex};

    struct FakeEmbed;
    #[async_trait]
    impl EmbeddingProvider for FakeEmbed {
        fn dimension(&self) -> usize {
            2
        }
        fn locality(&self) -> Locality {
            Locality::Local
        }
        fn model_id(&self) -> String {
            "fake".to_string()
        }
        async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, DomainError> {
            Ok(inputs.iter().map(|_| Embedding(vec![1.0, 0.0])).collect())
        }
    }

    struct FakeVectors(Vec<&'static str>);
    #[async_trait]
    impl VectorIndex for FakeVectors {
        async fn upsert(&self, _id: &str, _e: &Embedding) -> Result<(), DomainError> {
            Ok(())
        }
        async fn query(&self, _e: &Embedding, _k: usize) -> Result<Vec<VectorHit>, DomainError> {
            Ok(self
                .0
                .iter()
                .enumerate()
                .map(|(i, id)| VectorHit {
                    source_id: id.to_string(),
                    distance: i as f32,
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn vector_search_returns_ids_best_first() {
        let vectors = FakeVectors(vec!["x", "y"]);
        let ids = vector_search(&vectors, &FakeEmbed, "q", 10).await.unwrap();
        assert_eq!(ids, vec!["x".to_string(), "y".to_string()]);
    }

    #[tokio::test]
    async fn hybrid_is_vector_primary_with_keyword_backfill() {
        // vector: [b, c] is authoritative; keyword-only "a" backfills after it.
        let keyword = FakeKeyword(vec!["a", "b"]);
        let vectors = FakeVectors(vec!["b", "c"]);
        let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, "q", 3)
            .await
            .unwrap();
        assert_eq!(
            ids,
            vec!["b".to_string(), "c".to_string(), "a".to_string()],
            "vector order preserved, keyword-only ids appended"
        );
    }

    #[tokio::test]
    async fn hybrid_falls_back_to_keyword_when_no_vectors() {
        // Cold start: embeddings not computed yet, so vector returns nothing and keyword
        // carries the results — search still works.
        let keyword = FakeKeyword(vec!["a", "b"]);
        let vectors = FakeVectors(vec![]);
        let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, "q", 3)
            .await
            .unwrap();
        assert_eq!(ids, vec!["a".to_string(), "b".to_string()]);
    }
}
