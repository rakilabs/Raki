//! AI provider adapters (local + cloud) and the egress/consent policy.

mod egress;
mod fake;
mod fastembed;

pub use egress::EgressPolicy;
pub use fake::FakeEmbeddingProvider;
pub use fastembed::FastEmbedProvider;
