//! The single error type that crosses the IPC boundary. Domain errors map here.

use serde::Serialize;
use ts_rs::TS;

use raki_domain::DomainError;

#[derive(Debug, Serialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct AppError {
    pub kind: String,
    pub message: String,
}

impl From<DomainError> for AppError {
    fn from(e: DomainError) -> Self {
        let kind = match &e {
            DomainError::NotFound => "not_found",
            DomainError::Invalid(_) => "invalid",
            DomainError::Storage(_) => "storage",
            DomainError::Provider(_) => "provider",
        };
        AppError {
            kind: kind.to_string(),
            message: e.to_string(),
        }
    }
}
