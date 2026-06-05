//! The fastembed-backed EmbeddingProvider: in-process ONNX, model `bge-small-en-v1.5`
//! (384-dim). The model downloads once on first construction and is cached on disk.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use raki_domain::{DomainError, Embedding, EmbeddingProvider, Locality};

/// Stable model identifier stored alongside embeddings (drives staleness).
pub const MODEL_ID: &str = "bge-small-en-v1.5";
/// bge models want a query instruction prefix on the QUERY side only. Document
/// embeddings (the pipeline's path) are embedded as-is; the query-issuing layer
/// (retrieval/eval, later slices) applies this prefix. Exposed here for reuse.
pub const BGE_QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

/// Prepend the bge query instruction to each query. Pure (model-free) so it is unit
/// testable without downloading the model.
fn apply_query_prefix(queries: &[String]) -> Vec<String> {
    queries
        .iter()
        .map(|q| format!("{BGE_QUERY_PREFIX}{q}"))
        .collect()
}

pub struct FastEmbedProvider {
    model: Arc<Mutex<TextEmbedding>>,
}

impl FastEmbedProvider {
    pub fn try_new() -> Result<Self, DomainError> {
        let model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::BGESmallENV15))
            .map_err(|e| DomainError::Provider(format!("fastembed init: {e}")))?;
        Ok(Self {
            model: Arc::new(Mutex::new(model)),
        })
    }
}

#[async_trait]
impl EmbeddingProvider for FastEmbedProvider {
    fn dimension(&self) -> usize {
        384
    }

    fn locality(&self) -> Locality {
        Locality::Local
    }

    fn model_id(&self) -> String {
        MODEL_ID.to_string()
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, DomainError> {
        let model = self.model.clone();
        let owned: Vec<String> = inputs.to_vec();
        let vectors = tokio::task::spawn_blocking(move || model.lock().unwrap().embed(owned, None))
            .await
            .map_err(|e| DomainError::Provider(format!("embed join: {e}")))?
            .map_err(|e| DomainError::Provider(format!("embed: {e}")))?;
        Ok(vectors.into_iter().map(Embedding).collect())
    }

    async fn embed_query(&self, queries: &[String]) -> Result<Vec<Embedding>, DomainError> {
        self.embed(&apply_query_prefix(queries)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{EmbeddingProvider, Locality};

    #[test]
    fn query_prefix_is_applied_to_each_query() {
        let out = apply_query_prefix(&["apples".to_string(), "oranges".to_string()]);
        assert_eq!(out[0], format!("{BGE_QUERY_PREFIX}apples"));
        assert_eq!(out[1], format!("{BGE_QUERY_PREFIX}oranges"));
    }

    #[tokio::test]
    #[ignore = "downloads the bge-small model on first run; run explicitly with --ignored"]
    async fn fastembed_smoke_produces_384_dim_distinct_vectors() {
        let p = FastEmbedProvider::try_new().expect("model init");
        assert_eq!(p.dimension(), 384);
        assert_eq!(p.locality(), Locality::Local);
        assert_eq!(p.model_id(), "bge-small-en-v1.5");

        let out = p
            .embed(&[
                "apples are red".to_string(),
                "the stock market fell".to_string(),
            ])
            .await
            .unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].0.len(), 384);
        assert_ne!(out[0].0, out[1].0, "different text → different vectors");
    }
}
