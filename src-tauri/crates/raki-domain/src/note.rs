//! The Note entity. `body` carries ProseMirror JSON (opaque text for now).

use serde::{Deserialize, Serialize};

use crate::ids::NoteId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub id: NoteId,
    pub title: String,
    pub body: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: Option<i64>,
    pub version: i64,
}
