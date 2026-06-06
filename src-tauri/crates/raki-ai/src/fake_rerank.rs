//! `FakeReranker`: a deterministic, model-free reranker stub for offline `run_eval` and
//! unit tests.
//!
//! ORCHESTRATION STUB ONLY. It scores by query/document token overlap, which is
//! STRUCTURALLY UNCORRELATED with how a real cross-encoder scores (query, doc) pairs. It
//! exists to prove the plumbing — index→id mapping, truncation, empty pools — runs
//! deterministically without loading a model. It says NOTHING about real reranking quality
//! or real-model failure modes; those are validated only by `FastEmbedReranker`'s
//! `#[ignore]` integration test.

use std::collections::HashSet;

use async_trait::async_trait;

use raki_domain::{DomainError, Locality, RerankScore, Reranker};

pub struct FakeReranker;

/// Lowercase ascii-alphanumeric token set. Pure; shared shape with the harness's intent.
fn tokens(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect()
}

#[async_trait]
impl Reranker for FakeReranker {
    fn locality(&self) -> Locality {
        Locality::Local
    }
    fn model_id(&self) -> String {
        "fake-reranker".to_string()
    }
    async fn rerank(
        &self,
        query: &str,
        documents: &[String],
    ) -> Result<Vec<RerankScore>, DomainError> {
        let q = tokens(query);
        Ok(documents
            .iter()
            .enumerate()
            .map(|(index, doc)| {
                let overlap = tokens(doc).intersection(&q).count();
                RerankScore {
                    index,
                    score: overlap as f32,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn fake_reranker_scores_by_token_overlap_deterministically() {
        let docs = vec![
            "nothing relevant here".to_string(), // 0 overlap with "red apple"
            "a red apple and a green apple".to_string(), // overlaps red, apple
            "red things".to_string(),            // overlaps red
        ];
        let a = FakeReranker.rerank("red apple", &docs).await.unwrap();
        let b = FakeReranker.rerank("red apple", &docs).await.unwrap();
        assert_eq!(a, b, "deterministic");
        assert_eq!(a[0].score, 0.0);
        assert!(a[1].score > a[2].score, "more overlap scores higher");
    }
}
