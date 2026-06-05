//! Thin command adapters: translate + delegate. No business logic, no SQL, no ranking.

use tauri::State;

use raki_domain::{Note, NoteId};

use crate::dto::{CreateNoteInput, NoteDto};
use crate::error::AppError;
use crate::state::AppState;

#[tauri::command]
pub async fn create_note(
    state: State<'_, AppState>,
    input: CreateNoteInput,
) -> Result<NoteDto, AppError> {
    let note = Note::new(input.title, input.body, state.clock.now_ms());
    state.notes.upsert(&note).await?;
    state.index.trigger(); // embed the new note in the background (single-flight)
    Ok(NoteDto::from(note))
}

#[tauri::command]
pub async fn list_notes(state: State<'_, AppState>) -> Result<Vec<NoteDto>, AppError> {
    let notes = state.notes.list().await?;
    Ok(notes.into_iter().map(NoteDto::from).collect())
}

#[tauri::command]
pub async fn get_note(state: State<'_, AppState>, id: String) -> Result<Option<NoteDto>, AppError> {
    let note_id = NoteId::parse(&id)?;
    Ok(state.notes.get(&note_id).await?.map(NoteDto::from))
}

/// Hybrid search: fuse FTS5 keyword + sqlite-vec vector rankings (RRF), then hydrate
/// the ranked ids to DTOs. (Hydration is one `get` per hit; fine at k = 20.)
#[tauri::command]
pub async fn search_notes(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<NoteDto>, AppError> {
    let ids = raki_retrieval::hybrid_search(
        state.keyword.as_ref(),
        state.vectors.as_ref(),
        state.embedder.as_ref(),
        &query,
        20,
    )
    .await?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let note_id = NoteId::parse(&id)?;
        if let Some(note) = state.notes.get(&note_id).await? {
            out.push(NoteDto::from(note));
        }
    }
    Ok(out)
}
