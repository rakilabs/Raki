//! Standalone binary to measure memory-lifecycle signal lift on the R4 seed
//! corpus using the real local embedding provider (fastembed / bge-small-en-v1.5).

use std::sync::Arc;

use raki_ai::FastEmbedProvider;
use raki_eval::memory_corpus::index::index_seed_corpus_chunked_with_embedder;
use raki_eval::memory_corpus::measure::{ablate_signals, build_signal_map, MapSignalSource};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let embedder: Arc<dyn raki_domain::EmbeddingProvider> = Arc::new(FastEmbedProvider::try_new()?);
    let (_db, _repo, keyword, vectors, embedder) =
        index_seed_corpus_chunked_with_embedder(embedder).await;
    let now = 1_000_000_000_000i64;
    let source = MapSignalSource {
        signals: build_signal_map(now),
    };

    let report = ablate_signals(&keyword, &vectors, embedder.as_ref(), &source, 10, now).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
