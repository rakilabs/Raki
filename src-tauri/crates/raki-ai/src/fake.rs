//! A deterministic, local, dependency-free embedding provider for tests and the
//! skeleton. Real fastembed/cloud providers replace this behind the same port.

use async_trait::async_trait;

use raki_domain::{DomainError, Embedding, EmbeddingProvider, Locality};

pub struct FakeEmbeddingProvider {
    dim: usize,
}

impl FakeEmbeddingProvider {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

/// Deterministic pseudo-embedding: fill from a simple rolling hash of the input.
fn embed_one(text: &str, dim: usize) -> Vec<f32> {
    let mut v = vec![0.0_f32; dim];
    let mut h: u64 = 1469598103934665603; // FNV offset basis
    for (i, byte) in text.bytes().enumerate() {
        h ^= byte as u64;
        h = h.wrapping_mul(1099511628211); // FNV prime
        let slot = i % dim;
        v[slot] += ((h % 1000) as f32) / 1000.0;
    }
    v
}

#[async_trait]
impl EmbeddingProvider for FakeEmbeddingProvider {
    fn dimension(&self) -> usize {
        self.dim
    }

    fn locality(&self) -> Locality {
        Locality::Local
    }

    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, DomainError> {
        Ok(inputs
            .iter()
            .map(|s| Embedding(embed_one(s, self.dim)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{EmbeddingProvider, Locality};

    #[tokio::test]
    async fn fake_embeddings_are_deterministic_and_sized() {
        let p = FakeEmbeddingProvider::new(8);
        assert_eq!(p.dimension(), 8);
        assert_eq!(p.locality(), Locality::Local);

        let a = p.embed(&["hello".to_string()]).await.unwrap();
        let b = p.embed(&["hello".to_string()]).await.unwrap();
        assert_eq!(a[0].0.len(), 8);
        assert_eq!(a, b, "same input yields same vector");
    }
}
