//! raki-app: the composition root. Wires concrete adapters into ports and exposes
//! the Tauri command surface. The only crate that touches `tauri`.

mod commands;
mod dto;
mod error;
mod state;

use std::sync::Arc;

use tauri::Manager;

use raki_ai::{EgressPolicy, FakeEmbeddingProvider};
use raki_domain::Clock;
use raki_storage::{Database, SqliteKeywordIndex, SqliteNoteRepository};

use crate::commands::notes::{create_note, get_note, list_notes, search_notes};
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
            let keyword = Arc::new(SqliteKeywordIndex::new(db));

            app.manage(AppState {
                notes,
                keyword,
                clock: Arc::new(SystemClock),
                embedder: Arc::new(FakeEmbeddingProvider::new(384)),
                egress: EgressPolicy::LocalOnly,
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
