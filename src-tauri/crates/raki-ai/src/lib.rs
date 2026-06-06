//! AI provider adapters (local + cloud) and the egress/consent policy.

mod egress;
mod fake;
mod fake_rerank;
mod fastembed;
mod rerank;

pub use egress::EgressPolicy;
pub use fake::FakeEmbeddingProvider;
pub use fake_rerank::FakeReranker;
pub use fastembed::FastEmbedProvider;
pub use rerank::{FastEmbedReranker, RERANKER_MODEL_ID};
