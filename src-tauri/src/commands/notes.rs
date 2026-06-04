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

/// Naive substring search over titles/bodies. Demonstrates the retrieval wiring;
/// real hybrid FTS5 + sqlite-vec search replaces the body of this command later.
#[tauri::command]
pub async fn search_notes(
    state: State<'_, AppState>,
    query: String,
) -> Result<Vec<NoteDto>, AppError> {
    let needle = query.to_lowercase();
    let notes = state.notes.list().await?;
    Ok(notes
        .into_iter()
        .filter(|n| {
            n.title.to_lowercase().contains(&needle) || n.body.to_lowercase().contains(&needle)
        })
        .map(NoteDto::from)
        .collect())
}
