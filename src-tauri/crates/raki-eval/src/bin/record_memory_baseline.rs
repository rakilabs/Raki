//! Standalone binary to record the pure-retrieval baseline for the R4 corpus.

use raki_eval::memory_corpus::baseline::record_baseline;
use raki_eval::memory_corpus::index::index_seed_corpus;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (_db, _repo, keyword, vectors, embedder) = index_seed_corpus().await;

    let results = record_baseline(&keyword, &vectors, embedder.as_ref(), 5).await?;
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}
