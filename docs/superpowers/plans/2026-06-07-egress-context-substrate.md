# Egress + Context-Assembly Substrate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the enforced privacy substrate every model call must pass through — `EgressDecision` in the domain kernel, `AssembledContext.egress`, a `GatedLlmProvider` that makes an un-gated cloud call un-representable, and a persisted egress log + consent — fake-tested, with no model adapter, command, or UI.

**Architecture:** Contracts live in `raki-domain` (so `raki-ai` and `raki-memory`, which can't see each other, both depend on them). `raki-memory` attaches an `EgressDecision` to the context it assembles. `raki-ai` holds the policy + the gating wrapper. `raki-storage` persists the log and consent behind domain ports. The whole loop is driven by a `FakeLlmProvider`.

**Tech Stack:** Rust, `async_trait`, `uuid` (v7), `serde`/`serde_json`, `rusqlite` (SQLite, STRICT tables), the existing `Clock` port.

**Spec:** `docs/superpowers/specs/2026-06-07-egress-context-substrate-design.md` (D1–D7 + Limitations). This plan implements all of it.

**Verified facts (read before starting):**
- `raki-domain` modules: `clock` (`trait Clock { fn now_ms(&self) -> i64 }`), `error` (`enum DomainError { NotFound, Invalid(String), Storage(String), Provider(String) }`), `ids` (`NoteId(Uuid)` with `new()`→`Uuid::now_v7()`, `parse`, `Display`, `Default`), `ports` (`LlmProvider`, `CompletionRequest { prompt }`, `Completion { text }`, all `#[async_trait]`), `testing` (`FixedClock(pub i64)`). `lib.rs` re-exports each.
- `raki-memory/src/context.rs`: `Candidate { source_id: String, text, score: f64 }`, `ContextItem { source_id: String, text, token_estimate: usize, reason }`, `AssembledContext { items, total_tokens, budget }`, `assemble_context(candidates: &[Candidate], budget: usize) -> AssembledContext` (pure, greedy by score within budget). Two existing tests.
- `raki-ai` exports `FakeEmbeddingProvider`, `FakeReranker` (the fake pattern to mirror for `FakeLlmProvider`).
- `raki-storage`: `crates/raki-storage/src/migrations.rs` holds `const MIGRATIONS: &[&str]` (V1–V3) applied by `migrate()` via `PRAGMA user_version`; `Database::open_in_memory()` runs it. The `db.call(move |c| -> rusqlite::Result<T>).await -> Result<T, DomainError>` pattern is in `src/indexing.rs` (`SqliteIndexingStore`). Tables use `STRICT`.
- The app crate (`raki`) is `--exclude`d from CI, so all logic here lives in workspace library crates that ARE tested.

---

## File Structure

```
raki-domain/src/egress.rs        CREATE  SourceId, EgressLogId, EgressDecision, EgressRecord, Mode,
                                         EgressDenied, EgressError, EgressLog, EgressSettings ports
raki-domain/src/lib.rs           MODIFY  pub mod egress; re-exports
raki-memory/src/context.rs       MODIFY  egress field on AssembledContext; egress_of; assemble_context(+provider,+model)
raki-ai/src/egress.rs            CREATE  EgressPolicy, approve() (pub(crate)), GatedLlmProvider
raki-ai/src/testing.rs           CREATE  FakeLlmProvider (reusable test util)
raki-ai/src/lib.rs               MODIFY  pub mod egress; pub mod testing (or extend) ; re-exports
raki-storage/src/migrations.rs   MODIFY  migration V4 (egress_log, cloud_consent, app_settings)
raki-storage/src/egress.rs       CREATE  SqliteEgressLog, SqliteEgressSettings
raki-storage/src/lib.rs          MODIFY  re-export both stores
```

No `src-tauri/src` (app) or `src/` (frontend) changes.

---

## Task 1: Domain contracts (`raki-domain/src/egress.rs`)

**Files:**
- Create: `src-tauri/crates/raki-domain/src/egress.rs`
- Modify: `src-tauri/crates/raki-domain/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/crates/raki-domain/src/egress.rs` with the test module first (it won't compile until Step 2 adds the types — that IS the red state):

```rust
//! The egress contract: what would leave the device, the policy ports, and the gate's error type.
//! Lives in the kernel because `raki-ai` (the gate) and `raki-memory` (the context) both need it
//! and cannot see each other.

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
        assert_eq!(d.summary(), "2 notes, 1180 tokens → kimi/k2");
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
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cd src-tauri && cargo test -p raki-domain --lib egress`
Expected: FAIL — `EgressDecision`/`SourceId`/`EgressLogId` not defined.

- [ ] **Step 3: Implement the contracts**

Prepend to `egress.rs` (above the test module):

```rust
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
            "{} notes, {} tokens → {}/{}",
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
}

/// Persist a record of what left (or attempted to leave) the device.
#[async_trait]
pub trait EgressLog: Send + Sync {
    async fn record(&self, rec: &EgressRecord) -> Result<(), DomainError>;
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
```

- [ ] **Step 4: Wire the module + run**

In `crates/raki-domain/src/lib.rs`, add `pub mod egress;` (with the other `pub mod`s) and extend the re-export list:

```rust
pub use egress::{
    EgressDecision, EgressDenied, EgressError, EgressLog, EgressLogId, EgressRecord, EgressSettings,
    Mode, SourceId,
};
```

Run: `cd src-tauri && cargo test -p raki-domain --lib egress`
Expected: PASS (both tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-domain/src/egress.rs src-tauri/crates/raki-domain/src/lib.rs
git commit -m "Add egress domain contracts (EgressDecision, ports, gate error types)"
```

---

## Task 2: `AssembledContext` carries the egress decision

**Files:**
- Modify: `src-tauri/crates/raki-memory/src/context.rs`
- Modify: `src-tauri/crates/raki-memory/Cargo.toml` (confirm `raki-domain` dep — it exists)

- [ ] **Step 1: Update the existing tests to the new signature (they become the failing test)**

In `context.rs`, the two existing tests call `assemble_context(&candidates, budget)`. Change them to pass a provider + model and assert the egress, and add an `egress_of` test:

```rust
    #[test]
    fn picks_highest_scored_first_within_budget() {
        let candidates = vec![cand("low", "aaaa", 0.1), cand("high", "bbbb", 0.9)];
        let ctx = assemble_context(&candidates, 1, "kimi", "k2");
        assert_eq!(ctx.items.len(), 1);
        assert_eq!(ctx.items[0].source_id, "high");
        assert!(ctx.total_tokens <= ctx.budget);
        // egress mirrors the included items exactly.
        assert_eq!(ctx.egress.source_ids, vec![SourceId("high".to_string())]);
        assert_eq!(ctx.egress.total_tokens, ctx.total_tokens);
        assert_eq!(ctx.egress.provider, "kimi");
    }

    #[test]
    fn includes_everything_when_budget_is_large() {
        let candidates = vec![cand("a", "x", 0.5), cand("b", "y", 0.4)];
        let ctx = assemble_context(&candidates, 10_000, "kimi", "k2");
        assert_eq!(ctx.items.len(), 2);
        assert_eq!(ctx.egress.source_ids.len(), 2);
    }

    #[test]
    fn egress_of_is_metadata_of_included_items() {
        let items = vec![ContextItem {
            source_id: "n1".into(),
            text: "hello".into(),
            token_estimate: 3,
            reason: "x".into(),
        }];
        let e = egress_of(&items, "kimi", "k2");
        assert_eq!(e.source_ids, vec![SourceId("n1".to_string())]);
        assert_eq!(e.total_tokens, 3);
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo test -p raki-memory --lib context`
Expected: FAIL — `assemble_context` takes 2 args; `egress` field and `egress_of` don't exist.

- [ ] **Step 3: Add the egress field, `egress_of`, and the signature**

At the top of `context.rs`, add `use raki_domain::{EgressDecision, SourceId};`. Add the field to the struct:

```rust
pub struct AssembledContext {
    pub items: Vec<ContextItem>,
    pub total_tokens: usize,
    pub budget: usize,
    pub egress: EgressDecision,
}
```

Add the pure helper:

```rust
/// Derive the egress metadata for an assembled set of items aimed at `provider`/`model`.
pub fn egress_of(items: &[ContextItem], provider: &str, model: &str) -> EgressDecision {
    EgressDecision {
        provider: provider.to_string(),
        model: model.to_string(),
        source_ids: items.iter().map(|i| SourceId(i.source_id.clone())).collect(),
        total_tokens: items.iter().map(|i| i.token_estimate).sum(),
    }
}
```

Change `assemble_context` to take `provider`/`model` and set `egress` (the selection loop is unchanged):

```rust
pub fn assemble_context(
    candidates: &[Candidate],
    budget: usize,
    provider: &str,
    model: &str,
) -> AssembledContext {
    // ... unchanged ranked/greedy loop producing `items` and `total_tokens` ...
    let egress = egress_of(&items, provider, model);
    AssembledContext {
        items,
        total_tokens,
        budget,
        egress,
    }
}
```

- [ ] **Step 4: Run the tests**

Run: `cd src-tauri && cargo test -p raki-memory --lib context`
Expected: PASS (the two updated tests + `egress_of_is_metadata_of_included_items`).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-memory/src/context.rs
git commit -m "AssembledContext carries EgressDecision (egress_of; assemble_context targets a provider)"
```

---

## Task 3: Egress policy (`raki-ai/src/egress.rs`) — pure `approve()`

**Files:**
- Create: `src-tauri/crates/raki-ai/src/egress.rs`
- Modify: `src-tauri/crates/raki-ai/src/lib.rs` (`pub mod egress;`)

- [ ] **Step 1: Create the module with the failing test**

Create `src-tauri/crates/raki-ai/src/egress.rs`:

```rust
//! The egress gate: the single, type-enforced path from an `AssembledContext` to a model call.
//! `approve` is pure policy; `GatedLlmProvider` (Task 4) is the only thing the app is handed.

use std::collections::HashSet;

use raki_domain::{EgressDecision, EgressDenied, Mode};

/// A per-call snapshot of the egress settings. Built fresh from `EgressSettings` on every call —
/// never cached — so a consent change takes effect immediately.
pub struct EgressPolicy {
    pub mode: Mode,
    pub consented: HashSet<String>,
}

/// Decide whether `decision` may leave the device under `policy`. Pure. `pub(crate)` — it is an
/// implementation detail of the gate, exposed only to this crate's tests.
pub(crate) fn approve(decision: &EgressDecision, policy: &EgressPolicy) -> Result<(), EgressDenied> {
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
        assert_eq!(approve(&d, &policy(Mode::CloudAllowed, &["kimi"])), Err(EgressDenied::EmptyContext));
    }

    #[test]
    fn local_only_refuses_everything() {
        let d = decision("kimi", &["a"]);
        assert_eq!(approve(&d, &policy(Mode::LocalOnly, &["kimi"])), Err(EgressDenied::LocalOnlyMode));
    }

    #[test]
    fn cloud_requires_provider_consent() {
        let d = decision("kimi", &["a"]);
        assert_eq!(approve(&d, &policy(Mode::CloudAllowed, &[])), Err(EgressDenied::ConsentRequired));
        assert_eq!(approve(&d, &policy(Mode::CloudAllowed, &["kimi"])), Ok(()));
    }
}
```

- [ ] **Step 2: Wire + run**

In `crates/raki-ai/src/lib.rs`, add `pub mod egress;`.
Run: `cd src-tauri && cargo test -p raki-ai --lib egress::tests`
Expected: PASS (three tests).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/raki-ai/src/egress.rs src-tauri/crates/raki-ai/src/lib.rs
git commit -m "Add pure egress approval policy (empty/local-only/consent)"
```

---

## Task 4: `GatedLlmProvider` + `FakeLlmProvider` — the gate is real

**Files:**
- Create: `src-tauri/crates/raki-ai/src/testing.rs`
- Modify: `src-tauri/crates/raki-ai/src/egress.rs` (add the wrapper)
- Modify: `src-tauri/crates/raki-ai/src/lib.rs` (re-exports)

- [ ] **Step 1: Add a reusable `FakeLlmProvider`**

Create `src-tauri/crates/raki-ai/src/testing.rs`:

```rust
//! Reusable test doubles for the AI crate.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use raki_domain::{Completion, CompletionRequest, DomainError, Locality, LlmProvider};

/// An `LlmProvider` that returns a canned reply (or a canned error) and counts calls.
pub struct FakeLlmProvider {
    pub reply: Result<String, String>, // Ok(text) or Err(message → DomainError::Provider)
    pub calls: Arc<AtomicUsize>,
}

impl FakeLlmProvider {
    pub fn ok(text: &str) -> Self {
        Self { reply: Ok(text.to_string()), calls: Arc::new(AtomicUsize::new(0)) }
    }
    pub fn failing(msg: &str) -> Self {
        Self { reply: Err(msg.to_string()), calls: Arc::new(AtomicUsize::new(0)) }
    }
    pub fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for FakeLlmProvider {
    fn locality(&self) -> Locality {
        Locality::Cloud
    }
    async fn complete(&self, _req: CompletionRequest) -> Result<Completion, DomainError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.reply {
            Ok(text) => Ok(Completion { text: text.clone() }),
            Err(msg) => Err(DomainError::Provider(msg.clone())),
        }
    }
}
```

(Confirm the `LlmProvider` trait's exact method set — `locality()` + `model_id()`? Mirror `FakeReranker`'s impl in `raki-ai`; if `model_id()` is required, add `fn model_id(&self) -> String { "fake".into() }`.)

- [ ] **Step 2: Add the gate with the failing test**

Append to `egress.rs`:

```rust
use std::sync::Arc;

use raki_domain::{
    Clock, Completion, CompletionRequest, EgressDecision, EgressError, EgressLog, EgressLogId,
    EgressRecord, EgressSettings, LlmProvider,
};

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
        Self { inner, settings, log, clock }
    }

    pub async fn complete_gated(
        &self,
        egress: &EgressDecision,
        req: CompletionRequest,
    ) -> Result<Completion, EgressError> {
        // Live snapshot — never cached.
        let policy = EgressPolicy {
            mode: self.settings.mode().await?,
            consented: self.settings.consented().await?,
        };
        approve(egress, &policy)?; // EgressDenied → EgressError::Denied; no send, no log row.

        let result = self.inner.complete(req).await;
        // Log AFTER the call — record what DID (or did not) leave.
        let rec = EgressRecord {
            id: EgressLogId::new(),
            decision: egress.clone(),
            completed_at: self.clock.now_ms(),
            success: result.is_ok(),
        };
        self.log.record(&rec).await?;
        result.map_err(EgressError::Completion)
    }
}
```

Add a test module section in `egress.rs` (alongside the existing `tests`) with spies:

```rust
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
            Arc::new(FakeSettings { mode, consented: consented.iter().map(|s| s.to_string()).collect() }),
            log,
            Arc::new(FixedClock(1000)),
        )
    }

    #[tokio::test]
    async fn local_only_denies_without_calling_or_logging() {
        let fake = Arc::new(FakeLlmProvider::ok("hi"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone(), Mode::LocalOnly, &["kimi"]);
        let err = g.complete_gated(&decision(&["a"]), CompletionRequest { prompt: "q".into() }).await.unwrap_err();
        assert!(matches!(err, EgressError::Denied(_)));
        assert_eq!(fake.call_count(), 0, "no send");
        assert_eq!(log.rows.lock().unwrap().len(), 0, "no log row");
    }

    #[tokio::test]
    async fn consented_call_sends_once_and_logs_success() {
        let fake = Arc::new(FakeLlmProvider::ok("answer"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone(), Mode::CloudAllowed, &["kimi"]);
        let out = g.complete_gated(&decision(&["a"]), CompletionRequest { prompt: "q".into() }).await.unwrap();
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
        let err = g.complete_gated(&decision(&["a"]), CompletionRequest { prompt: "q".into() }).await.unwrap_err();
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
        let err = g.complete_gated(&decision(&[]), CompletionRequest { prompt: "q".into() }).await.unwrap_err();
        assert!(matches!(err, EgressError::Denied(EgressDenied::EmptyContext)));
        assert_eq!(fake.call_count(), 0);
        assert_eq!(log.rows.lock().unwrap().len(), 0);
    }
}
```

- [ ] **Step 3: Wire the modules**

In `crates/raki-ai/src/lib.rs`: add `pub mod testing;` and extend egress re-exports:

```rust
pub use egress::{EgressPolicy, GatedLlmProvider};
```

- [ ] **Step 4: Run the gate tests**

Run: `cd src-tauri && cargo test -p raki-ai --lib egress`
Expected: PASS (the four `gate_tests` + the three policy tests). These four assertions are the proof the gate is real.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-ai/src/egress.rs src-tauri/crates/raki-ai/src/testing.rs src-tauri/crates/raki-ai/src/lib.rs
git commit -m "Add GatedLlmProvider: live-consent gate, post-call logging, un-gated call unrepresentable"
```

---

## Task 5: Storage — migration V4 + `SqliteEgressLog` + `SqliteEgressSettings`

**Files:**
- Modify: `src-tauri/crates/raki-storage/src/migrations.rs`
- Create: `src-tauri/crates/raki-storage/src/egress.rs`
- Modify: `src-tauri/crates/raki-storage/src/lib.rs`

- [ ] **Step 1: Add migration V4**

In `migrations.rs`, append a fourth entry to `const MIGRATIONS` (after the V3 string). Audit/system tables: `id` + timestamps, **no** soft-delete/version (per ADR-0002's "user-data" qualifier).

```rust
    // V4: egress audit log + cloud consent + a tiny settings kv (egress mode). Audit/system tables:
    // id + timestamps, no soft-delete/version (not user-data).
    "CREATE TABLE egress_log (
        id TEXT PRIMARY KEY,
        created_at INTEGER NOT NULL,
        provider TEXT NOT NULL,
        model TEXT NOT NULL,
        token_count INTEGER NOT NULL,
        source_ids TEXT NOT NULL,   -- JSON array of source id strings
        success INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE cloud_consent (
        provider TEXT PRIMARY KEY,
        granted_at INTEGER NOT NULL
    ) STRICT;
    CREATE TABLE app_settings (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) STRICT;",
```

- [ ] **Step 2: Write the storage test (failing)**

Create `src-tauri/crates/raki-storage/src/egress.rs` with the impls' test first; it won't compile until Step 3:

```rust
//! SQLite adapters for the egress audit log and the consent/mode settings.

use async_trait::async_trait;
use rusqlite::params;

use raki_domain::{
    DomainError, EgressLog, EgressRecord, EgressSettings, Mode,
};

use crate::db::Database;

pub struct SqliteEgressLog {
    db: Database,
}
impl SqliteEgressLog {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

pub struct SqliteEgressSettings {
    db: Database,
}
impl SqliteEgressSettings {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{EgressDecision, EgressLogId, SourceId};
    use std::collections::HashSet;

    fn rec() -> EgressRecord {
        EgressRecord {
            id: EgressLogId::new(),
            decision: EgressDecision {
                provider: "kimi".into(),
                model: "k2".into(),
                source_ids: vec![SourceId("n1".into()), SourceId("n2".into())],
                total_tokens: 42,
            },
            completed_at: 1000,
            success: true,
        }
    }

    #[tokio::test]
    async fn log_record_roundtrips_source_ids_json() {
        let db = Database::open_in_memory().unwrap();
        let log = SqliteEgressLog::new(db.clone());
        let r = rec();
        log.record(&r).await.unwrap();
        let (provider, ids_json, success): (String, String, i64) = db
            .call(move |c| {
                c.query_row("SELECT provider, source_ids, success FROM egress_log", [], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?))
                })
            })
            .await
            .unwrap();
        assert_eq!(provider, "kimi");
        assert_eq!(success, 1);
        let ids: Vec<String> = serde_json::from_str(&ids_json).unwrap();
        assert_eq!(ids, vec!["n1".to_string(), "n2".to_string()]);
    }

    #[tokio::test]
    async fn settings_default_local_only_then_grant_and_revoke() {
        let db = Database::open_in_memory().unwrap();
        let s = SqliteEgressSettings::new(db.clone());
        assert_eq!(s.mode().await.unwrap(), Mode::LocalOnly); // default when unset
        assert!(s.consented().await.unwrap().is_empty());
        s.set_mode(Mode::CloudAllowed).await.unwrap();
        assert_eq!(s.mode().await.unwrap(), Mode::CloudAllowed);
        s.grant("kimi").await.unwrap();
        assert_eq!(s.consented().await.unwrap(), HashSet::from(["kimi".to_string()]));
        s.revoke("kimi").await.unwrap();
        assert!(s.consented().await.unwrap().is_empty());
    }
}
```

- [ ] **Step 3: Implement the adapters**

Add the trait impls to `egress.rs` (above the test module):

```rust
#[async_trait]
impl EgressLog for SqliteEgressLog {
    async fn record(&self, rec: &EgressRecord) -> Result<(), DomainError> {
        let id = rec.id.to_string();
        let created_at = rec.completed_at;
        let provider = rec.decision.provider.clone();
        let model = rec.decision.model.clone();
        let token_count = rec.decision.total_tokens as i64;
        let ids: Vec<String> = rec.decision.source_ids.iter().map(|s| s.0.clone()).collect();
        let source_ids = serde_json::to_string(&ids)
            .map_err(|e| DomainError::Storage(format!("serialize source_ids: {e}")))?;
        let success = rec.success as i64;
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO egress_log (id, created_at, provider, model, token_count, source_ids, success)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![id, created_at, provider, model, token_count, source_ids, success],
                )?;
                Ok(())
            })
            .await
    }
}

const EGRESS_MODE_KEY: &str = "egress_mode";

#[async_trait]
impl EgressSettings for SqliteEgressSettings {
    async fn mode(&self) -> Result<Mode, DomainError> {
        let v: Option<String> = self
            .db
            .call(move |c| {
                c.query_row(
                    "SELECT value FROM app_settings WHERE key = ?1",
                    params![EGRESS_MODE_KEY],
                    |r| r.get(0),
                )
                .optional()
            })
            .await?;
        Ok(match v.as_deref() {
            Some("cloud") => Mode::CloudAllowed,
            _ => Mode::LocalOnly, // default + any unknown value ⇒ safe
        })
    }

    async fn consented(&self) -> Result<std::collections::HashSet<String>, DomainError> {
        self.db
            .call(|c| {
                let mut stmt = c.prepare("SELECT provider FROM cloud_consent")?;
                let rows = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<std::collections::HashSet<String>>>()?;
                Ok(rows)
            })
            .await
    }

    async fn set_mode(&self, mode: Mode) -> Result<(), DomainError> {
        let value = match mode {
            Mode::LocalOnly => "local",
            Mode::CloudAllowed => "cloud",
        };
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    params![EGRESS_MODE_KEY, value],
                )?;
                Ok(())
            })
            .await
    }

    async fn grant(&self, provider: &str) -> Result<(), DomainError> {
        let provider = provider.to_string();
        // granted_at: a monotonic-ish stamp; the gate doesn't read it, so a constant is fine here.
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO cloud_consent (provider, granted_at) VALUES (?1, ?2)
                     ON CONFLICT(provider) DO NOTHING",
                    params![provider, 0_i64],
                )?;
                Ok(())
            })
            .await
    }

    async fn revoke(&self, provider: &str) -> Result<(), DomainError> {
        let provider = provider.to_string();
        self.db
            .call(move |c| {
                c.execute("DELETE FROM cloud_consent WHERE provider = ?1", params![provider])?;
                Ok(())
            })
            .await
    }
}
```

Add `use rusqlite::OptionalExtension;` at the top of `egress.rs` (for `.optional()`). Confirm `raki-storage/Cargo.toml` has `serde_json` (it's used by the eval; add to `[dependencies]` if missing).

- [ ] **Step 4: Wire + run**

In `crates/raki-storage/src/lib.rs`, add `mod egress;` and re-export: `pub use egress::{SqliteEgressLog, SqliteEgressSettings};`.
Run: `cd src-tauri && cargo test -p raki-storage --lib egress`
Expected: PASS (both tests) — and the existing migration tests still pass (V4 applied on top of V1–V3).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-storage/src/migrations.rs src-tauri/crates/raki-storage/src/egress.rs src-tauri/crates/raki-storage/src/lib.rs src-tauri/crates/raki-storage/Cargo.toml
git commit -m "Add egress_log + consent/mode storage (V4) behind the domain ports"
```

---

## Task 6: Verification + Definition of Done

- [ ] **Step 1: Full deterministic sweep (mirrors required CI)**

Run: `cd src-tauri && cargo test --workspace --exclude raki && cargo fmt --check && cargo clippy --workspace --exclude raki --all-targets -- -D warnings`
Expected: all pass, clean (the upstream sqlite-vec C `-Wunused-parameter` warnings are not clippy findings).

- [ ] **Step 2: The gate-proof tests pass (the heart of the slice)**

Run: `cd src-tauri && cargo test -p raki-ai --lib egress::gate_tests`
Expected: 4 passed — `local_only_denies_without_calling_or_logging`, `consented_call_sends_once_and_logs_success`, `inner_failure_still_logs_one_record_with_success_false`, `empty_egress_is_refused_before_any_call`. Together these prove: nothing leaves without approval; consent is read live; the log records the truth.

- [ ] **Step 3: Migration applies on a populated fixture (AGENTS.md DoD)**

Run: `cd src-tauri && cargo test -p raki-storage`
Expected: PASS — including the existing V1–V3 migration tests (V4 layered cleanly) and the new egress round-trip tests.

- [ ] **Step 4: Confirm no app/frontend changes**

Run (repo root): `git diff --name-only HEAD~5 | grep -E '^src/|^src-tauri/src/' || echo "no app/frontend changes (correct)"`
Expected: prints the "no app/frontend changes" line — this slice is library-crates only.

- [ ] **Step 5: DoD against the spec**

D1 (contracts in `raki-domain`) ✓ Task 1 · D2 (`AssembledContext.egress`, `egress_of`, pure) ✓ Task 2 · D3 (`EgressPolicy` + private `approve`, live snapshot, default `LocalOnly`) ✓ Tasks 3,5 · D4 (`GatedLlmProvider` takes `&EgressDecision`, live consent, post-call logging, un-gated call unrepresentable) ✓ Task 4 · D5 (V4 audit-shaped tables, JSON `source_ids`, domain `EgressLogId`) ✓ Task 5 · D6 (metadata-only, no content/keys) ✓ (the log columns store no text) · D7 (gate is the only path) ✓ (the wrapper is the only completion entry; raw provider not re-exported to the app). Limitations acknowledged. No model adapter, command, or UI ✓ Task 6 Step 4.

- [ ] **Step 6: Frontend sanity (unchanged)**

Run (repo root): `bun run typecheck && bun run build`
Expected: green (no frontend files changed).

---

## Self-Review

**Spec coverage:** D1 → Task 1 (all contracts in `raki-domain`). D2 → Task 2 (`egress` field, `egress_of`, provider-targeted `assemble_context`). D3 → Task 3 (`EgressPolicy`, `pub(crate) approve`, default `LocalOnly` realized in Task 5's `mode()`). D4 → Task 4 (`GatedLlmProvider` with `&EgressDecision`, live snapshot, post-call log, the four proof tests). D5 → Task 5 (V4 tables, JSON source_ids, `EgressLogId` not rowid, audit-shaped per ADR-0002 qualifier). D6 → Task 5 (columns hold only metadata). D7 → only `GatedLlmProvider` is re-exported as the completion path. Limitations (coarse consent, crate-boundary enforcement, metadata-only, no backpressure) are inherent to the design, not gaps.

**Placeholder scan:** none — every step has complete code or an exact command. The two "confirm X" notes (LlmProvider method set; `serde_json` in storage Cargo) are verification asks against existing files, not deferred work.

**Type/consistency:** `EgressDecision { provider, model, source_ids: Vec<SourceId>, total_tokens }` (Task 1) is constructed identically in Tasks 2/3/4/5. `approve(&EgressDecision, &EgressPolicy) -> Result<(), EgressDenied>` (Task 3) called in Task 4. `GatedLlmProvider::complete_gated(&self, &EgressDecision, CompletionRequest) -> Result<Completion, EgressError>` (Task 4) — takes `&EgressDecision`, never `AssembledContext` (the compile-correct boundary). `EgressLog::record(&EgressRecord)` / `EgressSettings::{mode,consented,set_mode,grant,revoke}` (Task 1) implemented in Task 5, spied in Task 4. `EgressLogId::new()` (Task 1) minted in Task 4's gate and persisted in Task 5. `Mode` serialized as `"local"`/`"cloud"` (Task 5) with default `LocalOnly` — matching D3.

**Known confirmations (against existing code, not placeholders):** the `LlmProvider` trait's required methods (mirror `FakeReranker`); `serde_json` present in `raki-storage/Cargo.toml`; `Database::call` signature (from `src/indexing.rs`). All three are read-and-match, done at implementation time.

---

## Execution Handoff

(Presented to the user after saving.)
