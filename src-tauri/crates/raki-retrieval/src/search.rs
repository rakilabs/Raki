//! The query-time ranking seams: `search` (keyword), `vector_search` (semantic), and
//! `hybrid_search`, a vector-primary recall union of the two.

use std::collections::HashSet;

use raki_domain::{DomainError, EmbeddingProvider, KeywordIndex, NoteId, VectorIndex};

/// Strip the chunk suffix (`#<n>`) and parse the leading note ID.
fn note_id_from_chunk(chunk_id: &str) -> Result<NoteId, DomainError> {
    let raw = chunk_id.split('#').next().unwrap_or(chunk_id);
    NoteId::parse(raw)
}

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
/// The recall **union** — vector-primary, keyword-backfilled — UNtruncated. This is the
/// candidate pool the precision stage (rerank) reorders. `pool` is the depth pulled from
/// each retriever; the union is at least `HYBRID_CANDIDATE_POOL` deep so backfill ids exist.
pub async fn hybrid_candidates(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    rewriter: Option<&dyn raki_domain::QueryRewriter>,
    query: &str,
    pool: usize,
) -> Result<Vec<NoteId>, DomainError> {
    let depth = pool.max(HYBRID_CANDIDATE_POOL);
    let effective_query = resolve_query(rewriter, query).await?;
    let mut out: Vec<NoteId> = Vec::new();
    let mut seen: HashSet<NoteId> = HashSet::new();

    for chunk_id in vector_search(vectors, embedder, &effective_query, depth).await? {
        let note_id = note_id_from_chunk(&chunk_id)?;
        if seen.insert(note_id) {
            out.push(note_id);
        }
    }

    for id in search(keyword, &effective_query, depth).await? {
        let note_id = NoteId::parse(&id)?;
        if seen.insert(note_id) {
            out.push(note_id);
        }
    }

    Ok(out)
}

/// Hybrid retrieval, **vector-primary**: `hybrid_candidates` truncated to `k`. The embedding
/// model is the stronger retriever on clean text, so vector's ranking is authoritative and
/// keyword only *backfills* ids vector did not return — provably never worse than vector
/// alone, while keyword gives cold-start and exact-token coverage. The cross-encoder rerank
/// stage (eval, ADR-0006) reorders `hybrid_candidates` for precision.
pub async fn hybrid_search(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    rewriter: Option<&dyn raki_domain::QueryRewriter>,
    query: &str,
    k: usize,
) -> Result<Vec<NoteId>, DomainError> {
    let mut out = hybrid_candidates(keyword, vectors, embedder, rewriter, query, k).await?;
    out.truncate(k);
    Ok(out)
}

/// A note id paired with its retrieval score.
pub struct ScoredNote {
    pub note_id: NoteId,
    pub retrieval_score: f64,
}

/// Hybrid retrieval with rank-derived scores: `1.0 / (1 + rank)`.
/// Vector results are ranked first; keyword backfill continues the ranking.
pub async fn hybrid_candidates_scored(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    rewriter: Option<&dyn raki_domain::QueryRewriter>,
    query: &str,
    top_k: usize,
) -> Result<Vec<ScoredNote>, DomainError> {
    let ids = hybrid_candidates(keyword, vectors, embedder, rewriter, query, top_k).await?;
    let scored: Vec<ScoredNote> = ids
        .into_iter()
        .take(top_k)
        .enumerate()
        .map(|(rank, note_id)| ScoredNote {
            note_id,
            retrieval_score: 1.0 / (1.0 + rank as f64),
        })
        .collect();
    Ok(scored)
}

