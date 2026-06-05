//! raki-app: the composition root. Wires concrete adapters into ports and exposes
//! the Tauri command surface. The only crate that touches `tauri`.

mod commands;
mod dto;
mod error;
mod indexing;
mod state;

use std::sync::Arc;

use tauri::Manager;

use raki_ai::{EgressPolicy, FakeEmbeddingProvider, FastEmbedProvider};
use raki_domain::{Clock, EmbeddingProvider, IndexingStore, VectorIndex};
use raki_storage::{
    Database, SqliteIndexingStore, SqliteKeywordIndex, SqliteNoteRepository, SqliteVectorIndex,
};

use crate::commands::notes::{create_note, get_note, list_notes, search_notes};
use crate::indexing::IndexingService;
use crate::state::AppState;

/// A system clock. Lives in the composition root so the domain stays IO-free.
struct SystemClock;
impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db = Database::open(&dir.join("raki.sqlite"))?;

            let notes = Arc::new(SqliteNoteRepository::new(db.clone()));
            let keyword = Arc::new(SqliteKeywordIndex::new(db.clone()));
            let vectors: Arc<dyn VectorIndex> = Arc::new(SqliteVectorIndex::new(db.clone()));
            let store: Arc<dyn IndexingStore> = Arc::new(SqliteIndexingStore::new(db));

            // Real embeddings if the model is available; otherwise degrade to the fake
            // so the app still runs (keyword search is unaffected). The model-id
            // staleness check re-embeds with the real model once it's available.
            let embedder: Arc<dyn EmbeddingProvider> = match FastEmbedProvider::try_new() {
                Ok(p) => Arc::new(p),
                Err(e) => {
                    eprintln!("fastembed unavailable ({e}); using fake embeddings this session");
                    Arc::new(FakeEmbeddingProvider::new(384))
                }
            };

            let index = Arc::new(IndexingService::new(store, embedder, vectors));
            index.trigger(); // startup catch-up pass (backfill + drain), single-flight

            app.manage(AppState {
                notes,
                keyword,
                clock: Arc::new(SystemClock),
                egress: EgressPolicy::LocalOnly,
                index,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_note,
            list_notes,
            get_note,
            search_notes
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
