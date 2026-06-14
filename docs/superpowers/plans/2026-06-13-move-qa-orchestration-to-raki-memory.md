# Move QA orchestration from `raki-generate` into `raki-memory`

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the `raki-generate` crate and relocate its grounded-QA orchestration into `raki-memory` behind domain ports, restoring the inward-dependency rule.

**Architecture:** `GatedLlmProvider` becomes a domain port implemented by a renamed concrete adapter in `raki-ai`. Pure groundedness evaluation moves to `raki-domain`. `raki-memory` gains `AnswerService`, which owns retrieve → assemble → gate → answer → verify. `raki-app` commands become thin adapters and the composition root wires `AnswerService` with concrete adapters.

**Tech Stack:** Rust workspace (`raki-domain`, `raki-memory`, `raki-ai`, `raki-app`), `async-trait`, Tauri, `ts-rs`.

---

## File structure

| File | Responsibility |
|---|---|
| `src-tauri/crates/raki-domain/src/egress.rs` | Adds the `GatedLlmProvider` port trait. |
| `src-tauri/crates/raki-domain/src/answer.rs` | `AnswerState`, `Answer`, `EgressPreview` value types. |
| `src-tauri/crates/raki-domain/src/groundedness.rs` | Pure `evaluate` function and its unit tests. |
| `src-tauri/crates/raki-domain/src/lib.rs` | Re-export new domain types and the port. |
| `src-tauri/crates/raki-ai/src/egress.rs` | Renames concrete gate to `AuditGate`; implements `raki_domain::GatedLlmProvider`. |
| `src-tauri/crates/raki-ai/src/lib.rs` | Re-export `AuditGate` instead of `GatedLlmProvider`. |
| `src-tauri/crates/raki-memory/src/answer.rs` | `AnswerConfig`, `GenerateError`, `AnswerService`, and migrated unit tests. |
| `src-tauri/crates/raki-memory/src/lib.rs` | Re-export `AnswerService`, `AnswerConfig`, `GenerateError`. |
| `src-tauri/src/state.rs` | Add `answer_service: Arc<AnswerService>`; change `gate` to `Arc<dyn GatedLlmProvider>`. |
| `src-tauri/src/lib.rs` | Construct `AnswerService` and `AuditGate`; remove `raki-generate` imports. |
| `src-tauri/src/commands/qa.rs` | Delegate to `state.answer_service` instead of `raki-generate` functions. |
| `src-tauri/src/error.rs` | Import `GenerateError` from `raki-memory`. |
| `src-tauri/Cargo.toml` | Remove `raki-generate` dependency. |
| `src-tauri/crates/raki-generate/` | Delete the entire crate. |
| `CONTEXT.md` | Already updated with new terms; verify no further terms are needed. |

---

## Task 1: Promote `GatedLlmProvider` to a domain port

**Files:**
- Modify: `src-tauri/crates/raki-domain/src/egress.rs`
- Modify: `src-tauri/crates/raki-domain/src/lib.rs`
- Test: `cargo check -p raki-domain`

- [ ] **Step 1: Add the `GatedLlmProvider` trait**

Append to `src-tauri/crates/raki-domain/src/egress.rs`, after the `EgressSettings` trait:

```rust
#[async_trait]
pub trait GatedLlmProvider: Send + Sync {
    /// Complete via the inner provider after enforcing locality-aware egress policy.
    /// Returns the completion and, for cloud providers, the audit-log id.
    async fn complete_gated(
        &self,
        egress: &EgressDecision,
        req: CompletionRequest,
    ) -> Result<(Completion, Option<EgressLogId>), EgressError>;

    /// Attach the groundedness verdict to a prior gated completion's log row.
    async fn set_grounded(&self, id: &EgressLogId, grounded: bool) -> Result<(), DomainError>;
}
```

Add `Completion` and `CompletionRequest` to the imports at the top of `egress.rs` if not already present. They are defined in `ports.rs`; import them:

```rust
use crate::ports::{Completion, CompletionRequest};
```

- [ ] **Step 2: Re-export from `raki-domain`**

