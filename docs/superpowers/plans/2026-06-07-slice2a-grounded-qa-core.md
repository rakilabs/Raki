# Slice 2a — Grounded QA Core (library) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the CI-testable core of grounded cloud QA — the `MessagesProvider` cloud adapter, the `raki-generate` orchestration crate, the `AnswerState` groundedness verdict, and the persisted `grounded` bit — all driven by fakes, with no Tauri command and no UI.

**Architecture:** Bottom-up. Extend `CompletionRequest` (domain) → add the `grounded` column + `set_grounded` port (domain/storage) → make `complete_gated` return its `EgressLogId` (ai) → add the `MessagesProvider` adapter (ai) → create `raki-generate` with the pure groundedness verdict, then the `answer_question` orchestration over injected ports. Every task ends green under `cargo test --workspace --exclude raki`.

**Tech Stack:** Rust, `async_trait`, `reqwest` (rustls), `serde`/`serde_json`, `rusqlite` (STRICT), the existing `Clock`/`GatedLlmProvider`/`assemble_context`/`hybrid_search`.

**Spec:** `docs/superpowers/specs/2026-06-07-slice2-grounded-cloud-qa-design.md` (D1–D8). This plan implements **Slice 2a** (D1–D5 library parts). Slice 2b (command + UI) is a separate plan.

**Verified facts (read before starting):**
- `raki-domain/src/ports.rs`: `CompletionRequest { prompt: String }` (`#[derive(Debug)]`), `Completion { text: String }`, `trait LlmProvider { fn locality(&self)->Locality; async fn complete(&self, CompletionRequest)->Result<Completion,DomainError> }`, `enum Locality { Local, Cloud }`. `NoteRepository::get(&self, &NoteId)->Result<Option<Note>,DomainError>`. `hybrid_search(keyword:&dyn KeywordIndex, vectors:&dyn VectorIndex, embedder:&dyn EmbeddingProvider, query:&str, k:usize)->Result<Vec<String>,DomainError>`.
- `raki-domain/src/egress.rs`: `EgressLogId(Uuid)` is `Copy`, `new()/parse/Display`. `trait EgressLog { async fn record(&self,&EgressRecord)->Result<(),DomainError> }`. `EgressRecord { id, decision, completed_at, success }`. `EgressDecision { provider, model, source_ids: Vec<SourceId>, total_tokens }`. `SourceId(pub String)`.
- `raki-ai/src/egress.rs`: `GatedLlmProvider { inner, settings, log, clock }`, `complete_gated(&self, egress:&EgressDecision, req:CompletionRequest)->Result<Completion,EgressError>` currently. `gate_tests` constructs `CompletionRequest { prompt: "q".into() }` in **4 places** (lines ~223/237/254/269) and defines private `SpyLog`/`FakeSettings`.
- `raki-ai/src/testing.rs`: `FakeLlmProvider::{ok,failing,call_count}` impls `LlmProvider`.
- `raki-storage/src/migrations.rs`: `const MIGRATIONS: &[&str]` (V1–V4); `egress_log` table exists (V4). `raki-storage/src/egress.rs`: `SqliteEgressLog`/`SqliteEgressSettings`, `db.call(move |c| -> rusqlite::Result<T>).await -> Result<T,DomainError>`.
- `raki-domain/src/note.rs`: `Note { id:NoteId, title:String, body:String, .. }`.
- `raki-memory/src/context.rs`: `Candidate { source_id:String, text:String, score:f64 }`, `assemble_context(&[Candidate], budget:usize, provider:&str, model:&str)->AssembledContext` (carries `.egress`).
- Workspace `members = ["crates/*"]` (a new `crates/raki-generate` is auto-included). `[workspace.dependencies]` has `serde`, `serde_json`, `async-trait`, `thiserror`, `uuid`, `tokio`, plus path deps `raki-domain/raki-storage/raki-retrieval/raki-ai/raki-memory`. **`reqwest` is NOT yet a workspace dep.**
- The app crate (`raki`) is `--exclude`d from CI; everything here is in CI-tested library crates.

---

## File Structure

```
raki-domain/src/ports.rs        MODIFY  CompletionRequest gains system + max_tokens
raki-domain/src/egress.rs       MODIFY  EgressLog::set_grounded
raki-storage/src/migrations.rs  MODIFY  V5: ALTER egress_log ADD grounded; populated-fixture test
raki-storage/src/egress.rs      MODIFY  SqliteEgressLog::set_grounded + test
raki-storage/src/db.rs          MODIFY  register_sqlite_vec → pub(crate) (for the populated-fixture test)
raki-ai/src/egress.rs           MODIFY  complete_gated returns (Completion, EgressLogId); gate.set_grounded; SpyLog gains set_grounded; 4 tests destructure
raki-ai/Cargo.toml              MODIFY  reqwest, serde, serde_json
raki-ai/src/messages.rs         CREATE  MessagesProvider (+ pure build/parse fns + tests)
raki-ai/src/lib.rs              MODIFY  pub mod messages; re-export
raki-generate/Cargo.toml        CREATE  new crate
raki-generate/src/lib.rs        CREATE  GenerateDeps, GenerateError, Answer, AnswerState, answer_question
raki-generate/src/groundedness.rs CREATE pure verdict (evaluate) + tests
```

---

## Task 1: `CompletionRequest` gains `system` + `max_tokens`

**Files:** Modify `src-tauri/crates/raki-domain/src/ports.rs`; Modify `src-tauri/crates/raki-ai/src/egress.rs` (4 construction sites).

