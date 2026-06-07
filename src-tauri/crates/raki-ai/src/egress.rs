//! The egress gate: the single, type-enforced path from an `AssembledContext` to a model call.
//! `approve` is pure policy; `GatedLlmProvider` (Task 4) is the only thing the app is handed.

use std::collections::HashSet;
use std::sync::Arc;

use raki_domain::{
    Clock, Completion, CompletionRequest, EgressDecision, EgressDenied, EgressError, EgressLog,
    EgressLogId, EgressRecord, EgressSettings, LlmProvider, Mode,
};

/// A per-call snapshot of the egress settings. Built fresh from `EgressSettings` on every call —
/// never cached — so a consent change takes effect immediately.
pub struct EgressPolicy {
    pub mode: Mode,
    pub consented: HashSet<String>,
}

/// Decide whether `decision` may leave the device under `policy`. Pure. `pub(crate)` — it is an
/// implementation detail of the gate, exposed only to this crate's tests.
pub(crate) fn approve(
    decision: &EgressDecision,
    policy: &EgressPolicy,
) -> Result<(), EgressDenied> {
    if decision.is_empty() {
        return Err(EgressDenied::EmptyContext);
    }
    match policy.mode {
        Mode::LocalOnly => Err(EgressDenied::LocalOnlyMode),
        Mode::CloudAllowed => {
            if policy.consented.contains(&decision.provider) {
                Ok(())
            } else {
                Err(EgressDenied::ConsentRequired)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::SourceId;

    fn policy(mode: Mode, consented: &[&str]) -> EgressPolicy {
        EgressPolicy {
            mode,
            consented: consented.iter().map(|s| s.to_string()).collect(),
        }
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
    fn empty_context_is_refused_regardless_of_mode() {
        let d = decision("kimi", &[]);
        assert_eq!(
            approve(&d, &policy(Mode::CloudAllowed, &["kimi"])),
            Err(EgressDenied::EmptyContext)
        );
    }

    #[test]
    fn local_only_refuses_everything() {
        let d = decision("kimi", &["a"]);
        assert_eq!(
            approve(&d, &policy(Mode::LocalOnly, &["kimi"])),
            Err(EgressDenied::LocalOnlyMode)
        );
    }

    #[test]
    fn cloud_requires_provider_consent() {
        let d = decision("kimi", &["a"]);
        assert_eq!(
            approve(&d, &policy(Mode::CloudAllowed, &[])),
            Err(EgressDenied::ConsentRequired)
        );
        assert_eq!(approve(&d, &policy(Mode::CloudAllowed, &["kimi"])), Ok(()));
    }
}

/// The ONLY way to obtain a completion. Wraps a raw provider; reads consent live; logs what
/// actually left (after the call). Constructed inside `raki-ai`; the app holds this, never the
/// raw `dyn LlmProvider`, so an un-gated call does not type-check outside this crate.
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

    pub async fn complete_gated(
        &self,
        egress: &EgressDecision,
        req: CompletionRequest,
    ) -> Result<Completion, EgressError> {
        // Live snapshot — never cached. Run the two reads concurrently.
        let (mode, consented) = tokio::try_join!(self.settings.mode(), self.settings.consented())?;
        let policy = EgressPolicy { mode, consented };
        approve(egress, &policy)?; // EgressDenied → EgressError::Denied; no send, no log row.

        let result = self.inner.complete(req).await;
        // Log AFTER the call — record what DID (or did not) leave.
        let rec = EgressRecord {
            id: EgressLogId::new(),
            decision: egress.clone(),
            completed_at: self.clock.now_ms(),
            success: result.is_ok(),
        };
        // Best-effort: a log write failure must not destroy an expensive completion result.
        let _ = self.log.record(&rec).await;
        result.map_err(EgressError::Completion)
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;
    use crate::testing::FakeLlmProvider;
    use raki_domain::{
        testing::FixedClock, DomainError, EgressRecord, EgressSettings, Mode, SourceId,
    };
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
    }

    struct FakeSettings {
        mode: Mode,
        consented: Vec<String>,
    }
    #[async_trait::async_trait]
    impl EgressSettings for FakeSettings {
        async fn mode(&self) -> Result<Mode, DomainError> {
            Ok(self.mode)
        }
        async fn consented(&self) -> Result<HashSet<String>, DomainError> {
            Ok(self.consented.iter().cloned().collect())
        }
        async fn set_mode(&self, _m: Mode) -> Result<(), DomainError> {
            Ok(())
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

    fn gate(
        inner: Arc<dyn LlmProvider>,
        log: Arc<SpyLog>,
        mode: Mode,
        consented: &[&str],
    ) -> GatedLlmProvider {
        GatedLlmProvider::new(
            inner,
            Arc::new(FakeSettings {
                mode,
                consented: consented.iter().map(|s| s.to_string()).collect(),
            }),
            log,
            Arc::new(FixedClock(1000)),
        )
    }

    #[tokio::test]
    async fn local_only_denies_without_calling_or_logging() {
        let fake = Arc::new(FakeLlmProvider::ok("hi"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone(), Mode::LocalOnly, &["kimi"]);
        let err = g
            .complete_gated(&decision(&["a"]), CompletionRequest { prompt: "q".into() })
            .await
            .unwrap_err();
        assert!(matches!(err, EgressError::Denied(_)));
        assert_eq!(fake.call_count(), 0, "no send");
        assert_eq!(log.rows.lock().unwrap().len(), 0, "no log row");
    }

    #[tokio::test]
    async fn consented_call_sends_once_and_logs_success() {
        let fake = Arc::new(FakeLlmProvider::ok("answer"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone(), Mode::CloudAllowed, &["kimi"]);
        let out = g
            .complete_gated(&decision(&["a"]), CompletionRequest { prompt: "q".into() })
            .await
            .unwrap();
        assert_eq!(out.text, "answer");
        assert_eq!(fake.call_count(), 1);
        let rows = log.rows.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].success);
        assert_eq!(rows[0].completed_at, 1000);
    }

    #[tokio::test]
    async fn inner_failure_still_logs_one_record_with_success_false() {
        let fake = Arc::new(FakeLlmProvider::failing("network down"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone(), Mode::CloudAllowed, &["kimi"]);
        let err = g
            .complete_gated(&decision(&["a"]), CompletionRequest { prompt: "q".into() })
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
        let g = gate(fake.clone(), log.clone(), Mode::CloudAllowed, &["kimi"]);
        let err = g
            .complete_gated(&decision(&[]), CompletionRequest { prompt: "q".into() })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            EgressError::Denied(EgressDenied::EmptyContext)
        ));
        assert_eq!(fake.call_count(), 0);
        assert_eq!(log.rows.lock().unwrap().len(), 0);
    }
}
