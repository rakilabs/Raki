//! AI provider adapters (local + cloud) and the egress/consent policy.

mod egress;
mod fake;

pub use egress::EgressPolicy;
pub use fake::FakeEmbeddingProvider;
