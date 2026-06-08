//! Data-transfer objects: the typed contract the frontend sees. Generated to TS via ts-rs.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use raki_domain::Note;

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct NoteDto {
    pub id: String,
    pub title: String,
    pub body: String,
    // i64 epoch-ms: serde_json sends these as JSON numbers over IPC, so the TS
    // contract must be `number`, not ts-rs's default `bigint`.
    #[ts(type = "number")]
    pub created_at: i64,
    #[ts(type = "number")]
    pub updated_at: i64,
}

impl From<Note> for NoteDto {
    fn from(n: Note) -> Self {
        NoteDto {
            id: n.id.to_string(),
            title: n.title,
            body: n.body,
            created_at: n.created_at,
            updated_at: n.updated_at,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct CreateNoteInput {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct CitedNote {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct EgressPreviewDto {
    pub provider: String,
    pub summary: String,
    pub source_titles: Vec<String>,
}

/// Either we need consent (and show what would leave), or we have an answer.
/// Tagged union so the frontend can pattern-match on `kind`.
#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(
    export,
    tag = "kind",
    rename_all = "snake_case",
    export_to = "../../src/shared/ipc/bindings/"
)]
pub enum AnswerOutcome {
    NeedsConsent {
        preview: EgressPreviewDto,
    },
    Answer {
        state: String,
        text: String,
        cited: Vec<CitedNote>,
    },
}