/// Hybrid retrieval boosted by memory-lifecycle signals. Off by default; used for R4 measurement.
#[allow(clippy::too_many_arguments)]
pub async fn hybrid_search_with_signals(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    rewriter: Option<&dyn raki_domain::QueryRewriter>,
    signal_source: &dyn raki_domain::SignalSource,
    booster: &dyn raki_domain::SignalBooster,
    query: &str,
    k: usize,
    now_ms: i64,
) -> Result<Vec<NoteId>, DomainError> {
    let scored = hybrid_candidates_scored(keyword, vectors, embedder, rewriter, query, k).await?;
    if scored.is_empty() {
        return Ok(Vec::new());
    }
    let note_ids: Vec<NoteId> = scored.iter().map(|s| s.note_id).collect();
    let signals = signal_source.get(&note_ids).await?;

    let mut boosted: Vec<(NoteId, f64)> = scored
        .into_iter()
        .map(|s| {
            let sig = signals.get(&s.note_id).cloned().unwrap_or_default();
            let (boosted_score, _) = booster.boost(s.retrieval_score, &sig, now_ms);
            (s.note_id, boosted_score)
        })
        .collect();

    boosted.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(boosted.into_iter().map(|(id, _)| id).collect())
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

async fn resolve_query(
    rewriter: Option<&dyn raki_domain::QueryRewriter>,
    query: &str,
) -> Result<String, DomainError> {
    match rewriter {
        Some(r) => {
            let u = r.understand(query).await?;
            if !u.is_fallback && !u.rewritten_query.trim().is_empty() {
                if u.needs_multi_hop && !u.sub_queries.is_empty() {
                    Ok(u.sub_queries[0].clone()) // stub: use first sub-query
                } else {
                    Ok(u.rewritten_query)
                }
            } else {
                // The rewriter explicitly returned a fallback (e.g. confidence 0, "no change").
                // This is the model's intentional decision, not an error.
                Ok(query.to_string())
            }
        }
        None => Ok(query.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use raki_domain::{DomainError, KeywordHit, KeywordIndex, QueryUnderstanding};

    // Stable UUIDs used in place of single-letter ids so NoteId::parse succeeds.
    const ID_A: &str = "00000000-0000-0000-0000-00000000000a";
    const ID_B: &str = "00000000-0000-0000-0000-00000000000b";
    const ID_C: &str = "00000000-0000-0000-0000-00000000000c";
    const ID_D: &str = "00000000-0000-0000-0000-00000000000d";
    const ID_E: &str = "00000000-0000-0000-0000-00000000000e";
    const ID_X: &str = "00000000-0000-0000-0000-00000000000x";
    const ID_Y: &str = "00000000-0000-0000-0000-00000000000y";

    fn nid(s: &str) -> NoteId {
        NoteId::parse(s).unwrap()
    }

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
        let index = FakeKeyword(vec![ID_B, ID_A, ID_C]);
        let ids = search(&index, "anything", 10).await.unwrap();
        assert_eq!(
            ids,
            vec![ID_B.to_string(), ID_A.to_string(), ID_C.to_string()]
        );
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

    struct FakeVectors(Vec<String>);
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
                    source_id: id.clone(),
                    distance: i as f32,
                })
                .collect())
        }
        async fn delete_by_prefix(&self, _prefix: &str) -> Result<(), DomainError> {
            Ok(())
        }
        async fn upsert_batch(&self, _items: &[(String, Embedding)]) -> Result<(), DomainError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn vector_search_returns_ids_best_first() {
        let vectors = FakeVectors(vec![ID_X.to_string(), ID_Y.to_string()]);
        let ids = vector_search(&vectors, &FakeEmbed, "q", 10).await.unwrap();
        assert_eq!(ids, vec![ID_X.to_string(), ID_Y.to_string()]);
    }

    #[tokio::test]
    async fn hybrid_search_output_is_characterized() {
        // Vector is authoritative [c, b, e]; keyword [a, b, c, d] backfills only the
        // ids vector missed (a, d), in keyword order, after the vector block.
        let keyword = FakeKeyword(vec![ID_A, ID_B, ID_C, ID_D]);
        let vectors = FakeVectors(vec![ID_C.to_string(), ID_B.to_string(), ID_E.to_string()]);
        let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, None, "q", 4)
            .await
            .unwrap();
        assert_eq!(
            ids,
            vec![nid(ID_C), nid(ID_B), nid(ID_E), nid(ID_A)],
            "vector order preserved; keyword-only ids backfill in order; truncated to k=4"
        );
    }

    #[tokio::test]
    async fn hybrid_candidates_returns_the_untruncated_union() {
        let keyword = FakeKeyword(vec![ID_A, ID_B]);
        let vectors = FakeVectors(vec![ID_B.to_string(), ID_C.to_string()]);
        let ids = hybrid_candidates(&keyword, &vectors, &FakeEmbed, None, "q", 20)
            .await
            .unwrap();
        assert_eq!(
            ids,
            vec![nid(ID_B), nid(ID_C), nid(ID_A)],
            "full union, vector-first, keyword backfill, no truncation"
        );
    }

    #[tokio::test]
    async fn hybrid_is_vector_primary_with_keyword_backfill() {
        // vector: [b, c] is authoritative; keyword-only "a" backfills after it.
        let keyword = FakeKeyword(vec![ID_A, ID_B]);
        let vectors = FakeVectors(vec![ID_B.to_string(), ID_C.to_string()]);
        let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, None, "q", 3)
            .await
            .unwrap();
        assert_eq!(
            ids,
            vec![nid(ID_B), nid(ID_C), nid(ID_A)],
            "vector order preserved, keyword-only ids appended"
        );
    }

    #[tokio::test]
    async fn hybrid_falls_back_to_keyword_when_no_vectors() {
        // Cold start: embeddings not computed yet, so vector returns nothing and keyword
        // carries the results — search still works.
        let keyword = FakeKeyword(vec![ID_A, ID_B]);
        let vectors = FakeVectors(vec![]);
        let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, None, "q", 3)
            .await
            .unwrap();
        assert_eq!(ids, vec![nid(ID_A), nid(ID_B)]);
    }

    #[tokio::test]
    async fn hybrid_candidates_rolls_up_chunk_ids_with_min_rank() {
        // Vector returns chunk IDs for the same note twice; first occurrence (min-rank) wins.
        let keyword = FakeKeyword(vec![ID_C]); // backfill
        let vectors = FakeVectors(vec![
            format!("{ID_A}#0"),
            format!("{ID_B}#1"),
            format!("{ID_A}#2"),
        ]);
        let ids = hybrid_candidates(&keyword, &vectors, &FakeEmbed, None, "q", 20)
            .await
            .unwrap();
        assert_eq!(
            ids,
            vec![nid(ID_A), nid(ID_B), nid(ID_C)],
            "duplicate note-a rolled up to first (min-rank) occurrence; keyword backfills c"
        );
    }

    #[tokio::test]
    async fn hybrid_candidates_never_emits_raw_chunk_ids() {
        let keyword = FakeKeyword(vec![]);
        let vectors = FakeVectors(vec![format!("{ID_A}#0"), format!("{ID_B}#1")]);
        let ids = hybrid_candidates(&keyword, &vectors, &FakeEmbed, None, "q", 20)
            .await
            .unwrap();
        for id in &ids {
            assert!(
                !id.to_string().contains('#'),
                "output must be bare NoteId, not a chunk ID"
            );
        }
        assert_eq!(ids, vec![nid(ID_A), nid(ID_B)]);
    }

    struct FakeRewriter {
        query: &'static str,
        is_fallback: bool,
    }
    #[async_trait]
    impl raki_domain::QueryRewriter for FakeRewriter {
        async fn understand(&self, _query: &str) -> Result<QueryUnderstanding, DomainError> {
            Ok(QueryUnderstanding {
                rewritten_query: self.query.to_string(),
                needs_multi_hop: false,
                sub_queries: vec![],
                confidence: 0.9,
                is_fallback: self.is_fallback,
            })
        }
    }

    struct FallbackRewriter;
    #[async_trait]
    impl raki_domain::QueryRewriter for FallbackRewriter {
        async fn understand(&self, _query: &str) -> Result<QueryUnderstanding, DomainError> {
            Ok(QueryUnderstanding {
                rewritten_query: "ignored".to_string(),
                needs_multi_hop: false,
                sub_queries: vec![],
                confidence: 0.9,
                is_fallback: true,
            })
        }
    }

    struct ErrorRewriter;
    #[async_trait]
    impl raki_domain::QueryRewriter for ErrorRewriter {
        async fn understand(&self, _query: &str) -> Result<QueryUnderstanding, DomainError> {
            Err(DomainError::Provider("test error".to_string()))
        }
    }

    struct MultiHopRewriter;
    #[async_trait]
    impl raki_domain::QueryRewriter for MultiHopRewriter {
        async fn understand(&self, _query: &str) -> Result<QueryUnderstanding, DomainError> {
            Ok(QueryUnderstanding {
                rewritten_query: "ignored".to_string(),
                needs_multi_hop: true,
                sub_queries: vec!["sub_a".to_string(), "sub_b".to_string()],
                confidence: 0.9,
                is_fallback: false,
            })
        }
    }

    #[tokio::test]
    async fn hybrid_search_uses_rewritten_query_when_rewriter_provided() {
        let keyword = FakeKeyword(vec![ID_A]);
        let vectors = FakeVectors(vec![ID_A.to_string()]);
        let rewriter = FakeRewriter {
            query: "explicit keyword",
            is_fallback: false,
        };
        let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, Some(&rewriter), "vague", 3)
            .await
            .unwrap();
        assert_eq!(ids, vec![nid(ID_A)]);
    }

    #[tokio::test]
    async fn hybrid_search_falls_back_when_rewriter_returns_empty_string() {
        let keyword = FakeKeyword(vec![ID_A]);
        let vectors = FakeVectors(vec![]);
        let rewriter = FakeRewriter {
            query: "",
            is_fallback: false,
        };
        let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, Some(&rewriter), "vague", 3)
            .await
            .unwrap();
        assert_eq!(ids, vec![nid(ID_A)]);
    }

    #[tokio::test]
    async fn hybrid_search_falls_back_when_rewriter_returns_fallback() {
        let keyword = FakeKeyword(vec![ID_A]);
        let vectors = FakeVectors(vec![]);
        let rewriter = FallbackRewriter;
        let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, Some(&rewriter), "vague", 3)
            .await
            .unwrap();
        assert_eq!(ids, vec![nid(ID_A)]);
    }

    #[tokio::test]
    async fn hybrid_search_propagates_rewriter_error() {
        let keyword = FakeKeyword(vec![ID_A]);
        let vectors = FakeVectors(vec![]);
        let rewriter = ErrorRewriter;
        let err = hybrid_search(&keyword, &vectors, &FakeEmbed, Some(&rewriter), "vague", 3)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("test error"));
    }

    #[tokio::test]
    async fn hybrid_search_uses_first_subquery_when_multi_hop() {
        let keyword = FakeKeyword(vec![ID_A]);
        let vectors = FakeVectors(vec![]);
        let rewriter = MultiHopRewriter;
        let ids = hybrid_search(&keyword, &vectors, &FakeEmbed, Some(&rewriter), "vague", 3)
            .await
            .unwrap();
        // sub_a matches via FakeKeyword
        assert_eq!(ids, vec![nid(ID_A)]);
    }

    #[tokio::test]
    async fn hybrid_candidates_scored_returns_rank_derived_scores() {
        let keyword = FakeKeyword(vec![ID_A, ID_B]);
        let vectors = FakeVectors(vec![ID_B.to_string(), ID_C.to_string()]);
        let scored = hybrid_candidates_scored(&keyword, &vectors, &FakeEmbed, None, "q", 3)
            .await
            .unwrap();
        assert_eq!(scored.len(), 3);
        assert_eq!(scored[0].note_id, nid(ID_B));
        assert!(scored[0].retrieval_score > scored[1].retrieval_score);
    }
}
