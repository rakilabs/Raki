//! Domain types for the grounded answer flow.

use crate::egress::{EgressLogId, SourceId};

/// The answer's relationship to the retrieved context.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerState {
    NothingMatched,
    NotAnswerable,
    ParseFailed,
    Ungrounded,
    Grounded,
}

impl AnswerState {
    pub fn name(&self) -> &'static str {
        match self {
            AnswerState::NothingMatched => "nothing_matched",
            AnswerState::NotAnswerable => "not_answerable",
            AnswerState::ParseFailed => "parse_failed",
            AnswerState::Ungrounded => "ungrounded",
            AnswerState::Grounded => "grounded",
        }
    }
    pub fn is_grounded(&self) -> bool {
        matches!(self, AnswerState::Grounded)
    }
}

/// The result of a gated answer request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub state: AnswerState,
    pub text: String,
    pub cited_ids: Vec<SourceId>,
    pub source_titles: std::collections::HashMap<String, String>,
    pub egress_log_id: Option<EgressLogId>,
}

/// What a cloud send WOULD disclose — metadata only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EgressPreview {
    pub provider: String,
    pub summary: String,
    pub source_titles: Vec<String>,
}
