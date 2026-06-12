//! Build an in-memory search index over the seed corpus for evaluation.

use std::sync::Arc;

use raki_ai::FakeEmbeddingProvider;
use raki_domain::{body_to_text, EmbeddingProvider, NoteRepository, VectorIndex};
use raki_storage::{Database, SqliteKeywordIndex, SqliteNoteRepository, SqliteVectorIndex};

use crate::build_in_memory_index;
use crate::memory_corpus::seed::seed_notes;

/// Build a fresh in-memory SQLite index, upsert every seed note, and embed the
/// whole-note text with a deterministic fake embedder.
///
/// The returned `Database` must be kept alive for the in-memory stores to
/// remain usable.
pub async fn index_seed_corpus() -> (
    Database,
    SqliteNoteRepository,
    SqliteKeywordIndex,
    SqliteVectorIndex,
    Arc<FakeEmbeddingProvider>,
) {
    let (db, repo, keyword, vectors) =
        build_in_memory_index().expect("in-memory index creation succeeds");
    let embedder = Arc::new(FakeEmbeddingProvider::new(384));

    for note in seed_notes() {
        let text = format!("{}\n\n{}", note.title, body_to_text(&note.body));
        repo.upsert(&note).await.expect("seed note upsert succeeds");
        let emb = embedder
            .embed(std::slice::from_ref(&text))
            .await
            .expect("fake embedder succeeds");
        let emb = emb
            .into_iter()
            .next()
            .expect("fake embedder returns one embedding per input");
        vectors
            .upsert(&note.id.to_string(), &emb)
            .await
            .expect("vector upsert succeeds");
    }

    (db, repo, keyword, vectors, embedder)
}
