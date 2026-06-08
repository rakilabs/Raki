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

    /// Apply an edit: new `title`/`body`, `updated_at = now_ms`, `version` bumped. Preserves
    /// `id`, `created_at`, and `deleted_at`. The "what an edit is" rule lives here, not in a
    /// command adapter.
    pub fn edit(&self, title: String, body: String, now_ms: i64) -> Note {
        Note {
            id: self.id,
            title,
            body,
            created_at: self.created_at,
            updated_at: now_ms,
            deleted_at: self.deleted_at,
            version: self.version + 1,
        }
    }
}

#[cfg(test)]
mod edit_tests {
    use super::*;

    #[test]
    fn edit_preserves_identity_and_bumps_version() {
        let original = Note::new("Trip".into(), "old".into(), 1000);
        let edited = original.edit("Trip v2".into(), "new".into(), 2000);
        assert_eq!(edited.id, original.id, "id preserved");
        assert_eq!(edited.created_at, 1000, "created_at preserved");
        assert_eq!(edited.title, "Trip v2");
        assert_eq!(edited.body, "new");
        assert_eq!(edited.updated_at, 2000);
        assert_eq!(edited.version, 2, "version bumped");
        assert_eq!(edited.deleted_at, None, "liveness preserved");
    }
}