Modify `src-tauri/crates/raki-domain/src/lib.rs` to include `GatedLlmProvider` in the egress re-export. Find the existing egress module re-export and update it:

```rust
pub mod egress;
pub use egress::{
    EgressDecision, EgressDenied, EgressError, EgressLog, EgressLogId, EgressRecord,
    EgressSettings, GatedLlmProvider, SourceId,
};
```

- [ ] **Step 3: Verify `raki-domain` compiles**

```bash
cd src-tauri && cargo check -p raki-domain
```

Expected: clean check.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-domain/src/egress.rs src-tauri/crates/raki-domain/src/lib.rs
git commit -m "domain: promote GatedLlmProvider to a port trait"
```

---

## Task 2: Rename `raki-ai` concrete gate and implement the port

**Files:**
- Modify: `src-tauri/crates/raki-ai/src/egress.rs`
- Modify: `src-tauri/crates/raki-ai/src/lib.rs`
- Test: `cargo test -p raki-ai`

- [ ] **Step 1: Rename concrete type to `AuditGate` and implement the domain port**

In `src-tauri/crates/raki-ai/src/egress.rs`:

1. Rename the struct and its `impl` block:

```rust
pub struct AuditGate {
    inner: Arc<dyn LlmProvider>,
    settings: Arc<dyn EgressSettings>,
    log: Arc<dyn EgressLog>,
    clock: Arc<dyn Clock>,
}

impl AuditGate {
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
}
```

2. Add the domain port implementation after the `impl AuditGate` block:

```rust
#[async_trait::async_trait]
impl raki_domain::GatedLlmProvider for AuditGate {
    async fn complete_gated(
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

        let consented = self.settings.consented().await?;
        approve(egress, &consented)?;

        let id = EgressLogId::new();
        let result = self.inner.complete(req).await;
        let rec = EgressRecord {
            id,
            decision: egress.clone(),
            completed_at: self.clock.now_ms(),
            success: result.is_ok(),
        };
        if let Err(e) = self.log.record(&rec).await {
            return Err(EgressError::Audit(e.to_string()));
        }
        let completion = result.map_err(EgressError::Completion)?;
        Ok((completion, Some(id)))
    }

    async fn set_grounded(&self, id: &EgressLogId, grounded: bool) -> Result<(), DomainError> {
        self.log.set_grounded(id, grounded).await
    }
}
```

3. Update all test constructor calls in the same file from `GatedLlmProvider::new(...)` to `AuditGate::new(...)`. There are three occurrences in the `gate_tests` module.

- [ ] **Step 2: Update `raki-ai` public exports**

Modify `src-tauri/crates/raki-ai/src/lib.rs`:

```rust
pub use egress::{AuditGate, GatedLlmProvider};
```

Wait: `GatedLlmProvider` is now a domain trait. Do not re-export it from `raki-ai` to avoid confusion. Only export the concrete adapter:

```rust
pub use egress::AuditGate;
```

- [ ] **Step 3: Run `raki-ai` tests**

```bash
cd src-tauri && cargo test -p raki-ai
```

Expected: all existing gate tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-ai/src/egress.rs src-tauri/crates/raki-ai/src/lib.rs
git commit -m "ai: rename concrete gate to AuditGate and implement domain GatedLlmProvider port"
```

---

## Task 3: Move answer value types and groundedness to `raki-domain`

**Files:**
- Create: `src-tauri/crates/raki-domain/src/answer.rs`
- Create: `src-tauri/crates/raki-domain/src/groundedness.rs`
- Modify: `src-tauri/crates/raki-domain/src/lib.rs`
- Test: `cargo test -p raki-domain`

- [ ] **Step 1: Create `raki-domain/src/answer.rs`**

Create `src-tauri/crates/raki-domain/src/answer.rs` with the answer value types:

