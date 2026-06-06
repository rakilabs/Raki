//! The fastembed-backed `Reranker`: in-process ONNX cross-encoder, model
//! `jina-reranker-v1-turbo-en` (English, ~37M params — the smallest fastembed reranker;
//! quality differences vs larger rerankers are noise at the eval's scale, so we pick the
//! cheapest). Swap the `RerankerModel` variant to change models. Downloads once, cached.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fastembed::{RerankInitOptions, RerankerModel, TextRerank};

use raki_domain::{DomainError, Locality, RerankScore, Reranker};

/// Stable reranker model identifier (recorded in the eval baseline).
pub const RERANKER_MODEL_ID: &str = "jina-reranker-v1-turbo-en";

pub struct FastEmbedReranker {
    model: Arc<Mutex<TextRerank>>,
}

impl FastEmbedReranker {
    pub fn try_new() -> Result<Self, DomainError> {
        let model =
            TextRerank::try_new(RerankInitOptions::new(RerankerModel::JINARerankerV1TurboEn))
                .map_err(|e| DomainError::Provider(format!("fastembed reranker init: {e}")))?;
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
        })
    }
}

#[async_trait]
impl Reranker for FastEmbedReranker {
    fn locality(&self) -> Locality {
        Locality::Local
    }
    fn model_id(&self) -> String {
        RERANKER_MODEL_ID.to_string()
    }
    async fn rerank(
        &self,
        query: &str,
        documents: &[String],
    ) -> Result<Vec<RerankScore>, DomainError> {
        if documents.is_empty() {
            return Ok(Vec::new());
        }
        let model = self.model.clone();
        let q = query.to_string();
        let docs = documents.to_vec();
        let results = tokio::task::spawn_blocking(move || {
            let mut guard = model.lock().unwrap();
            let refs: Vec<&str> = docs.iter().map(|s| s.as_str()).collect();
            // return_documents = false (we only need index + score); default batch size.
            guard.rerank(q.as_str(), refs, false, None)
        })
        .await
        .map_err(|e| DomainError::Provider(format!("rerank join: {e}")))?
        .map_err(|e| DomainError::Provider(format!("rerank: {e}")))?;
        Ok(results
            .into_iter()
            .map(|r| RerankScore {
                index: r.index,
                score: r.score,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "downloads the jina-reranker model on first run; run explicitly with --ignored"]
    async fn fastembed_reranker_orders_relevant_first_and_survives_edges() {
        let r = FastEmbedReranker::try_new().expect("reranker init");
        assert_eq!(r.locality(), Locality::Local);
        assert_eq!(r.model_id(), "jina-reranker-v1-turbo-en");

        // Relevance ordering: the panda doc should outscore the unrelated one.
        let docs = vec![
            "the giant panda is a bear endemic to china".to_string(),
            "mortgage refinance break-even is closing costs over monthly savings".to_string(),
        ];
        let scores = r.rerank("what is a panda?", &docs).await.unwrap();
        assert_eq!(scores.len(), 2);
        let panda = scores.iter().find(|s| s.index == 0).unwrap().score;
        let other = scores.iter().find(|s| s.index == 1).unwrap().score;
        assert!(panda > other, "relevant doc scores higher");

        // Edge cases (where real ONNX rerankers panic): empty pool, empty doc, oversized text.
        assert!(r.rerank("q", &[]).await.unwrap().is_empty());
        let big = "lorem ipsum ".repeat(4000); // far past the model's token window
        let edge = vec!["".to_string(), big];
        let out = r.rerank("anything", &edge).await.unwrap();
        assert_eq!(
            out.len(),
            2,
            "no panic on empty/oversized docs; one score each"
        );
        assert!(out.iter().all(|s| s.score.is_finite()), "no NaN/inf scores");
    }
}