- [ ] **Step 1: Change the struct (this is the failing state — `raki-ai` won't compile)**

In `ports.rs`, replace the `CompletionRequest` definition:

```rust
#[derive(Debug)]
pub struct CompletionRequest {
    /// System / grounding instructions (rules + numbered context). `None` = no system message.
    pub system: Option<String>,
    /// The user's question.
    pub prompt: String,
    /// Upper bound on completion length. `None` = adapter default.
    pub max_tokens: Option<u32>,
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo build -p raki-ai --tests`
Expected: FAIL — `gate_tests` build `CompletionRequest { prompt: ... }` (missing fields).

- [ ] **Step 3: Fix the 4 construction sites in `raki-ai/src/egress.rs`**

In each of the four `gate_tests` calls, change `CompletionRequest { prompt: "q".into() }` to:

```rust
CompletionRequest { system: None, prompt: "q".into(), max_tokens: None }
```

(Use `replace_all` on the exact string `CompletionRequest { prompt: "q".into() }`.)

- [ ] **Step 4: Run**

Run: `cd src-tauri && cargo test --workspace --exclude raki`
Expected: PASS (no behavioral change; `FakeLlmProvider` ignores the request).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-domain/src/ports.rs src-tauri/crates/raki-ai/src/egress.rs
git commit -m "CompletionRequest gains system + max_tokens (Slice 2 contract)"
```

---

## Task 2: `grounded` column (V5) + `EgressLog::set_grounded`

**Files:** Modify `raki-domain/src/egress.rs`, `raki-storage/src/migrations.rs`, `raki-storage/src/egress.rs`, `raki-ai/src/egress.rs` (SpyLog).

- [ ] **Step 1: Add the port method (failing state — impls don't satisfy the trait)**

In `raki-domain/src/egress.rs`, add to `trait EgressLog`:

```rust
#[async_trait]
pub trait EgressLog: Send + Sync {
    async fn record(&self, rec: &EgressRecord) -> Result<(), DomainError>;
    /// Attach the groundedness verdict to an already-logged egress row.
    async fn set_grounded(&self, id: &EgressLogId, grounded: bool) -> Result<(), DomainError>;
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo build -p raki-storage -p raki-ai --tests`
Expected: FAIL — `SqliteEgressLog` and the test `SpyLog` don't implement `set_grounded`.

- [ ] **Step 3: Migration V5 + storage impl**

In `raki-storage/src/migrations.rs`, append to `const MIGRATIONS` (after the V4 string):

```rust
    // V5: groundedness verdict for a QA answer. Nullable: NULL = not a QA answer / no send.
    "ALTER TABLE egress_log ADD COLUMN grounded INTEGER;",
```

In `raki-storage/src/egress.rs`, add to `impl EgressLog for SqliteEgressLog` (after `record`):

```rust
    async fn set_grounded(&self, id: &EgressLogId, grounded: bool) -> Result<(), DomainError> {
        let id = id.to_string();
        let grounded = grounded as i64;
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE egress_log SET grounded = ?2 WHERE id = ?1",
                    params![id, grounded],
                )?;
                Ok(())
            })
            .await
    }
```

- [ ] **Step 4: Fix the test `SpyLog` in `raki-ai/src/egress.rs`**

In the `gate_tests` module, add to `impl EgressLog for SpyLog` (it can no-op — the gate never calls it):

```rust
        async fn set_grounded(&self, _id: &EgressLogId, _grounded: bool) -> Result<(), DomainError> {
            Ok(())
        }
```

- [ ] **Step 5: Storage round-trip test**

In `raki-storage/src/egress.rs` `mod tests`, add:

```rust
    #[tokio::test]
    async fn set_grounded_updates_the_row() {
        let db = Database::open_in_memory().unwrap();
        let log = SqliteEgressLog::new(db.clone());
        let r = rec();
        let id = r.id;
        log.record(&r).await.unwrap();
        log.set_grounded(&id, false).await.unwrap();
        let grounded: Option<i64> = db
            .call(move |c| {
                c.query_row("SELECT grounded FROM egress_log", [], |row| row.get(0))
            })
            .await
            .unwrap();
        assert_eq!(grounded, Some(0));
    }