```rust
//! Domain types for the grounded answer flow.

use crate::egress::{EgressDecision, EgressLogId, SourceId};

/// The answer's relationship to the retrieved context.
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
    pub fn is_grounded(&self) -> bool {
        matches!(self, AnswerState::Grounded)
    }
}

/// The result of a gated answer request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub state: AnswerState,
    pub text: String,
    pub cited_ids: Vec<SourceId>,
    pub egress_log_id: Option<EgressLogId>,
}

/// What a cloud send WOULD disclose — metadata only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EgressPreview {
    pub provider: String,
    pub summary: String,
    pub source_titles: Vec<String>,
}
```

- [ ] **Step 2: Create `raki-domain/src/groundedness.rs`**

Copy the entire contents of `src-tauri/crates/raki-generate/src/groundedness.rs` into `src-tauri/crates/raki-domain/src/groundedness.rs`, changing the module doc and exports only. The code is already pure and depends only on `raki_domain::SourceId` and `serde`.

Top of file:

```rust
//! The deterministic groundedness verdict. No model call: parse-or-fail-closed, then classify
//! against the context's source ids.

use std::collections::HashSet;

use crate::SourceId;
use serde::Deserialize;
```

Keep the rest of the file identical to the original `groundedness.rs`, including tests.

- [ ] **Step 3: Re-export from `raki-domain`**

Modify `src-tauri/crates/raki-domain/src/lib.rs` to add the two new modules:

```rust
pub mod answer;
pub mod groundedness;

pub use answer::{Answer, AnswerState, EgressPreview};
pub use groundedness::evaluate;
```

- [ ] **Step 4: Run `raki-domain` tests**

```bash
cd src-tauri && cargo test -p raki-domain
```

Expected: existing tests pass plus migrated groundedness tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-domain/src/answer.rs src-tauri/crates/raki-domain/src/groundedness.rs src-tauri/crates/raki-domain/src/lib.rs
git commit -m "domain: move answer types and groundedness evaluation to raki-domain"
```

---

## Task 4: Create `AnswerService` in `raki-memory`

**Files:**
- Create: `src-tauri/crates/raki-memory/src/answer.rs`
- Modify: `src-tauri/crates/raki-memory/src/lib.rs`
- Test: `cargo test -p raki-memory`

- [ ] **Step 1: Create `raki-memory/src/answer.rs`**

Create `src-tauri/crates/raki-memory/src/answer.rs` with the service. Copy the orchestration logic from `raki-generate/src/lib.rs`, adapting it to a struct with constructor.

```rust
//! Grounded answer orchestration: retrieve → assemble → gate → answer → verify.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;

use raki_domain::{
    body_to_text, Answer, AnswerState, DomainError, EgressDenied, EgressError, EgressLogId,
    EgressPreview, EmbeddingProvider, GatedLlmProvider, KeywordIndex, NoteId, NoteRepository,
    QueryRewriter, SourceId, VectorIndex,
};
use raki_domain::{CompletionRequest, EgressDecision};
use raki_retrieval::hybrid_search;

use crate::context::{assemble_context, Candidate};
use crate::groundedness::evaluate;

