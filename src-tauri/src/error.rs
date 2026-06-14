//! The single error type that crosses the IPC boundary. Domain errors map here.

use serde::Serialize;
use ts_rs::TS;

use raki_domain::DomainError;
use raki_memory::GenerateError;

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

impl From<GenerateError> for AppError {
    fn from(e: GenerateError) -> Self {
        use raki_domain::EgressError;
        match e {
            GenerateError::Domain(d) => AppError::from(d),
            GenerateError::Egress(EgressError::Completion(d)) => AppError::from(d),
            GenerateError::Egress(EgressError::Audit(m)) => AppError {
                kind: "audit".into(),
                message: m,
            },
            GenerateError::Egress(EgressError::Denied(d)) => AppError {
                kind: "denied".into(),
                message: d.to_string(),
            },
        }
    }
}
