//! The egress contract: what would leave the device, the policy ports, and the gate's error type.
//! Lives in the kernel because `raki-ai` (the gate) and `raki-memory` (the context) both need it
//! and cannot see each other.

use std::collections::HashSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::DomainError;

/// Opaque id of a retrieval source in a context — a note id today, a block id once chunking ships.
/// A newtype so it can't be confused with a provider or model string.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceId(pub String);

impl std::fmt::Display for SourceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// UUID v7 id of an egress-log row (mirrors `NoteId`; never a leaked SQLite rowid).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EgressLogId(Uuid);

impl EgressLogId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
    pub fn parse(s: &str) -> Result<Self, DomainError> {
        Uuid::parse_str(s)
            .map(Self)
            .map_err(|e| DomainError::Invalid(format!("invalid EgressLogId: {e}")))
    }
}

impl Default for EgressLogId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for EgressLogId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// What WOULD leave the device on a cloud completion — metadata only, never note text or keys.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EgressDecision {
    pub provider: String,
    pub model: String,
    pub source_ids: Vec<SourceId>,
    pub total_tokens: usize,
}

impl EgressDecision {
    /// Human-readable one-liner for display/logging. Derived, never stored (no format migrations).
    pub fn summary(&self) -> String {
        format!(
            "{} sources, {} tokens → {}/{}",
            self.source_ids.len(),
            self.total_tokens,
            self.provider,
            self.model
        )
    }
    pub fn is_empty(&self) -> bool {
        self.source_ids.is_empty()
    }
}

/// The persisted egress event: the decision PLUS what actually happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EgressRecord {
    pub id: EgressLogId,
    pub decision: EgressDecision,
    pub completed_at: i64,
    pub success: bool,
}

/// Master egress switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    LocalOnly,
    CloudAllowed,
}

/// Why an egress was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EgressDenied {
    #[error("local-only mode: cloud calls are disabled")]
    LocalOnlyMode,
    #[error("consent required for this provider")]
    ConsentRequired,
    #[error("empty context: nothing to send")]
    EmptyContext,
}

/// The outcome of a gated completion: denied before sending, or the inner provider's result.
#[derive(Debug, thiserror::Error)]
pub enum EgressError {
    #[error("egress denied: {0}")]
    Denied(#[from] EgressDenied),
    #[error("completion failed: {0}")]
    Completion(#[from] DomainError),
    #[error("egress audit log failed: {0}")]
    Audit(String),
}

/// Persist a record of what left (or attempted to leave) the device.
#[async_trait]
pub trait EgressLog: Send + Sync {
    async fn record(&self, rec: &EgressRecord) -> Result<(), DomainError>;
    /// Attach the groundedness verdict to an already-logged egress row.
    async fn set_grounded(&self, id: &EgressLogId, grounded: bool) -> Result<(), DomainError>;
}

/// Live-read egress settings: the master mode + per-provider consent. Read every call (no caching).
#[async_trait]
pub trait EgressSettings: Send + Sync {
    async fn mode(&self) -> Result<Mode, DomainError>;
    async fn consented(&self) -> Result<HashSet<String>, DomainError>;
    async fn set_mode(&self, mode: Mode) -> Result<(), DomainError>;
    async fn grant(&self, provider: &str) -> Result<(), DomainError>;
    async fn revoke(&self, provider: &str) -> Result<(), DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(ids: &[&str], tokens: usize) -> EgressDecision {
        EgressDecision {
            provider: "kimi".into(),
            model: "k2".into(),
            source_ids: ids.iter().map(|s| SourceId(s.to_string())).collect(),
            total_tokens: tokens,
        }
    }

    #[test]
    fn summary_is_metadata_only_and_derived() {
        let d = decision(&["a", "b"], 1180);
        assert_eq!(d.summary(), "2 sources, 1180 tokens → kimi/k2");
        assert!(!d.is_empty());
        assert!(decision(&[], 0).is_empty());
    }

    #[test]
    fn egress_log_id_roundtrips() {
        let id = EgressLogId::new();
        assert_eq!(EgressLogId::parse(&id.to_string()).unwrap(), id);
        assert!(EgressLogId::parse("nope").is_err());
    }
}
