//! raki-app: the composition root. Wires concrete adapters into ports and exposes
//! the Tauri command surface. The only crate that touches `tauri`.

mod commands;
mod dto;
mod error;
mod indexing;
mod state;

use std::sync::Arc;

use tauri::Manager;

use raki_ai::{
    FakeEmbeddingProvider, FastEmbedProvider, FastEmbedReranker, GatedLlmProvider, MessagesProvider,
};
use raki_domain::{
    Clock, Completion, CompletionRequest, DomainError, EmbeddingProvider, IndexingStore,
    LlmProvider, Locality, Reranker, VectorIndex,
};
use raki_storage::{
    Database, SqliteEgressLog, SqliteEgressSettings, SqliteIndexingStore, SqliteKeywordIndex,
    SqliteNoteRepository, SqliteVectorIndex,
};

use crate::commands::notes::{create_note, get_note, list_notes, search_notes, update_note};
use crate::commands::qa::{answer_question, grant_cloud_consent, revoke_cloud_consent};
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

/// Used when no cloud model is configured. Never sends; fails clearly if a gated call reaches it
/// (only possible after the user grants consent, so the message is actionable).
struct UnconfiguredProvider;
#[async_trait::async_trait]
impl LlmProvider for UnconfiguredProvider {
    fn locality(&self) -> Locality {
        Locality::Cloud
    }
    async fn complete(&self, _req: CompletionRequest) -> Result<Completion, DomainError> {
        Err(DomainError::Provider(
            "no cloud model configured (set RAKI_LLM_BASE_URL / ANTHROPIC_API_KEY / RAKI_LLM_MODEL)".into(),
        ))
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Load .env if present so cloud provider config doesn't require shell exports.
    let _ = dotenvy::dotenv();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db = Database::open(&dir.join("raki.sqlite"))?;

            let notes = Arc::new(SqliteNoteRepository::new(db.clone()));
            let keyword = Arc::new(SqliteKeywordIndex::new(db.clone()));
            let vectors: Arc<dyn VectorIndex> = Arc::new(SqliteVectorIndex::new(db.clone()));
            let store: Arc<dyn IndexingStore> = Arc::new(SqliteIndexingStore::new(db.clone()));

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

            let reranker: Option<Arc<dyn Reranker>> = match FastEmbedReranker::try_new() {
                Ok(r) => Some(Arc::new(r)),
                Err(e) => {
                    eprintln!(
                        "reranker unavailable ({e}); search runs without reranking this session"
                    );
                    None
                }
            };

            let index = Arc::new(IndexingService::new(
                store,
                embedder.clone(),
                vectors.clone(),
            ));
            index.trigger(); // startup catch-up pass (backfill + drain), single-flight

            let settings: Arc<dyn raki_domain::EgressSettings> =
                Arc::new(SqliteEgressSettings::new(db.clone()));
            let egress_log: Arc<dyn raki_domain::EgressLog> =
                Arc::new(SqliteEgressLog::new(db.clone()));

            let provider = "kimi".to_string();
            let model = std::env::var("RAKI_LLM_MODEL").unwrap_or_else(|_| "unknown".to_string());
            let inner: Arc<dyn LlmProvider> = match MessagesProvider::from_env() {
                Ok(p) => Arc::new(p),
                Err(e) => {
                    eprintln!("cloud model unavailable ({e}); QA will error until configured");
                    Arc::new(UnconfiguredProvider)
                }
            };
            let clock: Arc<dyn Clock> = Arc::new(SystemClock);
            let gate = Arc::new(GatedLlmProvider::new(
                inner,
                settings.clone(),
                egress_log,
                clock.clone(),
            ));

            app.manage(AppState {
                notes,
                keyword,
                vectors,
                embedder,
                reranker,
                clock,
                index,
                gate,
                settings,
                provider,
                model,
                k: 10,
                budget_tokens: 2000,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_note,
            list_notes,
            get_note,
            search_notes,
            update_note,
            answer_question,
            grant_cloud_consent,
            revoke_cloud_consent
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
