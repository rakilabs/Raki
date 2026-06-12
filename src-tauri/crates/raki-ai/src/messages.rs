//! `MessagesProvider`: a cloud `LlmProvider` speaking the Anthropic Messages wire protocol.
//! Primary target is Kimi (model = `kimi-k2-5`). The team's `ckimi` shell shim exports
//! `ANTHROPIC_BASE_URL=https://api.kimi.com/coding/` + `ANTHROPIC_API_KEY`; the adapter also
//! accepts `KIMI_API_KEY` / `KIMI_API` as key fallbacks. reqwest is allowed here per AGENTS.md.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use raki_domain::{Completion, CompletionRequest, DomainError, LlmProvider, Locality};

const DEFAULT_MAX_TOKENS: u32 = 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const ANTHROPIC_VERSION: &str = "2023-06-01";

struct Config {
    base_url: String,
    api_key: String,
    model: String,
}

fn config_from_env() -> Result<Config, DomainError> {
    let base_url = std::env::var("RAKI_LLM_BASE_URL")
        .or_else(|_| std::env::var("ANTHROPIC_BASE_URL"))
        .map_err(|_| {
            DomainError::Provider("RAKI_LLM_BASE_URL / ANTHROPIC_BASE_URL not set".into())
        })?;
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .or_else(|_| std::env::var("KIMI_API_KEY"))
        .or_else(|_| std::env::var("KIMI_API"))
        .map_err(|_| {
            DomainError::Provider("ANTHROPIC_API_KEY / KIMI_API_KEY / KIMI_API not set".into())
        })?;
    let model = std::env::var("RAKI_LLM_MODEL").unwrap_or_else(|_| "kimi-k2-5".into());
    Ok(Config {
        base_url,
        api_key,
        model,
    })
}

/// Build the Messages request body. Pure — unit-testable without a network.
fn build_request_body(req: &CompletionRequest, model: &str) -> Value {
    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS),
        "messages": [{ "role": "user", "content": req.prompt }],
    });
    if let Some(system) = &req.system {
        body["system"] = Value::String(system.clone());
    }
    body
}

/// Extract the assistant text from a Messages response body. Pure.
fn parse_response(bytes: &[u8]) -> Result<String, DomainError> {
    let v: Value = serde_json::from_slice(bytes)
        .map_err(|e| DomainError::Provider(format!("invalid response JSON: {e}")))?;
    v.get("content")
        .and_then(|c| c.as_array())
        .and_then(|a| {
            a.iter()
                .find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
        })
        .and_then(|b| b.get("text"))
        .and_then(|t| t.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| DomainError::Provider("no text block in response".into()))
}

pub struct MessagesProvider {
    client: reqwest::Client,
    config: Config,
}

impl MessagesProvider {
    /// Build from env (`RAKI_LLM_BASE_URL`|`ANTHROPIC_BASE_URL`, `ANTHROPIC_API_KEY`, `RAKI_LLM_MODEL`).
    pub fn from_env() -> Result<Self, DomainError> {
        Self::from_env_with_model(None)
    }

    /// Build from env, overriding the model. Useful when query rewriting wants a cheaper/faster
    /// model than the main QA model.
    pub fn from_env_with_model(model_override: Option<String>) -> Result<Self, DomainError> {
        let mut config = config_from_env()?;
        if let Some(model) = model_override {
            config.model = model;
        }
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| DomainError::Provider(format!("http client: {e}")))?;
        Ok(Self { client, config })
    }
}

#[async_trait]
impl LlmProvider for MessagesProvider {
    fn locality(&self) -> Locality {
        Locality::Cloud
    }

    async fn complete(&self, req: CompletionRequest) -> Result<Completion, DomainError> {
        let url = format!("{}/v1/messages", self.config.base_url.trim_end_matches('/'));
        let body = build_request_body(&req, &self.config.model);

        // One retry on a transport error (timeout/connect); never on an HTTP status.
        for attempt in 0..2 {
            let resp = self
                .client
                .post(&url)
                .header("x-api-key", &self.config.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .json(&body)
                .send()
                .await;
            match resp {
                Ok(r) => {
                    let status = r.status();
                    let bytes = r
                        .bytes()
                        .await
                        .map_err(|e| DomainError::Provider(e.to_string()))?;
                    if !status.is_success() {
                        return Err(DomainError::Provider(format!(
                            "messages API {status}: {}",
                            String::from_utf8_lossy(&bytes)
                        )));
                    }
                    return Ok(Completion {
                        text: parse_response(&bytes)?,
                    });
                }
                Err(e) if attempt == 0 && (e.is_timeout() || e.is_connect()) => {
                    continue;
                }
                Err(e) => return Err(DomainError::Provider(e.to_string())),
            }
        }
        // Unreachable in practice — every loop path returns. Kept to satisfy the compiler.
        Err(DomainError::Provider("transport error after retry".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> CompletionRequest {
        CompletionRequest {
            system: Some("rules".into()),
            prompt: "why is the sky blue?".into(),
            max_tokens: Some(256),
        }
    }

    #[test]
    fn body_has_model_system_messages_and_max_tokens() {
        let b = build_request_body(&req(), "kimi-k2");
        assert_eq!(b["model"], "kimi-k2");
        assert_eq!(b["max_tokens"], 256);
        assert_eq!(b["system"], "rules");
        assert_eq!(b["messages"][0]["role"], "user");
        assert_eq!(b["messages"][0]["content"], "why is the sky blue?");
    }

    #[test]
    fn body_omits_system_when_none_and_defaults_max_tokens() {
        let r = CompletionRequest {
            system: None,
            prompt: "hi".into(),
            max_tokens: None,
        };
        let b = build_request_body(&r, "m");
        assert!(b.get("system").is_none());
        assert_eq!(b["max_tokens"], DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn parse_extracts_first_text_block() {
        let bytes = br#"{"content":[{"type":"text","text":"because Rayleigh scattering"}]}"#;
        assert_eq!(
            parse_response(bytes).unwrap(),
            "because Rayleigh scattering"
        );
    }

    #[test]
    fn parse_errors_when_no_text_block() {
        let bytes = br#"{"content":[]}"#;
        assert!(parse_response(bytes).is_err());
        assert!(parse_response(b"not json").is_err());
    }

    #[tokio::test]
    #[ignore = "hits the real cloud endpoint; needs RAKI_LLM_* env + network"]
    async fn live_completion_smoke() {
        let p = MessagesProvider::from_env().unwrap();
        let out = p
            .complete(CompletionRequest {
                system: Some("Reply with exactly: pong".into()),
                prompt: "ping".into(),
                max_tokens: Some(16),
            })
            .await
            .unwrap();
        assert!(!out.text.is_empty());
    }
}
