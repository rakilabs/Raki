//! Standalone binary to record the pure-retrieval baseline for the R4 corpus
//! using the real local embedding provider (fastembed / bge-small-en-v1.5).

use std::sync::Arc;

use raki_ai::FastEmbedProvider;
use raki_eval::memory_corpus::baseline::record_baseline;
use raki_eval::memory_corpus::index::index_seed_corpus_chunked_with_embedder;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let embedder: Arc<dyn raki_domain::EmbeddingProvider> = Arc::new(FastEmbedProvider::try_new()?);
    let (_db, _repo, keyword, vectors, embedder) =
        index_seed_corpus_chunked_with_embedder(embedder).await;

    let results = record_baseline(&keyword, &vectors, embedder.as_ref(), 10).await?;
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}