```

- [ ] **Step 6: Migration tested on a POPULATED V4 fixture (AGENTS.md §7, lines 398/545/579/611)**

The `set_grounded` test above runs on a DB migrated empty-to-latest, so the V5 `ALTER` never sees a pre-existing row. The contract requires testing the migration on populated data. `register_sqlite_vec()` is a process-wide auto-extension, so bump its visibility and drive the migration to V4, insert a row, *then* apply V5.

In `raki-storage/src/db.rs`, change `fn register_sqlite_vec()` to `pub(crate) fn register_sqlite_vec()`.

In `raki-storage/src/migrations.rs` `mod tests`, add (the test sees the private `MIGRATIONS` + `migrate` directly):

```rust
    #[test]
    fn v5_grounded_column_applies_to_a_populated_egress_log() {
        use crate::db::register_sqlite_vec;
        use rusqlite::Connection;

        register_sqlite_vec(); // auto-extension → vec0 (V3) resolves on a raw connection
        let conn = Connection::open_in_memory().unwrap();

        // Apply V1..V4 only, then stamp the version so migrate() resumes at V5.
        for sql in &MIGRATIONS[0..4] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 4i64).unwrap();

        // Populate egress_log BEFORE the ALTER (the point of the fixture).
        conn.execute(
            "INSERT INTO egress_log (id, created_at, provider, model, token_count, source_ids, success)
             VALUES ('row1', 1, 'kimi', 'k2', 10, '[]', 1)",
            [],
        )
        .unwrap();

        // Apply the remaining migration(s) — V5's ALTER runs on the populated table.
        migrate(&conn).unwrap();

        // NOTE: ADD COLUMN ... INTEGER (nullable) is a metadata-only change in SQLite — it does not
        // rewrite existing rows. This test exists to honor the project's migration contract and to
        // catch the general class (a future backfilling migration would fail here loudly).
        let pre: Option<i64> = conn
            .query_row("SELECT grounded FROM egress_log WHERE id = 'row1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(pre, None, "the pre-existing row gets NULL grounded");
        conn.execute("UPDATE egress_log SET grounded = 0 WHERE id = 'row1'", []).unwrap();
        let post: Option<i64> = conn
            .query_row("SELECT grounded FROM egress_log WHERE id = 'row1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(post, Some(0));
    }
```

- [ ] **Step 7: Run**

Run: `cd src-tauri && cargo test --workspace --exclude raki`
Expected: PASS (V5 applies on top of V1–V4 on both empty and populated fixtures).

- [ ] **Step 8: Commit**

```bash
git add src-tauri/crates/raki-domain/src/egress.rs src-tauri/crates/raki-storage/src/migrations.rs src-tauri/crates/raki-storage/src/egress.rs src-tauri/crates/raki-storage/src/db.rs src-tauri/crates/raki-ai/src/egress.rs
git commit -m "Add egress_log.grounded (V5) + EgressLog::set_grounded port"
```

---

## Task 3: `complete_gated` returns its `EgressLogId`; `gate.set_grounded`

**Files:** Modify `raki-ai/src/egress.rs`.

- [ ] **Step 1: Update the 4 gate tests to the new return shape (failing state)**

In `gate_tests`, the consented test currently does `let out = g.complete_gated(...).await.unwrap();`. Change it to destructure and assert the id matches the logged row:

```rust
    #[tokio::test]
    async fn consented_call_sends_once_and_logs_success() {
        let fake = Arc::new(FakeLlmProvider::ok("answer"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone(), Mode::CloudAllowed, &["kimi"]);
        let (out, id) = g
            .complete_gated(&decision(&["a"]), CompletionRequest { system: None, prompt: "q".into(), max_tokens: None })
            .await
            .unwrap();
        assert_eq!(out.text, "answer");
        assert_eq!(fake.call_count(), 1);
        let rows = log.rows.lock().unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].success);
        assert_eq!(rows[0].completed_at, 1000);
        assert_eq!(rows[0].id, id, "returned id is the logged row's id");
    }
```

The other three tests use `.await.unwrap_err()` and are unchanged.

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo build -p raki-ai --tests`
Expected: FAIL — `complete_gated` returns `Completion`, not `(Completion, EgressLogId)`.

- [ ] **Step 3: Change `complete_gated` + add `set_grounded`**

In `impl GatedLlmProvider`, replace `complete_gated` and add `set_grounded`:

```rust
    pub async fn complete_gated(
        &self,
        egress: &EgressDecision,
        req: CompletionRequest,
    ) -> Result<(Completion, EgressLogId), EgressError> {
        // Live snapshot — never cached. Run the two reads concurrently.
        let (mode, consented) = tokio::try_join!(self.settings.mode(), self.settings.consented())?;
        let policy = EgressPolicy { mode, consented };
        approve(egress, &policy)?; // EgressDenied → EgressError::Denied; no send, no log row.

        let id = EgressLogId::new();
        let result = self.inner.complete(req).await;
        // Log AFTER the call — record what DID (or did not) leave. Best-effort, but surface a drop.
        let rec = EgressRecord {
            id,
            decision: egress.clone(),
            completed_at: self.clock.now_ms(),
            success: result.is_ok(),
        };
        if let Err(e) = self.log.record(&rec).await {
            eprintln!("egress audit log write failed (record dropped): {e}");
        }
        let completion = result.map_err(EgressError::Completion)?;
        Ok((completion, id))
    }

    /// Attach the groundedness verdict to a prior gated completion's log row.
    pub async fn set_grounded(
        &self,
        id: &EgressLogId,
        grounded: bool,
    ) -> Result<(), DomainError> {
        self.log.set_grounded(id, grounded).await
    }
```

(`EgressLogId` is `Copy`, so `id` is reused in the record and the return.)

- [ ] **Step 4: Run**

