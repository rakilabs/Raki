//! End-to-end P1 integration test: note lifecycle + privacy settings + audit log.
//!
//! Run: cargo test -p raki --test p1_integration

use std::sync::Arc;

use raki_ai::AuditGate;
use raki_domain::{
    Clock, Completion, CompletionRequest, DomainError, EgressLog, EgressSettings, GatedLlmProvider,
    LlmProvider, Locality, Note, NoteRepository,
};
use raki_storage::{Database, SqliteEgressLog, SqliteEgressSettings, SqliteNoteRepository};

struct StubClock(i64);
impl Clock for StubClock {
    fn now_ms(&self) -> i64 {
        self.0
    }
}

struct SpyLlm;
#[async_trait::async_trait]
impl LlmProvider for SpyLlm {
    fn locality(&self) -> Locality {
        Locality::Cloud
    }
    async fn complete(&self, _req: CompletionRequest) -> Result<Completion, DomainError> {
        Ok(Completion {
            text: "hello".into(),
        })
    }
}

struct LocalSpyLlm;
#[async_trait::async_trait]
impl LlmProvider for LocalSpyLlm {
    fn locality(&self) -> Locality {
        Locality::Local
    }
    async fn complete(&self, _req: CompletionRequest) -> Result<Completion, DomainError> {
        Ok(Completion {
            text: "local".into(),
        })
    }
}

async fn setup() -> (
    SqliteNoteRepository,
    Arc<dyn EgressSettings>,
    Arc<dyn EgressLog>,
    Arc<dyn Clock>,
) {
    let db = Database::open_in_memory().unwrap();
    let notes = SqliteNoteRepository::new(db.clone());
    let settings: Arc<dyn EgressSettings> = Arc::new(SqliteEgressSettings::new(db.clone()));
    let log: Arc<dyn EgressLog> = Arc::new(SqliteEgressLog::new(db.clone()));
    let clock: Arc<dyn Clock> = Arc::new(StubClock(1_700_000_000_000));
    (notes, settings, log, clock)
}

#[tokio::test]
async fn note_lifecycle_create_delete_restore() {
    let (notes, _settings, _log, clock) = setup().await;

    // 1. Create
    let n = Note::new("Trip".into(), "Pack light".into(), clock.now_ms());
    let id = n.id;
    notes.upsert(&n).await.unwrap();

    // 2. List includes it
    let live = notes.list().await.unwrap();
    assert!(
        live.iter().any(|x| x.id == id),
        "live list contains the note"
    );

    // 3. Delete
    notes.soft_delete(&id, clock.now_ms()).await.unwrap();

    // 4. Live list excludes it
    let live_after = notes.list().await.unwrap();
    assert!(
        !live_after.iter().any(|x| x.id == id),
        "live list excludes deleted note"
    );

    // 5. Trash list includes it
    let trashed = notes.list_trashed().await.unwrap();
    assert!(
        trashed.iter().any(|x| x.id == id),
        "trash list includes deleted note"
    );

    // 6. Restore
    let mut revived = notes.get_any(&id).await.unwrap().unwrap();
    revived.deleted_at = None;
    revived.updated_at = clock.now_ms();
    revived.version += 1;
    notes.upsert(&revived).await.unwrap();

    // 7. Back in live list
    let live_restored = notes.list().await.unwrap();
    assert!(
        live_restored.iter().any(|x| x.id == id),
        "live list contains restored note"
    );

    // 8. Trash list empty again
    let trashed_after = notes.list_trashed().await.unwrap();
    assert!(
        !trashed_after.iter().any(|x| x.id == id),
        "trash list excludes restored note"
    );
}

#[tokio::test]
async fn settings_consent_grant_and_revoke() {
    let (_notes, settings, _log, _clock) = setup().await;

    // No consents initially
    let consented = settings.consented().await.unwrap();
    assert!(consented.is_empty(), "no consents initially");

    // Grant consent for a provider
    settings.grant("kimi").await.unwrap();
    let consented_after = settings.consented().await.unwrap();
    assert!(consented_after.contains("kimi"), "kimi is consented");

    // Revoke
    settings.revoke("kimi").await.unwrap();
    let consented_revoked = settings.consented().await.unwrap();
    assert!(!consented_revoked.contains("kimi"), "kimi consent revoked");
}

#[tokio::test]
async fn egress_log_records_cloud_calls_only() {
    let (_notes, settings, log, clock) = setup().await;

    // Initially empty
    let empty = log.list_recent(10).await.unwrap();
    assert!(empty.is_empty(), "no log entries initially");

    // Build a gated cloud provider and make a consented call
    settings.grant("kimi").await.unwrap();

    let inner: Arc<dyn LlmProvider> = Arc::new(SpyLlm);
    let gate = AuditGate::new(inner, settings, log.clone(), clock.clone());

    // Assemble a tiny context so the gate allows it
    let ctx = raki_memory::assemble_context(
        &[raki_memory::Candidate {
            source_id: "c1".into(),
            text: "hello world".into(),
            score: 1.0,
        }],
        100,
        "kimi",
        "test",
    );

    let (_, id) = gate
        .complete_gated(
            &ctx.egress,
            CompletionRequest {
                system: None,
                prompt: "hi".into(),
                max_tokens: None,
            },
        )
        .await
        .unwrap();

    assert!(id.is_some(), "cloud call returns a log id");

    // Now there should be a log entry
    let entries = log.list_recent(10).await.unwrap();
    assert_eq!(entries.len(), 1, "one egress record after a gated call");
    let rec = &entries[0];
    assert_eq!(rec.decision.provider, "kimi");
    assert!(rec.success, "spy llm succeeds");
}

#[tokio::test]
async fn local_provider_bypasses_log_and_never_records() {
    let (_notes, settings, log, clock) = setup().await;

    let inner: Arc<dyn LlmProvider> = Arc::new(LocalSpyLlm);
    let gate = AuditGate::new(inner, settings, log.clone(), clock.clone());

    let ctx = raki_memory::assemble_context(
        &[raki_memory::Candidate {
            source_id: "c1".into(),
            text: "hello world".into(),
            score: 1.0,
        }],
        100,
        "ollama",
        "llama3",
    );

    let (_, id) = gate
        .complete_gated(
            &ctx.egress,
            CompletionRequest {
                system: None,
                prompt: "hi".into(),
                max_tokens: None,
            },
        )
        .await
        .unwrap();

    assert_eq!(id, None, "local provider returns no log id");
    let entries = log.list_recent(10).await.unwrap();
    assert!(entries.is_empty(), "no log entries for local provider");
}
