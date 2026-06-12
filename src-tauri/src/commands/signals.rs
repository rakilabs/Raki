use tauri::State;

use crate::dto::RecordNoteViewInput;
use crate::error::AppError;
use crate::state::AppState;

#[tauri::command]
pub async fn record_note_view(
    state: State<'_, AppState>,
    input: RecordNoteViewInput,
) -> Result<(), AppError> {
    let id = raki_domain::NoteId::parse(&input.note_id).map_err(|_| AppError {
        kind: "invalid".into(),
        message: "invalid note_id".into(),
    })?;
    let now = state.clock.now_ms();
    state.signal_store.record_view(&id, now).await?;
    Ok(())
}
