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

impl Note {
    /// Create a new note: fresh v7 id, version 1, both timestamps set to `now_ms`,
    /// not deleted. The "what a new note is" rule lives here, not in command adapters.
    pub fn new(title: String, body: String, now_ms: i64) -> Self {
        Self {
            id: NoteId::new(),
            title,
            body,
            created_at: now_ms,
            updated_at: now_ms,
            deleted_at: None,
            version: 1,
        }
    }
}
