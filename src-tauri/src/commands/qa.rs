//! Grounded-QA command adapters: translate + delegate to `raki-generate`. No business logic.

use tauri::State;

use raki_domain::{EgressDenied, EgressError, Mode, NoteId};
use raki_generate::{answer_question as run_answer, preview, GenerateDeps, GenerateError};

use crate::dto::{AnswerOutcome, CitedNote, EgressPreviewDto};
use crate::error::AppError;
use crate::state::AppState;

// ---- block A: shared helpers ----
const K: usize = 10;
const BUDGET_TOKENS: usize = 2000;

fn deps(state: &AppState) -> GenerateDeps<'_> {
    GenerateDeps {
        keyword: state.keyword.as_ref(),
        vectors: state.vectors.as_ref(),
        embedder: state.embedder.as_ref(),
        notes: state.notes.as_ref(),
        gate: state.gate.as_ref(),
        provider: &state.provider,
        model: &state.model,
        budget: BUDGET_TOKENS,
        k: K,
    }
}

/// A `Denied(LocalOnlyMode | ConsentRequired)` is NOT an error — it means "ask the user first".
/// Sync: a match guard cannot `.await`.
fn needs_consent(e: &GenerateError) -> bool {
    matches!(
        e,
        GenerateError::Egress(EgressError::Denied(
            EgressDenied::LocalOnlyMode | EgressDenied::ConsentRequired
        ))
    )
}

fn into_app_error(e: GenerateError) -> AppError {
    match e {
        GenerateError::Domain(d) => AppError::from(d),
        GenerateError::Egress(EgressError::Completion(d)) => AppError::from(d),
        GenerateError::Egress(EgressError::Audit(m)) => AppError {
            kind: "audit".into(),
            message: m,
        },
        GenerateError::Egress(EgressError::Denied(d)) => AppError {
            kind: "denied".into(),
            message: d.to_string(),
        },
    }
}

// ---- block B: the answer command ----
#[tauri::command]
pub async fn answer_question(
    state: State<'_, AppState>,
    query: String,
) -> Result<AnswerOutcome, AppError> {
    match run_answer(&query, &deps(state.inner())).await {
        Ok(ans) => {
            let mut cited = Vec::with_capacity(ans.cited_ids.len());
            for sid in &ans.cited_ids {
                let title = match NoteId::parse(&sid.0) {
                    Ok(nid) => state
                        .notes
                        .get(&nid)
                        .await?
                        .map(|n| n.title)
                        .unwrap_or_else(|| sid.0.clone()),
                    Err(_) => sid.0.clone(),
                };
                cited.push(CitedNote {
                    id: sid.0.clone(),
                    title,
                });
            }
            Ok(AnswerOutcome::Answer {
                state: ans.state.name().to_string(),
                text: ans.text,
                cited,
            })
        }
        Err(e) if needs_consent(&e) => {
            // Re-run retrieve+assemble locally (no send) to show what WOULD leave.
            match preview(&query, &deps(state.inner())).await {
                Ok(Some(p)) => Ok(AnswerOutcome::NeedsConsent {
                    preview: EgressPreviewDto {
                        provider: p.provider,
                        summary: p.summary,
                        source_titles: p.source_titles,
                    },
                }),
                Ok(None) => Ok(AnswerOutcome::Answer {
                    state: "nothing_matched".into(),
                    text: "No relevant notes found.".into(),
                    cited: vec![],
                }),
                Err(pe) => Err(into_app_error(pe)),
            }
        }
        Err(e) => Err(into_app_error(e)),
    }
}

// ---- block C: consent mutation commands ----
#[tauri::command]
pub async fn grant_cloud_consent(
    state: State<'_, AppState>,
    provider: String,
) -> Result<(), AppError> {
    state.settings.set_mode(Mode::CloudAllowed).await?;
    state.settings.grant(&provider).await?;
    Ok(())
}

/// Revoking the provider is sufficient to block egress: `GatedLlmProvider` requires BOTH
/// `CloudAllowed` mode AND a provider-specific grant, so an empty consent set denies all sends
/// even though mode stays `CloudAllowed` (review #6).
#[tauri::command]
pub async fn revoke_cloud_consent(
    state: State<'_, AppState>,
    provider: String,
) -> Result<(), AppError> {
    state.settings.revoke(&provider).await?;
    Ok(())
}
