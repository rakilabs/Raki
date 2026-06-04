//! Stable, sortable identifiers.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DomainError;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NoteId(Uuid);

impl NoteId {
    /// Create a fresh, time-ordered identifier (UUID v7).
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn parse(s: &str) -> Result<Self, DomainError> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|e| DomainError::Invalid(format!("invalid NoteId: {e}")))
    }
}

impl Default for NoteId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for NoteId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_id_roundtrips_through_string() {
        let id = NoteId::new();
        let parsed = NoteId::parse(&id.to_string()).expect("should parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(NoteId::parse("not-a-uuid").is_err());
    }
}
