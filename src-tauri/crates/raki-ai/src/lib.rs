//! AI provider adapters (local + cloud) and the egress/consent policy.

pub mod egress;
mod fake;
mod fake_rerank;
mod fastembed;
mod messages;
mod query_rewrite;
mod rerank;
pub mod testing;

pub use egress::AuditGate;
pub use fake::FakeEmbeddingProvider;
pub use fake_rerank::FakeReranker;
pub use fastembed::FastEmbedProvider;
pub use messages::MessagesProvider;
pub use query_rewrite::CloudQueryRewriter;
pub use rerank::{FastEmbedReranker, RERANKER_MODEL_ID};
