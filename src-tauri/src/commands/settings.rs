//! Thin command adapters for egress settings and audit log. No business logic.

use tauri::State;

use raki_domain::Mode;

use crate::dto::{EgressLogEntryDto, EgressSettingsDto};
use crate::error::AppError;
use crate::state::AppState;

#[tauri::command]
pub async fn get_egress_settings(
    state: State<'_, AppState>,
) -> Result<EgressSettingsDto, AppError> {
    let mode = match state.settings.mode().await? {
        Mode::LocalOnly => "local_only",
        Mode::CloudAllowed => "cloud_allowed",
    };
    let consented: Vec<String> = state.settings.consented().await?.into_iter().collect();
    Ok(EgressSettingsDto {
        mode: mode.into(),
        consented_providers: consented,
    })
}

#[tauri::command]
pub async fn set_egress_mode(state: State<'_, AppState>, mode: String) -> Result<(), AppError> {
    let m = match mode.as_str() {
        "local_only" => Mode::LocalOnly,
        "cloud_allowed" => Mode::CloudAllowed,
        _ => {
            return Err(AppError {
                kind: "validation_error".into(),
                message: "mode must be local_only or cloud_allowed".into(),
            })
        }
    };
    state.settings.set_mode(m).await?;
    Ok(())
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
