//! Standalone binary to record the pure-retrieval baseline for the R4 corpus.

use std::sync::Arc;

use raki_ai::FakeEmbeddingProvider;
use raki_domain::{body_to_text, EmbeddingProvider, NoteRepository, VectorIndex};
use raki_eval::{build_in_memory_index, memory_corpus::baseline::record_baseline};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (_db, repo, keyword, vectors) = build_in_memory_index()?;
    let embedder = Arc::new(FakeEmbeddingProvider::new(384));

    for note in raki_eval::memory_corpus::seed::seed_notes() {
        repo.upsert(&note).await?;
        let text = format!("{}\n\n{}", note.title, body_to_text(&note.body));
        let emb = embedder.embed(std::slice::from_ref(&text)).await?;
        let emb = emb
            .into_iter()
            .next()
            .expect("fake embedder returns one embedding per input");
        vectors.upsert(&note.id.to_string(), &emb).await?;
    }

    let results = record_baseline(&keyword, &vectors, embedder.as_ref(), 5).await?;
    println!("{}", serde_json::to_string_pretty(&results)?);
    Ok(())
}
