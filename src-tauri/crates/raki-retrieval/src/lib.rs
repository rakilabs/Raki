//! Hybrid retrieval: rank fusion, ranking seams, and quality metrics over the domain index ports.

mod fusion;
mod metrics;
mod rerank;
mod search;

pub use fusion::{reciprocal_rank_fusion, DEFAULT_RRF_K};
pub use metrics::{average_precision_at_k, ndcg_at_k, recall_at_k, reciprocal_rank};
pub use rerank::rerank;
pub use search::{hybrid_candidates, hybrid_search, search, vector_search};
