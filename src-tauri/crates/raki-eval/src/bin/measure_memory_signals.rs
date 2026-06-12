//! Standalone binary to measure memory-lifecycle signal lift on the R4 seed corpus.

use raki_eval::memory_corpus::index::index_seed_corpus;
use raki_eval::memory_corpus::measure::{ablate_signals, build_signal_map, MapSignalSource};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (_db, _repo, keyword, vectors, embedder) = index_seed_corpus().await;
    let now = 1_000_000_000_000i64;
    let source = MapSignalSource {
        signals: build_signal_map(now),
    };

    let report = ablate_signals(&keyword, &vectors, embedder.as_ref(), &source, 10, now).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
