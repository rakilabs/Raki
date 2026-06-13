//! Build an in-memory search index over the seed corpus for evaluation.

use std::sync::Arc;

use raki_domain::{body_to_text, EmbeddingProvider, NoteRepository, VectorIndex};
use raki_storage::{Database, SqliteKeywordIndex, SqliteNoteRepository, SqliteVectorIndex};

use crate::build_in_memory_index;
use crate::memory_corpus::seed::seed_notes;

const USE_CONTEXTUAL_PREFIX: bool = false;

/// Build a fresh in-memory SQLite index, upsert every seed note, and embed the
/// whole-note text with the provided embedder.
///
/// The returned `Database` must be kept alive for the in-memory stores to
/// remain usable.
pub async fn index_seed_corpus_with_embedder(
    embedder: Arc<dyn EmbeddingProvider>,
) -> (
    Database,
    SqliteNoteRepository,
    SqliteKeywordIndex,
    SqliteVectorIndex,
    Arc<dyn EmbeddingProvider>,
) {
    let (db, repo, keyword, vectors) =
        build_in_memory_index().expect("in-memory index creation succeeds");

    for note in seed_notes() {
        let text = format!("{}\n\n{}", note.title, body_to_text(&note.body));
        repo.upsert(&note).await.expect("seed note upsert succeeds");
        let emb = embedder
            .embed(std::slice::from_ref(&text))
            .await
            .expect("embedder succeeds");
        let emb = emb
            .into_iter()
            .next()
            .expect("embedder returns one embedding per input");
        vectors
            .upsert(&note.id.to_string(), &emb)
            .await
            .expect("vector upsert succeeds");
    }

    (db, repo, keyword, vectors, embedder)
}

/// Build a fresh in-memory SQLite index over the seed corpus using the
/// provided embedder and production chunking (`body_to_blocks` + `cap_split`,
/// contextual prefix off by default). Each chunk is stored as `note_id#i`.
pub async fn index_seed_corpus_chunked_with_embedder(
    embedder: Arc<dyn EmbeddingProvider>,
) -> (
    Database,
    SqliteNoteRepository,
    SqliteKeywordIndex,
    SqliteVectorIndex,
    Arc<dyn EmbeddingProvider>,
) {
    let (db, repo, keyword, vectors) =
        build_in_memory_index().expect("in-memory index creation succeeds");

    for note in seed_notes() {
        repo.upsert(&note).await.expect("seed note upsert succeeds");

        let chunks = raki_memory::chunk_note(&note.title, &note.body, USE_CONTEXTUAL_PREFIX);
        if chunks.is_empty() {
            // Fallback: index the whole-note text if chunking produced nothing usable.
            let text = format!("{}\n\n{}", note.title, body_to_text(&note.body));
            let emb = embedder
                .embed(std::slice::from_ref(&text))
                .await
                .expect("embedder succeeds");
            let emb = emb
                .into_iter()
                .next()
                .expect("embedder returns one embedding per input");
            vectors
                .upsert(&note.id.to_string(), &emb)
                .await
                .expect("vector upsert succeeds");
            continue;
        }

        let embs = embedder.embed(&chunks).await.expect("embedder succeeds");
        assert_eq!(
            embs.len(),
            chunks.len(),
            "embedder returned one embedding per chunk"
        );
        for (i, (_chunk, emb)) in chunks.iter().zip(embs.iter()).enumerate() {
            // Include the chunk text in the embedding so the keyword index still sees it.
            let chunk_id = format!("{}#{i}", note.id);
            vectors
                .upsert(&chunk_id, emb)
                .await
                .expect("vector upsert succeeds");
        }
    }

    (db, repo, keyword, vectors, embedder)
}

/// Build a fresh in-memory SQLite index over the seed corpus using the
/// deterministic fake embedder. Convenience for tests.
pub async fn index_seed_corpus() -> (
    Database,
    SqliteNoteRepository,
    SqliteKeywordIndex,
    SqliteVectorIndex,
    Arc<dyn EmbeddingProvider>,
) {
    index_seed_corpus_with_embedder(Arc::new(raki_ai::FakeEmbeddingProvider::new(384))).await
}
