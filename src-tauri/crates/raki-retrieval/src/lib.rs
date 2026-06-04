//! Hybrid retrieval: rank fusion and ranking over the domain index ports.

mod fusion;
mod search;

pub use fusion::{reciprocal_rank_fusion, DEFAULT_RRF_K};
pub use search::search;