Run: `cd src-tauri && cargo test -p raki-ai --lib egress`
Expected: PASS — all 7 egress tests, including the 4 gate-proof tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-ai/src/egress.rs
git commit -m "complete_gated returns EgressLogId; add gate.set_grounded delegation"
```

> **Revert checkpoint (review #6):** this is the only breaking API change in the slice. The grep in the verified-facts section confirms the sole callers are this crate's four gate tests. If a hidden caller surfaces during execution (e.g. on an un-pushed branch), revert *this commit only* — Tasks 1–2 don't depend on it, and Tasks 4–6 aren't written yet — then re-plan the call sites before re-applying.

---

## Task 4: `MessagesProvider` cloud adapter

**Files:** Modify `raki-ai/Cargo.toml`; Create `raki-ai/src/messages.rs`; Modify `raki-ai/src/lib.rs`.

- [ ] **Step 1: Add dependencies**

In `raki-ai/Cargo.toml` `[dependencies]`, add:

```toml
serde = { workspace = true }
serde_json = { workspace = true }
reqwest = { version = "0.13", default-features = false, features = ["json", "rustls-tls"] }
```

- [ ] **Step 2: Create the module with pure helpers + failing tests**

Create `raki-ai/src/messages.rs`. The network call is thin; the testable logic is the pure request-body builder and response parser:

```rust
//! `MessagesProvider`: a cloud `LlmProvider` speaking the Anthropic Messages wire protocol.
//! One adapter covers Anthropic and Kimi (the team's `ckimi` shell shim points
//! `ANTHROPIC_BASE_URL` at `https://api.kimi.com/coding/`). reqwest is allowed here per AGENTS.md.

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use raki_domain::{Completion, CompletionRequest, DomainError, Locality, LlmProvider};

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
        .map_err(|_| DomainError::Provider("RAKI_LLM_BASE_URL / ANTHROPIC_BASE_URL not set".into()))?;
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| DomainError::Provider("ANTHROPIC_API_KEY not set".into()))?;
    let model = std::env::var("RAKI_LLM_MODEL")
        .map_err(|_| DomainError::Provider("RAKI_LLM_MODEL not set".into()))?;
    Ok(Config { base_url, api_key, model })
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
        .and_then(|a| a.iter().find(|b| b.get("type").and_then(|t| t.as_str()) == Some("text")))
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
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| DomainError::Provider(format!("http client: {e}")))?;
        Ok(Self { client, config: config_from_env()? })
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
        let mut last_err = None;
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
                    let bytes = r.bytes().await.map_err(|e| DomainError::Provider(e.to_string()))?;
                    if !status.is_success() {
                        return Err(DomainError::Provider(format!(
                            "messages API {status}: {}",
                            String::from_utf8_lossy(&bytes)
                        )));
                    }
                    return Ok(Completion { text: parse_response(&bytes)? });
                }
                Err(e) if attempt == 0 && (e.is_timeout() || e.is_connect()) => {
                    last_err = Some(e);
                    continue;
                }
                Err(e) => return Err(DomainError::Provider(e.to_string())),
            }
        }
        Err(DomainError::Provider(format!(
            "transport error after retry: {}",
            last_err.map(|e| e.to_string()).unwrap_or_default()
        )))
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
        let r = CompletionRequest { system: None, prompt: "hi".into(), max_tokens: None };
        let b = build_request_body(&r, "m");
        assert!(b.get("system").is_none());
        assert_eq!(b["max_tokens"], DEFAULT_MAX_TOKENS);
    }

    #[test]
    fn parse_extracts_first_text_block() {
        let bytes = br#"{"content":[{"type":"text","text":"because Rayleigh scattering"}]}"#;
        assert_eq!(parse_response(bytes).unwrap(), "because Rayleigh scattering");
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
```

- [ ] **Step 3: Wire the module**

In `raki-ai/src/lib.rs`, add `pub mod messages;` and `pub use messages::MessagesProvider;`.

- [ ] **Step 4: Run**

Run: `cd src-tauri && cargo test -p raki-ai --lib messages`
Expected: PASS (4 pure tests; the live test is `ignored`).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-ai/Cargo.toml src-tauri/crates/raki-ai/src/messages.rs src-tauri/crates/raki-ai/src/lib.rs src-tauri/Cargo.lock
git commit -m "Add MessagesProvider cloud adapter (Anthropic Messages protocol, timeout + 1 retry)"
```

---

## Task 5: `raki-generate` crate — types + pure groundedness verdict

**Files:** Create `raki-generate/Cargo.toml`, `raki-generate/src/lib.rs`, `raki-generate/src/groundedness.rs`.

- [ ] **Step 1: Create the crate manifest**

Create `src-tauri/crates/raki-generate/Cargo.toml`:

```toml
[package]
name = "raki-generate"
version.workspace = true
edition.workspace = true
license.workspace = true

[dependencies]
raki-domain = { workspace = true }
raki-retrieval = { workspace = true }
raki-memory = { workspace = true }
raki-ai = { workspace = true }
serde = { workspace = true }
serde_json = { workspace = true }

[dev-dependencies]
tokio = { workspace = true }
async-trait = { workspace = true }
```

- [ ] **Step 2: Create `groundedness.rs` with the verdict + failing tests**

Create `src-tauri/crates/raki-generate/src/groundedness.rs`:

```rust
//! The deterministic groundedness verdict. No model call: parse-or-fail-closed, then classify
//! against the context's source ids. See spec D4.

use std::collections::HashSet;

use raki_domain::SourceId;
use serde::Deserialize;

/// The answer's relationship to the retrieved context. Richer than a bool so the UI and a future
/// `qa-report` can distinguish the failure modes (spec D4 / Slice 1 line 185).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerState {
    NothingMatched,
    NotAnswerable,
    ParseFailed,
    Ungrounded,
    Grounded,
}

impl AnswerState {
    pub fn name(&self) -> &'static str {
        match self {
            AnswerState::NothingMatched => "nothing_matched",
            AnswerState::NotAnswerable => "not_answerable",
            AnswerState::ParseFailed => "parse_failed",
            AnswerState::Ungrounded => "ungrounded",
            AnswerState::Grounded => "grounded",
        }
    }
    /// The persisted bit (spec D5): only `Grounded` is true.
    pub fn is_grounded(&self) -> bool {
        matches!(self, AnswerState::Grounded)
    }
}

#[derive(Deserialize)]
struct ModelReply {
    #[serde(default)]
    answer: String,
    // `Option` so an explicit `null` (not just a missing field) is tolerated, not a parse error
    // (review #5). `unwrap_or_default()` below maps both null and missing to empty/false.
    #[serde(default)]
    cited_source_ids: Option<Vec<String>>,
    #[serde(default)]
    insufficient_context: Option<bool>,
}

/// Candidate JSON substrings in priority order: a fenced ```json … ``` block first (the model put
/// the answer there deliberately), then every balanced top-level `{…}` object. The caller try-parses
/// each and uses the first that fits `ModelReply` — so prose containing decoy braces before the real
/// object no longer forces `ParseFailed` (review #1). String contents are skipped so a `{` or `}`
/// inside a JSON string value can't miscount depth.
fn candidate_blocks(raw: &str) -> Vec<&str> {
    let mut out = Vec::new();
    if let Some(start) = raw.find("```") {
        let after = &raw[start + 3..];
        let after = after.strip_prefix("json").unwrap_or(after);
        if let Some(end) = after.find("```") {
            out.push(after[..end].trim());
        }
    }
    out.extend(balanced_objects(raw));
    out
}