#[derive(Clone, Debug)]
pub struct AnswerConfig {
    pub provider: String,
    pub model: String,
    pub k: usize,
    pub budget: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum GenerateError {
    #[error("egress denied or failed: {0}")]
    Egress(#[from] EgressError),
    #[error("domain error: {0}")]
    Domain(#[from] DomainError),
}

/// Either a verified answer or a preview shown when consent is required.
/// Bundling the preview avoids a second retrieval pass in the consent-denied path.
pub enum AnswerResult {
    Answer(Answer),
    NeedsConsent(EgressPreview),
}

pub struct AnswerService {
    keyword: Arc<dyn KeywordIndex>,
    vectors: Arc<dyn VectorIndex>,
    embedder: Arc<dyn EmbeddingProvider>,
    notes: Arc<dyn NoteRepository>,
    gate: Arc<dyn GatedLlmProvider>,
    config: AnswerConfig,
}

impl AnswerService {
    pub fn new(
        keyword: Arc<dyn KeywordIndex>,
        vectors: Arc<dyn VectorIndex>,
        embedder: Arc<dyn EmbeddingProvider>,
        notes: Arc<dyn NoteRepository>,
        gate: Arc<dyn GatedLlmProvider>,
        config: AnswerConfig,
    ) -> Self {
        Self {
            keyword,
            vectors,
            embedder,
            notes,
            gate,
            config,
        }
    }

    pub async fn answer(
        &self,
        query: &str,
        rewriter: Option<&dyn QueryRewriter>,
    ) -> Result<AnswerResult, GenerateError> {
        let Some((ctx, _titles)) = self.assemble(query, rewriter).await? else {
            return Ok(AnswerResult::Answer(Answer {
                state: AnswerState::NothingMatched,
                text: "No relevant notes found.".into(),
                cited_ids: vec![],
                egress_log_id: None,
            }));
        };
        match self.send(&ctx, query).await {
            Ok(ans) => Ok(AnswerResult::Answer(ans)),
            Err(GenerateError::Egress(EgressError::Denied(EgressDenied::ConsentRequired))) => {
                let preview = self.preview_from_context(&ctx, &_titles);
                Ok(AnswerResult::NeedsConsent(preview))
            }
            Err(e) => Err(e),
        }
    }

    pub async fn preview(
        &self,
        query: &str,
        rewriter: Option<&dyn QueryRewriter>,
    ) -> Result<Option<EgressPreview>, GenerateError> {
        let Some((ctx, titles)) = self.assemble(query, rewriter).await? else {
            return Ok(None);
        };
        Ok(Some(self.preview_from_context(&ctx, &titles)))
    }

    fn preview_from_context(
        &self,
        ctx: &crate::context::AssembledContext,
        titles: &HashMap<String, String>,
    ) -> EgressPreview {
        let source_titles = ctx
            .egress
            .source_ids
            .iter()
            .map(|s| titles.get(&s.0).cloned().unwrap_or_else(|| s.0.clone()))
            .collect();
        EgressPreview {
            provider: self.config.provider.clone(),
            summary: ctx.egress.summary(),
            source_titles,
        }
    }

    async fn assemble(
        &self,
        query: &str,
        rewriter: Option<&dyn QueryRewriter>,
    ) -> Result<Option<(crate::context::AssembledContext, HashMap<String, String>)>, GenerateError> {
        let ids = hybrid_search(
            self.keyword.as_ref(),
            self.vectors.as_ref(),
            self.embedder.as_ref(),
            rewriter,
            query,
            self.config.k,
        )
        .await
        .map_err(GenerateError::Domain)?;

        let mut candidates = Vec::new();
        let mut titles = HashMap::new();
        for (rank, id) in ids.iter().enumerate() {
            if let Some(note) = self.notes.get(id).await.map_err(GenerateError::Domain)? {
                titles.insert(id.to_string(), note.title.clone());
                candidates.push(Candidate {
                    source_id: id.to_string(),
                    text: format!("{}\n{}", note.title, body_to_text(&note.body)),
                    score: (ids.len() - rank) as f64,
                });
            }
        }
        if candidates.is_empty() {
            return Ok(None);
        }
        let ctx = assemble_context(
            &candidates,
            self.config.budget,
            &self.config.provider,
            &self.config.model,
        );
        Ok(Some((ctx, titles)))
    }

    async fn send(
        &self,
        ctx: &crate::context::AssembledContext,
        query: &str,
    ) -> Result<Answer, GenerateError> {
        let req = CompletionRequest {
            system: Some(build_system_prompt(ctx)),
            prompt: query.to_string(),
            max_tokens: None,
        };
        let (completion, log_id) = self
            .gate
            .complete_gated(&ctx.egress, req)
            .await
            .map_err(GenerateError::Egress)?;
        let context_ids: std::collections::HashSet<String> =
            ctx.egress.source_ids.iter().map(|s| s.0.clone()).collect();
        let (state, text, cited_ids) = evaluate(&completion.text, &context_ids);
        if let Some(id) = log_id {
            self.gate
                .set_grounded(&id, state.is_grounded())
                .await
                .map_err(GenerateError::Domain)?;
        }
        Ok(Answer {
            state,
            text,
            cited_ids,
            egress_log_id: log_id,
        })
    }
}

fn build_system_prompt(ctx: &crate::context::AssembledContext) -> String {
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
```

- [ ] **Step 2: Re-export from `raki-memory`**

Modify `src-tauri/crates/raki-memory/src/lib.rs`:

```rust
mod answer;
mod chunk;
mod context;
pub mod indexing;
pub mod signals;

pub use answer::{AnswerConfig, AnswerResult, AnswerService, GenerateError};
pub use chunk::chunk_note;
pub use context::{assemble_context, AssembledContext, Candidate, ContextItem};
pub use signals::DefaultSignalBooster;
```

- [ ] **Step 3: Migrate tests**

Copy the `flow_tests` module from `src-tauri/crates/raki-generate/src/lib.rs` into `src-tauri/crates/raki-memory/src/answer.rs` under `#[cfg(test)]`. Update the test helpers:

- Replace `use raki_ai::GatedLlmProvider;` with `use raki_ai::AuditGate;`.
- Replace `use raki_generate::{assemble_for, send_answer, ...}` with imports from `crate::answer::{AnswerService, GenerateError}`.
- Replace `GenerateDeps` construction with `AnswerService::new(...)` and per-call `answer()` / `preview()`.
- Replace `gate(...)` helper to return `Arc<dyn GatedLlmProvider>` wrapping `AuditGate`.

Example test migration for `grounded_answer_sets_grounded_true`:

```rust
#[tokio::test]
async fn grounded_answer_sets_grounded_true() {
    let nid = NoteId::new();
    let reply = r#"{"answer":"Pay cash.","cited_source_ids":["IDPLACEHOLDER"],"insufficient_context":false}"#
        .replace("IDPLACEHOLDER", &nid.to_string());
    let fake = Arc::new(FakeLlmProvider::ok(&reply));
    let log = Arc::new(SpyLog::default());
    let g = gate(fake, log.clone());
    let note = OneNote(nid);
    let vec = OneVector(nid.to_string());
    let svc = service(&g, &note, &vec);
    let ans = match svc.answer("how do I pay at the inn?", None).await.unwrap() {
        AnswerResult::Answer(a) => a,
        _ => panic!("expected an answer"),
    };
    assert_eq!(ans.state, AnswerState::Grounded);
    assert!(ans.egress_log_id.is_some());
    let g = log.grounded.lock().unwrap();
    assert_eq!(g.len(), 1);
    assert!(g[0].1, "set_grounded(true) called");
}
```

The `service` helper constructs `AnswerService::new(keyword, vectors, embedder, notes, gate, config)`.

- [ ] **Step 4: Run `raki-memory` tests**

```bash
cd src-tauri && cargo test -p raki-memory
```

Expected: all migrated flow tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-memory/src/answer.rs src-tauri/crates/raki-memory/src/lib.rs
git commit -m "memory: add AnswerService and migrate QA orchestration tests"
```

---

## Task 5: Update `raki-app` wiring, commands, and error mapping

**Files:**
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands/qa.rs`
- Modify: `src-tauri/src/error.rs`
- Modify: `src-tauri/Cargo.toml`
- Test: `cargo check -p raki`

- [ ] **Step 1: Update `AppState`**

In `src-tauri/src/state.rs`:

1. Add import for `AnswerService`:

```rust
use raki_memory::AnswerService;
```

2. Change `gate` field type:

```rust
/// The only cloud-completion path (wraps MessagesProvider; reads consent live; logs egress).
pub gate: Arc<dyn GatedLlmProvider>,
```

3. Add `answer_service` field:

```rust
pub answer_service: Arc<AnswerService>,
```

- [ ] **Step 2: Update composition root in `src-tauri/src/lib.rs`**

1. Change imports:

```rust
use raki_ai::{AuditGate, CloudQueryRewriter, FakeEmbeddingProvider, FastEmbedProvider, MessagesProvider};
use raki_domain::{Clock, GatedLlmProvider, /* ... keep rest ... */};
use raki_memory::{signals::DefaultSignalBooster, AnswerConfig, AnswerService};
```

2. Replace the `GatedLlmProvider::new(...)` construction with `AuditGate::new(...)`:

```rust
let gate: Arc<dyn GatedLlmProvider> = Arc::new(AuditGate::new(
    inner,
    settings.clone(),
    egress_log.clone(),
    clock.clone(),
));
```

3. Do the same for the `rewrite_gate` construction inside the rewriter block:

```rust
let rewrite_gate: Arc<dyn GatedLlmProvider> = Arc::new(AuditGate::new(
    rewrite_inner,
    settings.clone(),
    egress_log.clone(),
    clock.clone(),
));
```

4. Construct `AnswerService` before `app.manage(...)`:

```rust
let answer_service = Arc::new(AnswerService::new(
    keyword.clone(),
    vectors.clone(),
    embedder.clone(),
    notes.clone(),
    gate.clone(),
    AnswerConfig {
        provider: provider.clone(),
        model: model.clone(),
        k: 10,
        budget: 2000,
    },
));
```

5. Add `answer_service` and update `gate` in the `AppState` struct literal:

```rust
app.manage(AppState {
    notes,
    keyword,
    vectors,
    embedder,
    reranker,
    clock,
    index,
    gate,
    answer_service,
    settings,
    egress_log,
    provider,
    model,
    k: 10,
    budget_tokens: 2000,
    rewriter,
    signal_source: signals.clone(),
    signal_store: signal_store.clone(),
    signal_booster: signal_booster.clone(),
});
```

- [ ] **Step 3: Rewrite `commands/qa.rs`**

Replace `src-tauri/src/commands/qa.rs` with a thin adapter:

```rust
//! Grounded-QA command adapter: translate + delegate to `AnswerService`. No business logic.

use tauri::State;

use raki_domain::AnswerState;
use raki_memory::AnswerResult;

use crate::dto::{AnswerOutcome, CitedNote, EgressPreviewDto};
use crate::error::AppError;
use crate::state::AppState;

#[tauri::command]
pub async fn answer_question(
    state: State<'_, AppState>,
    query: String,
) -> Result<AnswerOutcome, AppError> {
    let rewriter = state.rewriter.as_ref().map(|r| r.as_ref());

    match state.answer_service.answer(&query, rewriter).await? {
        AnswerResult::Answer(ans) if ans.state == AnswerState::NothingMatched => {
            Ok(AnswerOutcome::Answer {
                state: AnswerState::NothingMatched.name().to_string(),
                text: "No relevant notes found.".into(),
                cited: vec![],
            })
        }
        AnswerResult::Answer(ans) => Ok(AnswerOutcome::Answer {
            state: ans.state.name().to_string(),
            text: ans.text,
            cited: ans
                .cited_ids
                .into_iter()
                .map(|sid| CitedNote {
                    id: sid.0.clone(),
                    title: sid.0,
                })
                .collect(),
        }),
        AnswerResult::NeedsConsent(preview) => Ok(AnswerOutcome::NeedsConsent {
            preview: EgressPreviewDto {
                provider: preview.provider,
                summary: preview.summary,
                source_titles: preview.source_titles,
            },
        }),
    }
}
```

Wait: the original command builds the preview from the already-assembled context titles to avoid a second retrieval pass. The new adapter calls `preview()` again, which does another retrieval. That is wasteful. Instead, `AnswerService::answer` should return the preview alongside the answer when consent is required, or we should expose an `AnswerResult` enum.

Better design: change `AnswerService::answer` to return an enum:

```rust
pub enum AnswerOutcomeInternal {
    Answer(Answer),
    NeedsConsent(EgressPreview),
}
```

But that complicates the interface. Alternatively, add a method `answer_with_preview` or keep `preview()` and accept the extra retrieval. For now, since the plan must be concrete, let's adjust: have `AnswerService::answer` return `Result<AnswerResult, GenerateError>` where `AnswerResult` is either an `Answer` or a preview for consent.

Actually, looking at the original code, it assembles once, then tries to send. If denied, it returns the preview built from the same context. The cleanest is to make `AnswerService::answer` return an enum. Let me update Task 4 to include this.

So in Task 4 Step 1, change `AnswerService::answer` to:

```rust
pub enum AnswerResult {
    Answer(Answer),
    NeedsConsent(EgressPreview),
}
```

And `answer` returns `Result<AnswerResult, GenerateError>`.

Then `commands/qa.rs` becomes:

```rust
match state.answer_service.answer(&query, rewriter).await? {
    AnswerResult::Answer(ans) if ans.state == AnswerState::NothingMatched => ...
    AnswerResult::Answer(ans) => ...
    AnswerResult::NeedsConsent(preview) => ...
}
```

This avoids double retrieval and keeps the command thin.

Update Task 4 accordingly.

- [ ] **Step 4: Update error mapping**

In `src-tauri/src/error.rs`:

```rust
use raki_memory::GenerateError;
```

Remove `use raki_generate::GenerateError;`.

- [ ] **Step 5: Remove `raki-generate` from `raki` crate dependencies**

In `src-tauri/Cargo.toml`, remove these two lines:

```toml
raki-generate = { workspace = true }
```

from `[workspace.dependencies]` and `[dependencies]`.

- [ ] **Step 6: Verify `raki` crate compiles**

```bash
cd src-tauri && cargo check -p raki
```

Expected: clean check.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/state.rs src-tauri/src/lib.rs src-tauri/src/commands/qa.rs src-tauri/src/error.rs src-tauri/Cargo.toml
git commit -m "app: wire AnswerService and remove raki-generate dependency"
```

---

## Task 6: Delete the `raki-generate` crate

**Files:**
- Delete: `src-tauri/crates/raki-generate/`
- Test: `cargo test --workspace`

- [ ] **Step 1: Delete crate directory**

```bash
rm -rf src-tauri/crates/raki-generate
```

- [ ] **Step 2: Remove from workspace dependencies**

Confirm `src-tauri/Cargo.toml` no longer references `raki-generate`. The `[workspace]` member glob (`members = ["crates/*"]`) automatically excludes deleted crates, but verify no explicit references remain.

- [ ] **Step 3: Run workspace tests**

```bash
cd src-tauri && cargo test --workspace
```

Expected: all workspace tests pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/Cargo.toml
git commit -m "chore: delete raki-generate crate"
```

---

## Task 7: Final verification

**Files:**
- All touched files.

- [ ] **Step 1: Build and lint**

```bash
cd src-tauri && cargo build --workspace
cd src-tauri && cargo clippy --workspace -- -D warnings
cd src-tauri && cargo fmt --check
```

Expected: clean build, no clippy warnings, formatting clean.

- [ ] **Step 2: Regenerate IPC bindings if needed**

No DTOs changed (`AnswerOutcome` shape is unchanged), so no regeneration is required. Verify with:

```bash
cd src-tauri && cargo check -p raki
```

- [ ] **Step 3: Commit final verification fixes**

```bash
git commit -am "style: formatting and clippy fixes for QA move"
```

---

## Self-review

**Spec coverage:**
- Move QA orchestration from `raki-generate` to `raki-memory` — Tasks 1–4.
- Restore inward dependency rule — Tasks 5–6.
- Keep commands thin — Task 5.
- Preserve fake-adapter testability — Tasks 3–4.
- Delete `raki-generate` — Task 6.

**Placeholder scan:**
- No "TBD", "TODO", or "implement later" strings.
- All code steps contain concrete code.
- All commands include expected output.

**Type consistency:**
- `GatedLlmProvider` is the domain port throughout.
- `AuditGate` is the concrete adapter name throughout.
- `AnswerService` constructor and methods are consistent across tasks.
- `GenerateError` is imported from `raki-memory` in `error.rs`.

**One open issue addressed during planning:**
- The first draft of `commands/qa.rs` called `preview()` again on `ConsentRequired`, which would re-run retrieval. Task 4 now defines `AnswerResult::NeedsConsent(EgressPreview)` so the service returns the preview from the same assembled context.
