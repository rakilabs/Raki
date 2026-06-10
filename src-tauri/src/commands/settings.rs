//! Thin command adapters for egress settings and audit log. No business logic.

use tauri::State;

use crate::dto::{EgressLogEntryDto, EgressSettingsDto};
use crate::error::AppError;
use crate::state::AppState;

#[tauri::command]
pub async fn get_egress_settings(
    state: State<'_, AppState>,
) -> Result<EgressSettingsDto, AppError> {
    let consented: Vec<String> = state.settings.consented().await?.into_iter().collect();
    Ok(EgressSettingsDto {
        consented_providers: consented,
    })
}

#[tauri::command]
pub async fn grant_provider_consent(
    state: State<'_, AppState>,
    provider: String,
) -> Result<(), AppError> {
    state.settings.grant(&provider).await?;
    Ok(())
}

#[tauri::command]
pub async fn revoke_provider_consent(
    state: State<'_, AppState>,
    provider: String,
) -> Result<(), AppError> {
    state.settings.revoke(&provider).await?;
    Ok(())
}

#[tauri::command]
pub async fn list_egress_log(
    state: State<'_, AppState>,
    limit: usize,
) -> Result<Vec<EgressLogEntryDto>, AppError> {
    let recs = state.egress_log.list_recent(limit).await?;
    Ok(recs
        .into_iter()
        .map(|r| EgressLogEntryDto {
            id: r.id.to_string(),
            provider: r.decision.provider,
            model: r.decision.model,
            token_count: r.decision.total_tokens as i64,
            source_count: r.decision.source_ids.len(),
            success: r.success,
            created_at: r.completed_at,
        })
        .collect())
}
