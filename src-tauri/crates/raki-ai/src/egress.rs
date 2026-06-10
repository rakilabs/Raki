//! The egress gate: the single, type-enforced path from an `AssembledContext` to a model call.
//! `approve` is pure policy; `GatedLlmProvider` (Task 4) is the only thing the app is handed.

use std::collections::HashSet;
use std::sync::Arc;

use raki_domain::{
    Clock, Completion, CompletionRequest, DomainError, EgressDecision, EgressDenied, EgressError,
    EgressLog, EgressLogId, EgressRecord, EgressSettings, LlmProvider, Locality,
};

/// Decide whether `decision` may leave the device under the consented set. Pure. `pub(crate)` —
/// it is an implementation detail of the gate, exposed only to this crate's tests.
pub(crate) fn approve(
    decision: &EgressDecision,
    consented: &HashSet<String>,
) -> Result<(), EgressDenied> {
    if decision.is_empty() {
        return Err(EgressDenied::EmptyContext);
    }
    if consented.contains(&decision.provider) {
        Ok(())
    } else {
        Err(EgressDenied::ConsentRequired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::SourceId;

    fn consented(providers: &[&str]) -> HashSet<String> {
        providers.iter().map(|s| s.to_string()).collect()
    }
    fn decision(provider: &str, ids: &[&str]) -> EgressDecision {
        EgressDecision {
            provider: provider.into(),
            model: "m".into(),
            source_ids: ids.iter().map(|s| SourceId(s.to_string())).collect(),
            total_tokens: 10,
        }
    }

    #[test]
    fn empty_context_is_refused_regardless_of_consent() {
        let d = decision("kimi", &[]);
        assert_eq!(
            approve(&d, &consented(&["kimi"])),
            Err(EgressDenied::EmptyContext)
        );
    }

    #[test]
    fn cloud_requires_provider_consent() {
        let d = decision("kimi", &["a"]);
        assert_eq!(
            approve(&d, &consented(&[])),
            Err(EgressDenied::ConsentRequired)
        );
        assert_eq!(approve(&d, &consented(&["kimi"])), Ok(()));
    }
}

/// The single intended path to a completion. Wraps a raw provider; reads consent live; logs what
/// actually left (after the call). The boundary is enforced by *convention*, not the type system:
/// `LlmProvider::complete` is public in `raki-domain`, so the compiler cannot forbid an un-gated
/// call. The guarantee holds because `raki-ai` constructs every cloud provider inside this wrapper
/// and re-exports only `GatedLlmProvider` — never the raw `dyn LlmProvider` — and AGENTS.md forbids
/// completion calls outside this crate. Keep it that way: don't re-export a raw cloud provider.
pub struct GatedLlmProvider {
    inner: Arc<dyn LlmProvider>,
    settings: Arc<dyn EgressSettings>,
    log: Arc<dyn EgressLog>,
    clock: Arc<dyn Clock>,
}

impl GatedLlmProvider {
    pub fn new(
        inner: Arc<dyn LlmProvider>,
        settings: Arc<dyn EgressSettings>,
        log: Arc<dyn EgressLog>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            inner,
            settings,
            log,
            clock,
        }
    }

    /// Complete via the inner provider, enforcing locality-aware policy:
    /// - `Locality::Local` → always allowed, no log row (nothing left the device).
    /// - `Locality::Cloud` → requires provider consent; on approval, call + log; on denial,
    ///   return `EgressError::Denied` with no send and no log row.
    pub async fn complete_gated(
        &self,
        egress: &EgressDecision,
        req: CompletionRequest,
    ) -> Result<(Completion, Option<EgressLogId>), EgressError> {
        if self.inner.locality() == Locality::Local {
            let completion = self
                .inner
                .complete(req)
                .await
                .map_err(EgressError::Completion)?;
            return Ok((completion, None));
        }

        // Cloud path: live consent snapshot, never cached.
        let consented = self.settings.consented().await?;
        approve(egress, &consented)?; // EgressDenied → EgressError::Denied; no send, no log row.

        let id = EgressLogId::new();
        let result = self.inner.complete(req).await;
        // Log AFTER the call — record what DID (or did not) leave.
        let rec = EgressRecord {
            id,
            decision: egress.clone(),
            completed_at: self.clock.now_ms(),
            success: result.is_ok(),
        };
        if let Err(e) = self.log.record(&rec).await {
            // Do not hand back an unlogged id: the audit trail is the contract.
            return Err(EgressError::Audit(e.to_string()));
        }
        let completion = result.map_err(EgressError::Completion)?;
        Ok((completion, Some(id)))
    }

    /// Attach the groundedness verdict to a prior gated completion's log row.
    pub async fn set_grounded(&self, id: &EgressLogId, grounded: bool) -> Result<(), DomainError> {
        self.log.set_grounded(id, grounded).await
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;
    use crate::testing::FakeLlmProvider;
    use raki_domain::{testing::FixedClock, DomainError, EgressRecord, EgressSettings, SourceId};
    use std::collections::HashSet;
    use std::sync::Mutex;

    #[derive(Default)]
    struct SpyLog {
        rows: Mutex<Vec<EgressRecord>>,
    }
    #[async_trait::async_trait]
    impl EgressLog for SpyLog {
        async fn record(&self, rec: &EgressRecord) -> Result<(), DomainError> {
            self.rows.lock().unwrap().push(rec.clone());
            Ok(())
        }
        async fn set_grounded(
            &self,
            _id: &EgressLogId,
            _grounded: bool,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn list_recent(&self, _limit: usize) -> Result<Vec<EgressRecord>, DomainError> {
            Ok(self.rows.lock().unwrap().clone())
        }
    }

    /// An `EgressLog` whose `record` write always fails — to prove the audit-or-fail contract.
    struct FailingLog;
    #[async_trait::async_trait]
    impl EgressLog for FailingLog {
        async fn record(&self, _rec: &EgressRecord) -> Result<(), DomainError> {
            Err(DomainError::Storage("disk full".into()))
        }
        async fn set_grounded(
            &self,
            _id: &EgressLogId,
            _grounded: bool,
        ) -> Result<(), DomainError> {
            Ok(())
        }
        async fn list_recent(&self, _limit: usize) -> Result<Vec<EgressRecord>, DomainError> {
            Ok(vec![])
        }
    }

    struct FakeSettings {
        consented: Vec<String>,
    }
    #[async_trait::async_trait]
    impl EgressSettings for FakeSettings {
        async fn consented(&self) -> Result<HashSet<String>, DomainError> {
            Ok(self.consented.iter().cloned().collect())
        }
        async fn grant(&self, _p: &str) -> Result<(), DomainError> {
            Ok(())
        }
        async fn revoke(&self, _p: &str) -> Result<(), DomainError> {
            Ok(())
        }
    }

    fn decision(ids: &[&str]) -> EgressDecision {
        EgressDecision {
            provider: "kimi".into(),
            model: "k2".into(),
            source_ids: ids.iter().map(|s| SourceId(s.to_string())).collect(),
            total_tokens: 10,
        }
    }

    fn gate(inner: Arc<dyn LlmProvider>, log: Arc<SpyLog>, consented: &[&str]) -> GatedLlmProvider {
        GatedLlmProvider::new(
            inner,
            Arc::new(FakeSettings {
                consented: consented.iter().map(|s| s.to_string()).collect(),
            }),
            log,
            Arc::new(FixedClock(1000)),
        )
    }

    #[tokio::test]
    async fn local_provider_bypasses_consent_and_does_not_log() {
        // A provider whose locality() == Local should always succeed, never log.
        let fake = Arc::new(FakeLlmProvider::ok("hi").with_locality(Locality::Local));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone(), &[]); // no consents
        let (out, id) = g
            .complete_gated(
                &decision(&["a"]),
                CompletionRequest {
                    system: None,
                    prompt: "q".into(),
                    max_tokens: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(out.text, "hi");
        assert_eq!(id, None, "local provider returns no log id");
        assert_eq!(fake.call_count(), 1, "inner was called");
        assert_eq!(log.rows.lock().unwrap().len(), 0, "no log row for local");
    }

    #[tokio::test]
    async fn cloud_unconsented_denies_without_calling_or_logging() {
        let fake = Arc::new(FakeLlmProvider::ok("hi")); // default locality = Cloud
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone(), &[]); // no consents
        let err = g
            .complete_gated(
                &decision(&["a"]),
                CompletionRequest {
                    system: None,
                    prompt: "q".into(),
                    max_tokens: None,
                },
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, EgressError::Denied(EgressDenied::ConsentRequired)),
            "got {err:?}"
        );
        assert_eq!(fake.call_count(), 0, "no send");
        assert_eq!(log.rows.lock().unwrap().len(), 0, "no log row");
    }

    #[tokio::test]
    async fn consented_call_sends_once_and_logs_success() {
        let fake = Arc::new(FakeLlmProvider::ok("answer"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone(), &["kimi"]);
        let (out, id) = g
            .complete_gated(
                &decision(&["a"]),
                CompletionRequest {
                    system: None,
                    prompt: "q".into(),
                    max_tokens: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(out.text, "answer");
        let id = id.expect("cloud call returns a log id");
        assert_eq!(fake.call_count(), 1);
        let rows = log.rows.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].success);
        assert_eq!(rows[0].completed_at, 1000);
        assert_eq!(rows[0].id, id, "returned id is the logged row's id");
    }

    #[tokio::test]
    async fn inner_failure_still_logs_one_record_with_success_false() {
        let fake = Arc::new(FakeLlmProvider::failing("network down"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone(), &["kimi"]);
        let err = g
            .complete_gated(
                &decision(&["a"]),
                CompletionRequest {
                    system: None,
                    prompt: "q".into(),
                    max_tokens: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, EgressError::Completion(_)));
        let rows = log.rows.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].success);
    }

    #[tokio::test]
    async fn empty_egress_is_refused_before_any_call() {
        let fake = Arc::new(FakeLlmProvider::ok("hi"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone(), &["kimi"]);
        let err = g
            .complete_gated(
                &decision(&[]),
                CompletionRequest {
                    system: None,
                    prompt: "q".into(),
                    max_tokens: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            EgressError::Denied(EgressDenied::EmptyContext)
        ));
        assert_eq!(fake.call_count(), 0);
        assert_eq!(log.rows.lock().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn audit_log_failure_fails_the_call_and_returns_no_id() {
        // The data already left (inner was called once), but the audit write failed — the gate must
        // surface that as EgressError::Audit rather than hand back a dangling, unlogged id.
        let fake = Arc::new(FakeLlmProvider::ok("answer"));
        let g = GatedLlmProvider::new(
            fake.clone(),
            Arc::new(FakeSettings {
                consented: vec!["kimi".to_string()],
            }),
            Arc::new(FailingLog),
            Arc::new(FixedClock(1000)),
        );
        let err = g
            .complete_gated(
                &decision(&["a"]),
                CompletionRequest {
                    system: None,
                    prompt: "q".into(),
                    max_tokens: None,
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(err, EgressError::Audit(_)), "got {err:?}");
        assert_eq!(fake.call_count(), 1, "the egress did happen");
    }
}
