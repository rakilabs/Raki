//! The single error type returned across domain ports.

#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("entity not found")]
    NotFound,
    #[error("invalid input: {0}")]
    Invalid(String),
    #[error("storage error: {0}")]
    Storage(String),
    #[error("provider error: {0}")]
    Provider(String),
}
