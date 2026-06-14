//! Grounded-QA command adapter: translate + delegate to `AnswerService`. No business logic.

use tauri::State;

use raki_domain::AnswerState;
use raki_memory::AnswerResult;

use crate::dto::{AnswerOutcome, CitedNote, EgressPreviewDto};
use crate::error::AppError;
use crate::state::AppState;

#[tauri::command]
pub async fn answer_question(
    state: State<'_, AppState>,
    query: String,
) -> Result<AnswerOutcome, AppError> {
    let rewriter = state.rewriter.as_ref().map(|r| r.as_ref());

    match state.answer_service.answer(&query, rewriter).await? {
        AnswerResult::Answer(ans) if ans.state == AnswerState::NothingMatched => {
            Ok(AnswerOutcome::Answer {
                state: AnswerState::NothingMatched.name().to_string(),
                text: "No relevant notes found.".into(),
                cited: vec![],
            })
        }
        AnswerResult::Answer(ans) => Ok(AnswerOutcome::Answer {
            state: ans.state.name().to_string(),
            text: ans.text,
            cited: ans
                .cited_ids
                .into_iter()
                .map(|sid| {
                    let id = sid.0.clone();
                    CitedNote {
                        title: ans
                            .source_titles
                            .get(&id)
                            .cloned()
                            .unwrap_or_else(|| id.clone()),
                        id,
                    }
                })
                .collect(),
        }),
        AnswerResult::NeedsConsent(preview) => Ok(AnswerOutcome::NeedsConsent {
            preview: EgressPreviewDto {
                provider: preview.provider,
                summary: preview.summary,
                source_titles: preview.source_titles,
            },
        }),
    }
}