/// Every top-level balanced `{…}` object in `raw`, in order, string/escape aware.
fn balanced_objects(raw: &str) -> Vec<&str> {
    let b = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'{' {
            i += 1;
            continue;
        }
        let (mut depth, mut in_str, mut esc) = (0usize, false, false);
        let mut j = i;
        while j < b.len() {
            let c = b[j];
            if in_str {
                if esc {
                    esc = false;
                } else if c == b'\\' {
                    esc = true;
                } else if c == b'"' {
                    in_str = false;
                }
            } else {
                match c {
                    b'"' => in_str = true,
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            out.push(&raw[i..=j]);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            j += 1;
        }
        i = j + 1;
    }
    out
}

fn first_parseable(raw: &str) -> Option<ModelReply> {
    candidate_blocks(raw)
        .into_iter()
        .find_map(|c| serde_json::from_str::<ModelReply>(c).ok())
}

/// Classify a raw model reply against the context ids. Returns (state, answer_text, cited).
pub fn evaluate(raw: &str, context_ids: &HashSet<String>) -> (AnswerState, String, Vec<SourceId>) {
    let Some(reply) = first_parseable(raw) else {
        return (AnswerState::ParseFailed, raw.to_string(), vec![]);
    };
    if reply.insufficient_context.unwrap_or(false) {
        return (AnswerState::NotAnswerable, reply.answer, vec![]);
    }
    // Dedup citations (m3), preserving order.
    let mut seen = HashSet::new();
    let cites: Vec<String> = reply
        .cited_source_ids
        .unwrap_or_default()
        .into_iter()
        .filter(|c| seen.insert(c.clone()))
        .collect();
    if cites.is_empty() {
        return (AnswerState::Ungrounded, reply.answer, vec![]); // review #2/M10: no provenance
    }
    if cites.iter().any(|c| !context_ids.contains(c)) {
        let ids = cites.into_iter().map(SourceId).collect();
        return (AnswerState::Ungrounded, reply.answer, ids); // fabricated citation
    }
    let ids = cites.into_iter().map(SourceId).collect();
    (AnswerState::Grounded, reply.answer, ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn grounded_when_all_cites_present() {
        let raw = r#"{"answer":"yes","cited_source_ids":["n1"],"insufficient_context":false}"#;
        let (s, text, cited) = evaluate(raw, &ctx(&["n1", "n2"]));
        assert_eq!(s, AnswerState::Grounded);
        assert_eq!(text, "yes");
        assert_eq!(cited, vec![SourceId("n1".into())]);
        assert!(s.is_grounded());
    }

    #[test]
    fn tolerates_markdown_fence() {
        let raw = "```json\n{\"answer\":\"ok\",\"cited_source_ids\":[\"n1\"]}\n```";
        assert_eq!(evaluate(raw, &ctx(&["n1"])).0, AnswerState::Grounded);
    }

    #[test]
    fn not_answerable_on_sentinel() {
        let raw = r#"{"answer":"I don't know","insufficient_context":true}"#;
        assert_eq!(evaluate(raw, &ctx(&["n1"])).0, AnswerState::NotAnswerable);
    }

    #[test]
    fn ungrounded_when_zero_citations() {
        let raw = r#"{"answer":"sky is blue","cited_source_ids":[]}"#;
        assert_eq!(evaluate(raw, &ctx(&["n1"])).0, AnswerState::Ungrounded);
    }

    #[test]
    fn ungrounded_when_citation_not_in_context() {
        let raw = r#"{"answer":"x","cited_source_ids":["n9"]}"#;
        assert_eq!(evaluate(raw, &ctx(&["n1"])).0, AnswerState::Ungrounded);
    }

    #[test]
    fn parse_failed_on_non_json() {
        assert_eq!(evaluate("I cannot comply.", &ctx(&["n1"])).0, AnswerState::ParseFailed);
    }

    #[test]
    fn skips_decoy_braces_before_the_real_json() {
        // review #1: prose with a non-JSON brace pair, then the real fenced object.
        let raw = "Here is the answer: {not available}\n```json\n{\"answer\":\"yes\",\"cited_source_ids\":[\"n1\"]}\n```";
        let (s, text, _) = evaluate(raw, &ctx(&["n1"]));
        assert_eq!(s, AnswerState::Grounded);
        assert_eq!(text, "yes");
    }

    #[test]
    fn null_citations_are_ungrounded_not_parse_failed() {
        // review #5: explicit null array → 0 citations → Ungrounded, not ParseFailed.
        let raw = r#"{"answer":"x","cited_source_ids":null,"insufficient_context":null}"#;
        assert_eq!(evaluate(raw, &ctx(&["n1"])).0, AnswerState::Ungrounded);
    }
}
```

- [ ] **Step 3: Create `lib.rs` with the types (orchestration stub comes in Task 6)**

Create `src-tauri/crates/raki-generate/src/lib.rs`:

```rust
//! Grounded QA orchestration: retrieve → assemble → gate → answer → verify. Composes the leaf
//! crates (the dependency rule forbids a leaf from doing this — see spec "Crate placement").

mod groundedness;

pub use groundedness::AnswerState;

use raki_domain::{
    DomainError, EgressError, EmbeddingProvider, KeywordIndex, NoteRepository, SourceId, VectorIndex,
};
use raki_ai::GatedLlmProvider;

/// Everything `answer_question` needs, injected so the flow is fake-testable.
pub struct GenerateDeps<'a> {
    pub keyword: &'a dyn KeywordIndex,
    pub vectors: &'a dyn VectorIndex,
    pub embedder: &'a dyn EmbeddingProvider, // assumed LOCAL (spec M4)
    pub notes: &'a dyn NoteRepository,
    pub gate: &'a GatedLlmProvider,
    pub model: &'a str,
    pub budget: usize,
    pub k: usize,
}

/// The result of a QA request.
pub struct Answer {
    pub state: AnswerState,
    pub text: String,
    pub cited_ids: Vec<SourceId>,
    pub egress_log_id: Option<raki_domain::EgressLogId>,
}

/// Non-egress vs egress failures stay distinguishable (spec C2).
#[derive(Debug)]
pub enum GenerateError {
    Egress(EgressError),
    Domain(DomainError),
}
```

- [ ] **Step 4: Run**

Run: `cd src-tauri && cargo test -p raki-generate`
Expected: PASS (6 groundedness tests; the crate compiles and is auto-discovered by `members = ["crates/*"]`).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-generate src-tauri/Cargo.lock
git commit -m "Add raki-generate crate: types + deterministic groundedness verdict"
```

---

## Task 6: `answer_question` orchestration

**Files:** Modify `raki-generate/src/lib.rs`.

- [ ] **Step 1: Write the integration test first (failing)**

Append to `raki-generate/src/lib.rs` a `#[cfg(test)] mod flow_tests`. It wires a real `GatedLlmProvider` (with `FakeLlmProvider` + in-crate fakes for settings/log/retrieval/repo) and asserts the end-to-end state. Fakes are local to this test module:

```rust
#[cfg(test)]
mod flow_tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex};

    use raki_ai::testing::FakeLlmProvider;
    use raki_domain::testing::FixedClock;
    use raki_domain::{
        Embedding, EgressLog, EgressLogId, EgressRecord, EgressSettings, KeywordHit, Mode, Note,
        NoteId, VectorHit,
    };

    // --- fakes (impl domain ports) ---
    struct OneVector(String); // returns a single source id from the vector index
    #[async_trait]
    impl VectorIndex for OneVector {
        async fn upsert(&self, _: &str, _: &Embedding) -> Result<(), DomainError> { Ok(()) }
        async fn query(&self, _: &Embedding, _: usize) -> Result<Vec<VectorHit>, DomainError> {
            Ok(vec![VectorHit { source_id: self.0.clone(), distance: 0.1 }])
        }
    }
    struct NoKeyword;
    #[async_trait]
    impl KeywordIndex for NoKeyword {
        async fn query(&self, _: &str, _: usize) -> Result<Vec<KeywordHit>, DomainError> { Ok(vec![]) }
    }
    struct FakeEmbed;
    #[async_trait]
    impl EmbeddingProvider for FakeEmbed {
        fn dimension(&self) -> usize { 1 }
        fn locality(&self) -> raki_domain::Locality { raki_domain::Locality::Local }
        fn model_id(&self) -> String { "fake".into() }
        async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, DomainError> {
            Ok(inputs.iter().map(|_| Embedding(vec![0.0])).collect())
        }
    }
    struct OneNote(NoteId);
    #[async_trait]
    impl NoteRepository for OneNote {
        async fn upsert(&self, _: &Note) -> Result<(), DomainError> { Ok(()) }
        async fn get(&self, id: &NoteId) -> Result<Option<Note>, DomainError> {
            Ok((*id == self.0).then(|| Note::new("Trip".into(), "Pay cash at the ryokan.".into(), 0)))
        }
        async fn list(&self) -> Result<Vec<Note>, DomainError> { Ok(vec![]) }
        async fn soft_delete(&self, _: &NoteId, _: i64) -> Result<(), DomainError> { Ok(()) }
    }
    struct EmptyRepo;
    #[async_trait]
    impl NoteRepository for EmptyRepo {
        async fn upsert(&self, _: &Note) -> Result<(), DomainError> { Ok(()) }
        async fn get(&self, _: &NoteId) -> Result<Option<Note>, DomainError> { Ok(None) }
        async fn list(&self) -> Result<Vec<Note>, DomainError> { Ok(vec![]) }
        async fn soft_delete(&self, _: &NoteId, _: i64) -> Result<(), DomainError> { Ok(()) }
    }
    #[derive(Default)]
    struct SpyLog { grounded: Mutex<Vec<(EgressLogId, bool)>> }
    #[async_trait]
    impl EgressLog for SpyLog {
        async fn record(&self, _: &EgressRecord) -> Result<(), DomainError> { Ok(()) }
        async fn set_grounded(&self, id: &EgressLogId, g: bool) -> Result<(), DomainError> {
            self.grounded.lock().unwrap().push((*id, g));
            Ok(())
        }
    }
    struct CloudSettings;
    #[async_trait]
    impl EgressSettings for CloudSettings {
        async fn mode(&self) -> Result<Mode, DomainError> { Ok(Mode::CloudAllowed) }
        async fn consented(&self) -> Result<HashSet<String>, DomainError> {
            Ok(HashSet::from(["kimi".to_string()]))
        }
        async fn set_mode(&self, _: Mode) -> Result<(), DomainError> { Ok(()) }
        async fn grant(&self, _: &str) -> Result<(), DomainError> { Ok(()) }
        async fn revoke(&self, _: &str) -> Result<(), DomainError> { Ok(()) }
    }

    fn gate(inner: Arc<dyn raki_domain::LlmProvider>, log: Arc<SpyLog>) -> GatedLlmProvider {
        GatedLlmProvider::new(inner, Arc::new(CloudSettings), log, Arc::new(FixedClock(1000)))
    }

    #[tokio::test]
    async fn grounded_answer_sets_grounded_true() {
        let nid = NoteId::new();
        let reply = r#"{"answer":"Pay cash.","cited_source_ids":["IDPLACEHOLDER"],"insufficient_context":false}"#
            .replace("IDPLACEHOLDER", &nid.to_string());
        let fake = Arc::new(FakeLlmProvider::ok(&reply));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake, log.clone());
        let deps = GenerateDeps {
            keyword: &NoKeyword,
            vectors: &OneVector(nid.to_string()),
            embedder: &FakeEmbed,
            notes: &OneNote(nid),
            gate: &g,
            model: "k2",
            budget: 10_000,
            k: 5,
        };
        let ans = answer_question("how do I pay at the inn?", &deps).await.unwrap();
        assert_eq!(ans.state, AnswerState::Grounded);
        assert_eq!(ans.egress_log_id.is_some(), true);
        let g = log.grounded.lock().unwrap();
        assert_eq!(g.len(), 1);
        assert!(g[0].1, "set_grounded(true) called");
    }

    #[tokio::test]
    async fn no_candidates_short_circuits_before_the_gate() {
        let nid = NoteId::new();
        let fake = Arc::new(FakeLlmProvider::ok("unused"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone());
        let deps = GenerateDeps {
            keyword: &NoKeyword,
            vectors: &OneVector(nid.to_string()),
            embedder: &FakeEmbed,
            notes: &EmptyRepo, // id retrieved but note missing → 0 candidates
            gate: &g,
            model: "k2",
            budget: 10_000,
            k: 5,
        };
        let ans = answer_question("anything", &deps).await.unwrap();
        assert_eq!(ans.state, AnswerState::NothingMatched);
        assert!(ans.egress_log_id.is_none());
        assert_eq!(fake.call_count(), 0, "no send");
        assert!(log.grounded.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn ungrounded_answer_sets_grounded_false() {
        // review #2: a SENT answer that isn't grounded must still persist the bit — as false.
        let nid = NoteId::new();
        // Valid JSON, but zero citations → Ungrounded.
        let fake = Arc::new(FakeLlmProvider::ok(r#"{"answer":"the sky is blue","cited_source_ids":[]}"#));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone());
        let deps = GenerateDeps {
            keyword: &NoKeyword,
            vectors: &OneVector(nid.to_string()),
            embedder: &FakeEmbed,
            notes: &OneNote(nid),
            gate: &g,
            model: "k2",
            budget: 10_000,
            k: 5,
        };
        let ans = answer_question("why is the sky blue?", &deps).await.unwrap();
        assert_eq!(ans.state, AnswerState::Ungrounded);
        assert_eq!(fake.call_count(), 1, "it did send");
        let grounded = log.grounded.lock().unwrap();
        assert_eq!(grounded.len(), 1);
        assert!(!grounded[0].1, "set_grounded(false) persisted for the sent-but-ungrounded answer");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cd src-tauri && cargo build -p raki-generate --tests`
Expected: FAIL — `answer_question` is not defined.

- [ ] **Step 3: Implement `answer_question` + the prompt builder**

In `raki-generate/src/lib.rs`, add the imports and function (above the test module):

```rust
use raki_domain::{CompletionRequest, NoteId};
use raki_memory::{assemble_context, AssembledContext, Candidate};
use raki_retrieval::hybrid_search;

use groundedness::evaluate;

/// System prompt: bind the model to the numbered context and the JSON reply contract (spec D4).
fn build_system_prompt(ctx: &AssembledContext) -> String {
    let mut s = String::from(
        "You answer ONLY from the notes below. Reply with a single JSON object and nothing else: \
         {\"answer\": string, \"cited_source_ids\": [string], \"insufficient_context\": bool}. \
         Cite the source_id of every note you used. If the notes do not contain the answer, set \
         insufficient_context to true.\n\nNOTES:\n",
    );
    for item in &ctx.items {
        s.push_str(&format!("[{}] {}\n", item.source_id, item.text));
    }
    s
}

pub async fn answer_question(
    query: &str,
    deps: &GenerateDeps<'_>,
) -> Result<Answer, GenerateError> {
    let ids = hybrid_search(deps.keyword, deps.vectors, deps.embedder, query, deps.k)
        .await
        .map_err(GenerateError::Domain)?;

    // Resolve ids → notes. hybrid_search already ranked best-first; keep that order via descending
    // synthetic scores (assemble_context sorts by score). Missing notes are simply skipped.
    let mut candidates = Vec::new();
    for (rank, id) in ids.iter().enumerate() {
        let nid = NoteId::parse(id).map_err(GenerateError::Domain)?;
        if let Some(note) = deps.notes.get(&nid).await.map_err(GenerateError::Domain)? {
            candidates.push(Candidate {
                source_id: id.clone(),
                text: format!("{}\n{}", note.title, note.body),
                score: (ids.len() - rank) as f64,
            });
        }
    }

    if candidates.is_empty() {
        return Ok(Answer {
            state: AnswerState::NothingMatched,
            text: "No relevant notes found.".into(),
            cited_ids: vec![],
            egress_log_id: None,
        });
    }

    let ctx = assemble_context(&candidates, deps.budget, "kimi", deps.model);
    let req = CompletionRequest {
        system: Some(build_system_prompt(&ctx)),
        prompt: query.to_string(),
        // `None` → the adapter applies its own `DEFAULT_MAX_TOKENS` (review #7: single source of truth).
        max_tokens: None,
    };

    let (completion, log_id) = deps
        .gate
        .complete_gated(&ctx.egress, req)
        .await
        .map_err(GenerateError::Egress)?;

    let context_ids = ctx.egress.source_ids.iter().map(|s| s.0.clone()).collect();
    let (state, text, cited_ids) = evaluate(&completion.text, &context_ids);

    deps.gate
        .set_grounded(&log_id, state.is_grounded())
        .await
        .map_err(GenerateError::Domain)?;

    Ok(Answer { state, text, cited_ids, egress_log_id: Some(log_id) })
}
```

Also extend the `pub use` line: `pub use groundedness::AnswerState;` stays; add `use std::collections::HashSet;` is **not** needed here (the collect target is inferred via the call site — if the compiler asks, annotate `let context_ids: std::collections::HashSet<String> = ...`).

- [ ] **Step 4: Run**

Run: `cd src-tauri && cargo test -p raki-generate`
Expected: PASS — 6 groundedness tests + 2 flow tests (grounded path sets `true`; zero-candidate path short-circuits with no send).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-generate/src/lib.rs
git commit -m "Add answer_question orchestration (retrieve→assemble→gate→verify, NothingMatched short-circuit)"
```

---

## Task 7: Verification + Definition of Done

- [ ] **Step 1: Full deterministic sweep (mirrors required CI)**

Run: `cd src-tauri && cargo test --workspace --exclude raki && cargo fmt --check && cargo clippy --workspace --exclude raki --all-targets -- -D warnings`
Expected: all pass, clean (upstream sqlite-vec C `-Wunused-parameter` warnings are not clippy findings).

- [ ] **Step 2: The groundedness branches are all proven**

Run: `cd src-tauri && cargo test -p raki-generate`
Expected: PASS — every `AnswerState` branch (Grounded / NotAnswerable / Ungrounded-0-cite / Ungrounded-bad-cite / ParseFailed) + `NothingMatched` short-circuit + `set_grounded` spy.

- [ ] **Step 3: The gate still proves egress (Slice 1 invariant intact)**

Run: `cd src-tauri && cargo test -p raki-ai --lib egress::gate_tests`
Expected: 4 passed — the return-type change did not weaken the gate.

- [ ] **Step 4: Confirm no app/frontend changes**

Run (repo root): `git diff --name-only HEAD~6 | grep -E '^src/|^src-tauri/src/' || echo "no app/frontend changes (correct)"`
Expected: prints the "no app/frontend changes" line — Slice 2a is library crates only.

- [ ] **Step 5: DoD against the spec**

D1 (`CompletionRequest` system+max_tokens) ✓ Task 1 · D2 (`MessagesProvider`, Anthropic protocol, timeout+retry, pure-fn tests) ✓ Task 4 · D3 (`answer_question` real signature, `GenerateDeps`/`GenerateError`, `NothingMatched` short-circuit, gate-only send) ✓ Tasks 5,6 · D4 (`AnswerState`, tolerant parse-or-fail-closed, dedup, 0-cite→Ungrounded) ✓ Tasks 5,6 · D5 (V5 column, `complete_gated` returns id, `set_grounded`, derived bit) ✓ Tasks 2,3,6. Crate placement honored (`raki-generate` composes leaves like `raki-eval`). No command/UI (Slice 2b) ✓ Step 4. Limitations acknowledged in the spec.

- [ ] **Step 6: Frontend sanity (unchanged)**

Run (repo root): `bun run typecheck && bun run build`
Expected: green (no frontend files changed).

---

## Self-Review

**Spec coverage:** D1→T1, D2→T4, D3→T5/T6, D4→T5/T6, D5→T2/T3/T6. The crate-placement decision is realized by `raki-generate` depending on the leaves (`raki-domain`+`raki-retrieval`+`raki-memory`+`raki-ai`), mirroring `raki-eval`. Slice 2b (command, `AnswerOutcome`, consent commands, ask-box) is explicitly out of this plan.

**Placeholder scan:** none — every step has complete code or an exact command. The one `if the compiler asks, annotate` note (Task 6 Step 3, the `HashSet` collect target) is a known type-inference fallback, not deferred work; the annotated form is supplied.

**Type/consistency:** `CompletionRequest { system: Option<String>, prompt: String, max_tokens: Option<u32> }` (T1) is constructed identically in T3's tests, T4's adapter, and T6's flow. `complete_gated(&EgressDecision, CompletionRequest) -> Result<(Completion, EgressLogId), EgressError>` (T3) is destructured in T6. `EgressLog::{record, set_grounded}` (T2) implemented by `SqliteEgressLog` (T2) and the test `SpyLog`s (T2, T6). `gate.set_grounded(&EgressLogId, bool) -> Result<(), DomainError>` (T3) called in T6. `evaluate(&str, &HashSet<String>) -> (AnswerState, String, Vec<SourceId>)` (T5) called in T6. `AnswerState::{name, is_grounded}` (T5) used for the persisted bit. `hybrid_search`/`assemble_context`/`Note`/`NoteId::parse` signatures match the verified facts.

**Known confirmations (read-and-match at implementation time):** `Database::call` closure return type (`rusqlite::Result<T>`) from `raki-storage/src/egress.rs`; that `reqwest` 0.13 (the AGENTS.md-documented version, already in `Cargo.lock`) resolves with `default-features = false, features = ["json","rustls-tls"]` (run `cargo build -p raki-ai` after Step 1 of Task 4 to surface any feature-resolution issue before writing the module).

**Applied from the plan review (`docs/raki/reviews/2026-06-07-slice2a-grounded-qa-core-plan-review.md`, "Go with fixes"):** #1 hardened JSON extraction (fence-first + try-parse balanced objects, string-aware) with a decoy-brace test; #2 `set_grounded(false)` flow test for a sent-but-ungrounded answer; #3 `reqwest 0.13` (stack alignment); #4 V5 migration tested on a populated V4 fixture (`register_sqlite_vec` → `pub(crate)`); #5 `null` citations tolerated (Option fields) → `Ungrounded`; #6 revert checkpoint on the breaking change; #7 `DEFAULT_MAX_TOKENS` de-duplicated (adapter owns it; `raki-generate` passes `None`).

---

## Execution Handoff

(Presented to the user after saving.)
