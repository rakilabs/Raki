//! Grounded-QA command adapters: translate + delegate to `raki-generate`. No business logic.

use tauri::State;

use raki_domain::EgressError;
use raki_generate::{assemble_for, send_answer, GenerateError};

use crate::dto::{AnswerOutcome, CitedNote, EgressPreviewDto};
use crate::error::AppError;
use crate::state::AppState;

fn deps(state: &AppState) -> raki_generate::GenerateDeps<'_> {
    raki_generate::GenerateDeps {
        keyword: state.keyword.as_ref(),
        vectors: state.vectors.as_ref(),
        embedder: state.embedder.as_ref(),
        notes: state.notes.as_ref(),
        gate: state.gate.as_ref(),
        provider: &state.provider,
        model: &state.model,
        budget: state.budget_tokens,
        k: state.k,
    }
}

#[tauri::command]
pub async fn answer_question(
    state: State<'_, AppState>,
    query: String,
) -> Result<AnswerOutcome, AppError> {
    let d = deps(state.inner());

    let Some((ctx, titles)) = assemble_for(&query, &d).await? else {
        return Ok(AnswerOutcome::Answer {
            state: "nothing_matched".into(),
            text: "No relevant notes found.".into(),
            cited: vec![],
        });
    };

    // Build the preview now — we'll need it if the gate denies.
    let preview = EgressPreviewDto {
        provider: d.provider.to_string(),
        summary: ctx.egress.summary(),
        source_titles: ctx
            .egress
            .source_ids
            .iter()
            .map(|s| titles.get(&s.0).cloned().unwrap_or_else(|| s.0.clone()))
            .collect(),
    };

    match send_answer(&ctx, &query, &d).await {
        Ok(ans) => {
            let mut cited = Vec::with_capacity(ans.cited_ids.len());
            for sid in &ans.cited_ids {
                let title = titles.get(&sid.0).cloned().unwrap_or_else(|| sid.0.clone());
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
        Err(GenerateError::Egress(EgressError::Denied(
            raki_domain::EgressDenied::LocalOnlyMode | raki_domain::EgressDenied::ConsentRequired,
        ))) => Ok(AnswerOutcome::NeedsConsent { preview }),
        Err(e) => Err(AppError::from(e)),
    }
}
