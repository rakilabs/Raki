# Slice 2b — Grounded QA App Shell Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Slice 2a's grounded-QA core user-facing — an `answer_question` Tauri command returning a typed `AnswerOutcome` (consent-preview *or* answer), `grant_cloud_consent`/`revoke_cloud_consent` commands, and an opt-in ask-box (in the app shell) with the preview-then-consent flow. Also **repair the app crate**, which currently does not compile.

**Architecture:** The testable preview logic lives in `raki-generate` (a `preview()` function, CI-tested with fakes). The app crate (`raki`) is the thin shell: it wires the live egress stack (`SqliteEgressSettings` + `SqliteEgressLog` + `MessagesProvider` → `GatedLlmProvider`) into `AppState`, exposes three commands, and the SolidJS shell renders the ask-box. The frontend follows the existing typed-`commands` + per-module-`api` + `@tanstack/solid-query` conventions.

**Tech Stack:** Rust, `tauri` v2, `ts-rs` (TS bindings), SolidJS, `@tanstack/solid-query`, `vitest`.

**Spec:** `docs/superpowers/specs/2026-06-07-slice2-grounded-cloud-qa-design.md` (D5 command, D6 outcome enum, D7 consent flow). Slice 2a (library core) is committed (`981cb77`, `417ccc3`).

**Applied from the plan review** (`docs/raki/reviews/2026-06-07-slice2b-grounded-qa-app-plan-review.md`, "Conditional Go"): #1 add `raki-generate` app dep; #2 `needs_consent` is sync (no `.await` in a match guard); #3 use `state.inner()`; #4/#5 oversized steps split; #6 document the revoke two-factor; #7 register handlers in Task 3; #8 mount `<AskBox>` in the shell (AGENTS.md §5 forbids module↔module imports); #9 add `AskBox` tests; #10 walkthrough covers notes regression; #13 diff bindings before staging. Plus a ctx7-found fix: `AnswerOutcome` needs `#[ts(tag = "kind")]`, not just `#[serde(tag)]`, for ts-rs to emit a discriminated union.

> **⚠ Pre-existing breakage this plan repairs.** The `raki` app crate is `--exclude`d from CI, and Slice 1 refactored `EgressPolicy` from an enum into a struct. `src-tauri/src/lib.rs:74` still reads `egress: EgressPolicy::LocalOnly` — a dangling reference that **fails to compile** (`cargo build -p raki` → E0599). Task 2 fixes this as part of the AppState rewire. The DoD builds the app crate explicitly so this can't regress unseen again.

**Rollback:** Task 1 (library) and Tasks 2–3 (app) and 4–5 (frontend) are independent commits. If `cargo build -p raki` fails after Tasks 2+3, `git revert` those commits restores the prior (broken-but-known) app state; Task 1 is unaffected. If the manual walkthrough fails, inspect stderr for `UnconfiguredProvider` / gate-denial messages and the cloud env vars before reverting; a frontend-only issue reverts with Tasks 4–5 without touching Rust.

**Parallelism:** Task 1 (`raki-generate`) and Task 2 (`raki` app) touch disjoint crates with no ordering dependency — a subagent runner may do them concurrently. Tasks 3→4→5 are sequential (DTOs → bindings → frontend).

**Verified facts (read before starting):**
- **App command pattern** (`src-tauri/src/commands/notes.rs`): `#[tauri::command] pub async fn name(state: State<'_, AppState>, arg: T) -> Result<Dto, AppError>`. Args arrive by name. `NoteId::parse(&id)?` then `state.notes.get(&nid)` hydration.
- **AppState** (`src-tauri/src/state.rs`): `{ notes, keyword, vectors, embedder, clock, egress: EgressPolicy, index }`, `#[allow(dead_code)]`. The `egress` field is dead — replace it.
- **Composition root** (`src-tauri/src/lib.rs`): builds adapters from `Database::open`, `app.manage(AppState{…})`, `invoke_handler(tauri::generate_handler![create_note, list_notes, get_note, search_notes])`. Degrades `FastEmbedProvider`→`FakeEmbeddingProvider` on failure (mirror for the cloud provider).
- **AppError** (`src-tauri/src/error.rs`): `{ kind: String, message: String }`, `#[derive(Serialize, TS)]`, `From<DomainError>`. ts-rs export path `"../../src/shared/ipc/bindings/"`.
- **DTO/ts-rs** (`src-tauri/src/dto.rs`): `#[derive(Debug, Serialize, Deserialize, TS)] #[ts(export, export_to = "../../src/shared/ipc/bindings/")]`. Bindings regenerate when the crate's tests run. `ts-rs = "12.0"`. **For a discriminated union ts-rs requires `#[ts(tag = "...")]`** (struct variants only).
- **Cargo:** `src-tauri/Cargo.toml` has `async-trait` (line 53) but **not** `raki-generate`. `[workspace.dependencies]` lists the path deps but **not** `raki-generate`.
- **Frontend IPC** (`src/shared/ipc/index.ts`): one `commands` object wrapping `invoke<T>(...)`; components never call `invoke`. Per-module `api.ts`. Tests mock `~/shared/ipc` (`src/modules/notes/api.test.ts`). **AGENTS.md §5 (line 206): "Modules never import from each other."** Cross-module composition happens in the shell.
- **Shell** (`src/app/App.tsx`): `<QueryClientProvider><main class="container"><NotesView/></main></QueryClientProvider>`. The shell is the only place that composes modules → mount `<AskBox>` here.
- **Scripts:** `bun run typecheck` (`tsc --noEmit`), `bun run test` (`vitest run`), `bun run build` (`vite build`).
- **raki-generate (2a):** `answer_question(query, &GenerateDeps) -> Result<Answer, GenerateError>`; `GenerateDeps { keyword, vectors, embedder, notes, gate, provider, model, budget, k }`; `Answer { state: AnswerState, text, cited_ids: Vec<SourceId>, egress_log_id }`; `GenerateError { Egress(EgressError), Domain(DomainError) }`. `flow_tests` already defines `SpyLog` and `gate(fake, log)` (two-arg) — verified present; the preview tests reuse them.
- **raki-ai:** `MessagesProvider::from_env() -> Result<Self, DomainError>`, `GatedLlmProvider::new(inner, settings, log, clock)`. **raki-storage:** `SqliteEgressSettings::new(db)`, `SqliteEgressLog::new(db)`. **Gate two-factor:** egress requires BOTH `Mode::CloudAllowed` AND a provider grant (Slice 1 gate tests).

---

## File Structure

```
src-tauri/Cargo.toml            MODIFY  add raki-generate dep (workspace + app)
raki-generate/src/lib.rs        MODIFY  extract assemble_for; add EgressPreview + preview(); tests
src-tauri/src/state.rs          MODIFY  AppState: drop dead `egress`; add gate + settings + provider/model
src-tauri/src/lib.rs            MODIFY  wire egress stack + MessagesProvider→gate (FIXES build)
src-tauri/src/dto.rs            MODIFY  AnswerOutcome (#[ts(tag)]), CitedNote, EgressPreviewDto
src-tauri/src/commands/qa.rs    CREATE  answer_question, grant/revoke_cloud_consent; register handlers
src-tauri/src/commands/mod.rs   MODIFY  pub mod qa;
src/shared/ipc/index.ts         MODIFY  answerQuestion / grantCloudConsent / revokeCloudConsent
src/modules/qa/api.ts           CREATE  qaApi
src/modules/qa/api.test.ts      CREATE  delegation tests
src/modules/qa/AskBox.tsx       CREATE  the ask-box + preview→consent flow
src/modules/qa/AskBox.test.tsx  CREATE  state-machine tests
src/app/App.tsx                 MODIFY  opt-in toggle + mount <AskBox> (shell, not a module)
src/shared/ipc/bindings/*.ts    GENERATED by ts-rs (AnswerOutcome, CitedNote, EgressPreviewDto)
```

**Decision (opt-in setting):** the "Enable experimental retrieval diagnostics" opt-in (spec D7) is a **frontend `localStorage` flag** (`raki.qa.enabled`) in the shell, gating whether `<AskBox>` renders. Pure UX — the privacy guarantee is the backend gate, not the toggle — so no command/migration. The spec says "ask-box in `NotesView`"; AGENTS.md §5 overrides that placement (modules can't import each other), so it lives in the shell beside `<NotesView>`.

---

## Task 1: `raki-generate` — `preview()` (CI-tested, no app)

**Files:** Modify `src-tauri/crates/raki-generate/src/lib.rs`.

- [ ] **Step 1: Extract `assemble_for`; refactor `answer_question` (behavior-preserving)**

Add a private helper that does retrieve + assemble (no send), returning the context plus an id→title map. Then rewrite `answer_question`'s retrieve/assemble block to call it (the rest unchanged):

```rust
/// Retrieve + assemble locally (no model call). Returns the assembled context and an
/// id→title map for the included sources, or `None` when nothing matched.
async fn assemble_for(
    query: &str,
    deps: &GenerateDeps<'_>,
) -> Result<Option<(AssembledContext, std::collections::HashMap<String, String>)>, GenerateError> {
    let ids = hybrid_search(deps.keyword, deps.vectors, deps.embedder, query, deps.k)
        .await
        .map_err(GenerateError::Domain)?;

    let mut candidates = Vec::new();
    let mut titles = std::collections::HashMap::new();
    for (rank, id) in ids.iter().enumerate() {
        let nid = match NoteId::parse(id) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("skipping malformed source id {id}: {e}");
                continue;
            }
        };
        if let Some(note) = deps.notes.get(&nid).await.map_err(GenerateError::Domain)? {
            titles.insert(id.clone(), note.title.clone());
            candidates.push(Candidate {
                source_id: id.clone(),
                text: format!("{}\n{}", note.title, note_body_to_text(&note.body)),
                score: (ids.len() - rank) as f64,
            });
        }
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    let ctx = assemble_context(&candidates, deps.budget, deps.provider, deps.model);
    Ok(Some((ctx, titles)))
}
```

Rewrite `answer_question` to reuse it (replace its retrieve/assemble block):

```rust
pub async fn answer_question(
    query: &str,
    deps: &GenerateDeps<'_>,
) -> Result<Answer, GenerateError> {
    let Some((ctx, _titles)) = assemble_for(query, deps).await? else {
        return Ok(Answer {
            state: AnswerState::NothingMatched,
            text: "No relevant notes found.".into(),
            cited_ids: vec![],
            egress_log_id: None,
        });
    };
    let req = CompletionRequest {
        system: Some(build_system_prompt(&ctx)),
        prompt: query.to_string(),
        max_tokens: None,
    };
    let (completion, log_id) = deps
        .gate
        .complete_gated(&ctx.egress, req)
        .await
        .map_err(GenerateError::Egress)?;
    let context_ids: std::collections::HashSet<String> =
        ctx.egress.source_ids.iter().map(|s| s.0.clone()).collect();
    let (state, text, cited_ids) = evaluate(&completion.text, &context_ids);
    deps.gate
        .set_grounded(&log_id, state.is_grounded())
        .await
        .map_err(GenerateError::Domain)?;
    Ok(Answer { state, text, cited_ids, egress_log_id: Some(log_id) })
}
```

- [ ] **Step 2: Verify the refactor preserved behavior**

Run: `cd src-tauri && cargo test -p raki-generate`
Expected: PASS — all existing flow/groundedness tests still green (no new code path yet).

- [ ] **Step 3: Write the failing `preview` tests**

In `flow_tests` (reuses the existing `SpyLog` + two-arg `gate` — verified present):

```rust
    #[tokio::test]
    async fn preview_returns_egress_metadata_without_sending() {
        let nid = NoteId::new();
        let fake = Arc::new(FakeLlmProvider::ok("unused"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake.clone(), log.clone());
        let deps = GenerateDeps {
            keyword: &NoKeyword, vectors: &OneVector(nid.to_string()), embedder: &FakeEmbed,
            notes: &OneNote(nid), gate: &g, provider: "kimi", model: "k2", budget: 10_000, k: 5,
        };
        let p = preview("how do I pay?", &deps).await.unwrap().expect("some preview");
        assert_eq!(p.provider, "kimi");
        assert_eq!(p.source_titles, vec!["Trip".to_string()]);
        assert!(p.summary.contains("→ kimi/k2"));
        assert_eq!(fake.call_count(), 0, "preview never sends");
    }

    #[tokio::test]
    async fn preview_is_none_when_nothing_matched() {
        let nid = NoteId::new();
        let fake = Arc::new(FakeLlmProvider::ok("unused"));
        let log = Arc::new(SpyLog::default());
        let g = gate(fake, log);
        let deps = GenerateDeps {
            keyword: &NoKeyword, vectors: &OneVector(nid.to_string()), embedder: &FakeEmbed,
            notes: &EmptyRepo, gate: &g, provider: "kimi", model: "k2", budget: 10_000, k: 5,
        };
        assert!(preview("x", &deps).await.unwrap().is_none());
    }
```

Run: `cd src-tauri && cargo build -p raki-generate --tests`
Expected: FAIL — `preview` / `EgressPreview` undefined.

- [ ] **Step 4: Add `EgressPreview` + `preview()`**

```rust
/// What a cloud send WOULD disclose — shown to the user before consent (spec D7). Metadata only.
pub struct EgressPreview {
    pub provider: String,
    pub summary: String,
    pub source_titles: Vec<String>,
}

/// The egress preview for `query` (no send), or `None` if nothing matched.
pub async fn preview(
    query: &str,
    deps: &GenerateDeps<'_>,
) -> Result<Option<EgressPreview>, GenerateError> {
    let Some((ctx, titles)) = assemble_for(query, deps).await? else {
        return Ok(None);
    };
    let source_titles = ctx
        .egress
        .source_ids
        .iter()
        .map(|s| titles.get(&s.0).cloned().unwrap_or_else(|| s.0.clone()))
        .collect();
    Ok(Some(EgressPreview {
        provider: deps.provider.to_string(),
        summary: ctx.egress.summary(),
        source_titles,
    }))
}
```

- [ ] **Step 5: Verify + commit**

Run: `cd src-tauri && cargo test -p raki-generate`
Expected: PASS — the 2 new preview tests + all prior tests.

```bash
git add src-tauri/crates/raki-generate/src/lib.rs
git commit -m "raki-generate: add preview() (egress metadata, no send); share assemble helper"
```

---

## Task 2: Repair + rewire `AppState` (makes `raki` compile again)

**Files:** Modify `src-tauri/Cargo.toml`, `src-tauri/src/state.rs`, `src-tauri/src/lib.rs`.

- [ ] **Step 1: Confirm the breakage (red) + add the missing dep**

Run: `cd src-tauri && cargo build -p raki`
Expected: FAIL — `error[E0599]: no associated item named LocalOnly found for struct EgressPolicy` at `lib.rs:74`.

In `src-tauri/Cargo.toml`, add to `[workspace.dependencies]` (after `raki-memory`): `raki-generate = { path = "crates/raki-generate" }`. And to the app `[dependencies]` (after `raki-memory = { workspace = true }`): `raki-generate = { workspace = true }`.

- [ ] **Step 2: Rewire `AppState` (`state.rs`)**

```rust
//! Application state: the injected ports the command layer delegates to.

use std::sync::Arc;

use raki_ai::GatedLlmProvider;
use raki_domain::{Clock, EgressSettings, EmbeddingProvider, KeywordIndex, NoteRepository, VectorIndex};

use crate::indexing::IndexingService;

pub struct AppState {
    pub notes: Arc<dyn NoteRepository>,
    pub keyword: Arc<dyn KeywordIndex>,
    pub vectors: Arc<dyn VectorIndex>,
    pub embedder: Arc<dyn EmbeddingProvider>,
    pub clock: Arc<dyn Clock>,
    pub index: Arc<IndexingService>,
    /// The only cloud-completion path (wraps MessagesProvider; reads consent live; logs egress).
    pub gate: Arc<GatedLlmProvider>,
    /// Consent + mode mutation surface for the consent commands.
    pub settings: Arc<dyn EgressSettings>,
    /// The cloud provider/model the egress decision is attributed to (display + consent key).
    pub provider: String,
    pub model: String,
}
```

(Remove `#[allow(dead_code)]` — every field is now used.)

- [ ] **Step 3: Wire the egress stack (`lib.rs`)**

Update imports:

```rust
use raki_ai::{FakeEmbeddingProvider, FastEmbedProvider, GatedLlmProvider, MessagesProvider};
use raki_domain::{
    Clock, Completion, CompletionRequest, DomainError, EmbeddingProvider, IndexingStore,
    LlmProvider, Locality, VectorIndex,
};
use raki_storage::{
    Database, SqliteEgressLog, SqliteEgressSettings, SqliteIndexingStore, SqliteKeywordIndex,
    SqliteNoteRepository, SqliteVectorIndex,
};
```

(Remove the old `use raki_ai::EgressPolicy;`.) Add the fallback provider near `SystemClock` (`async-trait` is already an app dep):

```rust
/// Used when no cloud model is configured. Never sends; fails clearly if a gated call reaches it
/// (only possible after the user grants consent, so the message is actionable).
struct UnconfiguredProvider;
#[async_trait::async_trait]
impl LlmProvider for UnconfiguredProvider {
    fn locality(&self) -> Locality {
        Locality::Cloud
    }
    async fn complete(&self, _req: CompletionRequest) -> Result<Completion, DomainError> {
        Err(DomainError::Provider(
            "no cloud model configured (set RAKI_LLM_BASE_URL / ANTHROPIC_API_KEY / RAKI_LLM_MODEL)".into(),
        ))
    }
}
```

Replace the `app.manage(AppState { … egress: EgressPolicy::LocalOnly … })` block with:

```rust
            let settings: Arc<dyn raki_domain::EgressSettings> =
                Arc::new(SqliteEgressSettings::new(db.clone()));
            let egress_log: Arc<dyn raki_domain::EgressLog> =
                Arc::new(SqliteEgressLog::new(db.clone()));

            let provider = "kimi".to_string();
            let model = std::env::var("RAKI_LLM_MODEL").unwrap_or_else(|_| "unknown".to_string());
            let inner: Arc<dyn LlmProvider> = match MessagesProvider::from_env() {
                Ok(p) => Arc::new(p),
                Err(e) => {
                    eprintln!("cloud model unavailable ({e}); QA will error until configured");
                    Arc::new(UnconfiguredProvider)
                }
            };
            let clock: Arc<dyn Clock> = Arc::new(SystemClock);
            let gate = Arc::new(GatedLlmProvider::new(inner, settings.clone(), egress_log, clock.clone()));

            app.manage(AppState {
                notes, keyword, vectors, embedder, clock, index,
                gate, settings, provider, model,
            });
```

- [ ] **Step 4: Build the app crate (handlers still the original four)**

Do **not** touch `invoke_handler` yet (the QA commands don't exist until Task 3 — registering them now would not compile, review #7).

Run: `cd src-tauri && cargo build -p raki && cargo clippy -p raki -- -D warnings`
Expected: PASS — the E0599 is gone; the egress stack compiles; the four existing commands still register.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/state.rs src-tauri/src/lib.rs
git commit -m "App: wire egress stack into AppState; fix EgressPolicy drift (app compiles again)"
```

---

## Task 3: DTOs + the three commands + handler registration

**Files:** Modify `src-tauri/src/dto.rs`, `src-tauri/src/commands/mod.rs`, `src-tauri/src/lib.rs`; Create `src-tauri/src/commands/qa.rs`.

- [ ] **Step 1: Add the DTOs (`dto.rs`)**

`AnswerOutcome` needs **`#[ts(tag = "kind")]`** for ts-rs to emit a discriminated union (struct variants only — these qualify). `#[serde(tag = "kind")]` drives the runtime JSON; both are required.

```rust
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct CitedNote {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct EgressPreviewDto {
    pub provider: String,
    pub summary: String,
    pub source_titles: Vec<String>,
}

/// Either we need consent (and show what would leave), or we have an answer.
/// Tagged union so the frontend can pattern-match on `kind`.
#[derive(Debug, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export, tag = "kind", rename_all = "snake_case", export_to = "../../src/shared/ipc/bindings/")]
pub enum AnswerOutcome {
    NeedsConsent { preview: EgressPreviewDto },
    Answer { state: String, text: String, cited: Vec<CitedNote> },
}
```

- [ ] **Step 2: Create `commands/qa.rs`** (one file, three labeled blocks — splitting into separate commits would leave unused-fn warnings under `-D warnings`)

```rust
//! Grounded-QA command adapters: translate + delegate to `raki-generate`. No business logic.

use tauri::State;

use raki_domain::{EgressDenied, EgressError, Mode, NoteId};
use raki_generate::{answer_question as run_answer, preview, GenerateDeps, GenerateError};

use crate::dto::{AnswerOutcome, CitedNote, EgressPreviewDto};
use crate::error::AppError;
use crate::state::AppState;

// ---- block A: shared helpers ----
const K: usize = 10;
const BUDGET_TOKENS: usize = 2000;

fn deps(state: &AppState) -> GenerateDeps<'_> {
    GenerateDeps {
        keyword: state.keyword.as_ref(),
        vectors: state.vectors.as_ref(),
        embedder: state.embedder.as_ref(),
        notes: state.notes.as_ref(),
        gate: state.gate.as_ref(),
        provider: &state.provider,
        model: &state.model,
        budget: BUDGET_TOKENS,
        k: K,
    }
}

/// A `Denied(LocalOnlyMode | ConsentRequired)` is NOT an error — it means "ask the user first".
/// Sync: a match guard cannot `.await`.
fn needs_consent(e: &GenerateError) -> bool {
    matches!(
        e,
        GenerateError::Egress(EgressError::Denied(
            EgressDenied::LocalOnlyMode | EgressDenied::ConsentRequired
        ))
    )
}

fn into_app_error(e: GenerateError) -> AppError {
    match e {
        GenerateError::Domain(d) => AppError::from(d),
        GenerateError::Egress(EgressError::Completion(d)) => AppError::from(d),
        GenerateError::Egress(EgressError::Audit(m)) => AppError { kind: "audit".into(), message: m },
        GenerateError::Egress(EgressError::Denied(d)) => AppError { kind: "denied".into(), message: d.to_string() },
    }
}

// ---- block B: the answer command ----
#[tauri::command]
pub async fn answer_question(
    state: State<'_, AppState>,
    query: String,
) -> Result<AnswerOutcome, AppError> {
    match run_answer(&query, &deps(state.inner())).await {
        Ok(ans) => {
            let mut cited = Vec::with_capacity(ans.cited_ids.len());
            for sid in &ans.cited_ids {
                let title = match NoteId::parse(&sid.0) {
                    Ok(nid) => state.notes.get(&nid).await?.map(|n| n.title).unwrap_or_else(|| sid.0.clone()),
                    Err(_) => sid.0.clone(),
                };
                cited.push(CitedNote { id: sid.0.clone(), title });
            }
            Ok(AnswerOutcome::Answer { state: ans.state.name().to_string(), text: ans.text, cited })
        }
        Err(e) if needs_consent(&e) => {
            // Re-run retrieve+assemble locally (no send) to show what WOULD leave.
            match preview(&query, &deps(state.inner())).await {
                Ok(Some(p)) => Ok(AnswerOutcome::NeedsConsent {
                    preview: EgressPreviewDto {
                        provider: p.provider,
                        summary: p.summary,
                        source_titles: p.source_titles,
                    },
                }),
                Ok(None) => Ok(AnswerOutcome::Answer {
                    state: "nothing_matched".into(),
                    text: "No relevant notes found.".into(),
                    cited: vec![],
                }),
                Err(pe) => Err(into_app_error(pe)),
            }
        }
        Err(e) => Err(into_app_error(e)),
    }
}

// ---- block C: consent mutation commands ----
#[tauri::command]
pub async fn grant_cloud_consent(state: State<'_, AppState>, provider: String) -> Result<(), AppError> {
    state.settings.set_mode(Mode::CloudAllowed).await?;
    state.settings.grant(&provider).await?;
    Ok(())
}

/// Revoking the provider is sufficient to block egress: `GatedLlmProvider` requires BOTH
/// `CloudAllowed` mode AND a provider-specific grant, so an empty consent set denies all sends
/// even though mode stays `CloudAllowed` (review #6).
#[tauri::command]
pub async fn revoke_cloud_consent(state: State<'_, AppState>, provider: String) -> Result<(), AppError> {
    state.settings.revoke(&provider).await?;
    Ok(())
}
```

- [ ] **Step 3: Register the module + the handlers**

In `src-tauri/src/commands/mod.rs`, add `pub mod qa;`. In `src-tauri/src/lib.rs`, import and register:

```rust
use crate::commands::qa::{answer_question, grant_cloud_consent, revoke_cloud_consent};
```
```rust
        .invoke_handler(tauri::generate_handler![
            create_note, list_notes, get_note, search_notes,
            answer_question, grant_cloud_consent, revoke_cloud_consent
        ])
```

- [ ] **Step 4: Build, generate bindings, inspect them**

Run: `cd src-tauri && cargo build -p raki && cargo test -p raki 2>&1 | tail -5 && cargo clippy -p raki -- -D warnings`
Then inspect the emitted union before staging (review #13):
Run (repo root): `git --no-pager diff --stat src/shared/ipc/bindings/ && cat src/shared/ipc/bindings/AnswerOutcome.ts`
Expected: `AnswerOutcome.ts` is a discriminated union — `{ "kind": "needs_consent", preview: EgressPreviewDto } | { "kind": "answer", state: string, text: string, cited: Array<CitedNote> }`. If the `kind` tag or snake_case variant names are missing, the `#[ts(tag/rename_all)]` attributes weren't applied — fix before proceeding.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src src/shared/ipc/bindings
git commit -m "App: AnswerOutcome DTO + QA/consent commands; register handlers"
```

---

## Task 4: Frontend IPC + qa api + test

**Files:** Modify `src/shared/ipc/index.ts`; Create `src/modules/qa/api.ts`, `src/modules/qa/api.test.ts`.

- [ ] **Step 1: Extend the typed command surface (`src/shared/ipc/index.ts`)**

```ts
import { invoke } from "@tauri-apps/api/core";
import type { NoteDto } from "./bindings/NoteDto";
import type { CreateNoteInput } from "./bindings/CreateNoteInput";
import type { AnswerOutcome } from "./bindings/AnswerOutcome";

export type { NoteDto, CreateNoteInput, AnswerOutcome };

export const commands = {
  createNote: (input: CreateNoteInput) => invoke<NoteDto>("create_note", { input }),
  listNotes: () => invoke<NoteDto[]>("list_notes"),
  getNote: (id: string) => invoke<NoteDto | null>("get_note", { id }),
  searchNotes: (query: string) => invoke<NoteDto[]>("search_notes", { query }),
  answerQuestion: (query: string) => invoke<AnswerOutcome>("answer_question", { query }),
  grantCloudConsent: (provider: string) => invoke<null>("grant_cloud_consent", { provider }),
  revokeCloudConsent: (provider: string) => invoke<null>("revoke_cloud_consent", { provider }),
};
```

- [ ] **Step 2: Create `src/modules/qa/api.ts`**

```ts
import { commands, type AnswerOutcome } from "~/shared/ipc";

export type { AnswerOutcome };

export const qaApi = {
  ask: (query: string) => commands.answerQuestion(query),
  grant: (provider: string) => commands.grantCloudConsent(provider),
  revoke: (provider: string) => commands.revokeCloudConsent(provider),
};
```

- [ ] **Step 3: Failing test `src/modules/qa/api.test.ts`**

```ts
import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("~/shared/ipc", () => ({
  commands: { answerQuestion: vi.fn(), grantCloudConsent: vi.fn(), revokeCloudConsent: vi.fn() },
}));

import { commands } from "~/shared/ipc";
import { qaApi } from "./api";

const mocked = vi.mocked(commands);

describe("qaApi", () => {
  beforeEach(() => vi.clearAllMocks());

  it("ask delegates to answerQuestion with the query", async () => {
    mocked.answerQuestion.mockResolvedValue({ kind: "answer", state: "grounded", text: "x", cited: [] });
    await qaApi.ask("why is the sky blue?");
    expect(mocked.answerQuestion).toHaveBeenCalledWith("why is the sky blue?");
  });

  it("grant delegates to grantCloudConsent with the provider", async () => {
    mocked.grantCloudConsent.mockResolvedValue(null);
    await qaApi.grant("kimi");
    expect(mocked.grantCloudConsent).toHaveBeenCalledWith("kimi");
  });
});
```

- [ ] **Step 4: Verify + commit**

Run (repo root): `bun run test && bun run typecheck`
Expected: PASS — qa tests green; `tsc` resolves the `AnswerOutcome` union.

```bash
git add src/shared/ipc/index.ts src/modules/qa/api.ts src/modules/qa/api.test.ts
git commit -m "Frontend: typed QA + consent commands and qaApi"
```

---

## Task 5: `<AskBox>` + shell mount + component tests

**Files:** Create `src/modules/qa/AskBox.tsx`, `src/modules/qa/AskBox.test.tsx`; Modify `src/app/App.tsx`.

- [ ] **Step 1: Create `src/modules/qa/AskBox.tsx`**

```tsx
import { createSignal, Show, For } from "solid-js";
import { qaApi, type AnswerOutcome } from "./api";

const PROVIDER = "kimi";

function errMessage(e: unknown): string {
  return typeof e === "object" && e && "message" in e ? String((e as { message: unknown }).message) : String(e);
}

export function AskBox() {
  const [question, setQuestion] = createSignal("");
  const [outcome, setOutcome] = createSignal<AnswerOutcome | null>(null);
  const [pending, setPending] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  async function run(fn: () => Promise<AnswerOutcome>) {
    setPending(true);
    setError(null);
    try {
      setOutcome(await fn());
    } catch (e) {
      setError(errMessage(e));
    } finally {
      setPending(false);
    }
  }

  const ask = () => {
    const q = question().trim();
    if (q) run(() => qaApi.ask(q));
  };

  const confirmSend = () =>
    run(async () => {
      await qaApi.grant(PROVIDER); // grant consent + flip to CloudAllowed, then re-ask
      return qaApi.ask(question().trim());
    });

  return (
    <section aria-label="Ask AI (experimental)">
      <h2>Ask your notes (experimental)</h2>
      <form onSubmit={(e) => { e.preventDefault(); ask(); }}>
        <input
          placeholder="Ask a question about your notes…"
          value={question()}
          onInput={(e) => setQuestion(e.currentTarget.value)}
        />
        <button type="submit" disabled={pending()}>Ask</button>
      </form>

      <Show when={error()}>{(msg) => <p role="alert">Error: {msg()}</p>}</Show>

      <Show when={outcome()}>
        {(o) => (
          <Show
            when={o().kind === "needs_consent" ? (o() as Extract<AnswerOutcome, { kind: "needs_consent" }>) : null}
            fallback={
              <div>
                <p>{(o() as Extract<AnswerOutcome, { kind: "answer" }>).text}</p>
                <Show when={(o() as Extract<AnswerOutcome, { kind: "answer" }>).cited.length > 0}>
                  <p>Sources:</p>
                  <ul>
                    <For each={(o() as Extract<AnswerOutcome, { kind: "answer" }>).cited}>
                      {(c) => <li>{c.title}</li>}
                    </For>
                  </ul>
                </Show>
              </div>
            }
          >
            {(nc) => (
              <div>
                <p>This will send to the cloud: <strong>{nc().preview.summary}</strong></p>
                <ul>
                  <For each={nc().preview.source_titles}>{(t) => <li>{t}</li>}</For>
                </ul>
                <button type="button" disabled={pending()} onClick={confirmSend}>Send to cloud</button>
                <button type="button" onClick={() => setOutcome(null)}>Stay local</button>
              </div>
            )}
          </Show>
        )}
      </Show>
    </section>
  );
}
```

- [ ] **Step 2: Component tests `src/modules/qa/AskBox.test.tsx`** (review #9)

```tsx
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, fireEvent, screen, waitFor } from "@solidjs/testing-library";

vi.mock("./api", () => ({ qaApi: { ask: vi.fn(), grant: vi.fn(), revoke: vi.fn() } }));

import { qaApi } from "./api";
import { AskBox } from "./AskBox";

const mocked = vi.mocked(qaApi);

describe("AskBox", () => {
  beforeEach(() => vi.clearAllMocks());

  it("asking renders a consent preview without sending", async () => {
    mocked.ask.mockResolvedValue({
      kind: "needs_consent",
      preview: { provider: "kimi", summary: "1 sources, 10 tokens → kimi/k2", source_titles: ["Trip"] },
    });
    render(() => <AskBox />);
    fireEvent.input(screen.getByPlaceholderText(/Ask a question/i), { target: { value: "how do I pay?" } });
    fireEvent.submit(screen.getByRole("button", { name: "Ask" }).closest("form")!);
    await waitFor(() => screen.getByText(/This will send to the cloud/i));
    expect(screen.getByText("Trip")).toBeInTheDocument();
    expect(mocked.grant).not.toHaveBeenCalled();
  });

  it("confirming sends: grants then re-asks, then shows the answer", async () => {
    mocked.ask
      .mockResolvedValueOnce({
        kind: "needs_consent",
        preview: { provider: "kimi", summary: "s", source_titles: ["Trip"] },
      })
      .mockResolvedValueOnce({ kind: "answer", state: "grounded", text: "Pay cash.", cited: [{ id: "n1", title: "Trip" }] });
    mocked.grant.mockResolvedValue(null);
    render(() => <AskBox />);
    fireEvent.input(screen.getByPlaceholderText(/Ask a question/i), { target: { value: "pay?" } });
    fireEvent.submit(screen.getByRole("button", { name: "Ask" }).closest("form")!);
    await waitFor(() => screen.getByRole("button", { name: "Send to cloud" }));
    fireEvent.click(screen.getByRole("button", { name: "Send to cloud" }));
    await waitFor(() => screen.getByText("Pay cash."));
    expect(mocked.grant).toHaveBeenCalledWith("kimi");
  });

  it("renders an error alert when ask rejects", async () => {
    mocked.ask.mockRejectedValue({ kind: "provider", message: "boom" });
    render(() => <AskBox />);
    fireEvent.input(screen.getByPlaceholderText(/Ask a question/i), { target: { value: "x" } });
    fireEvent.submit(screen.getByRole("button", { name: "Ask" }).closest("form")!);
    await waitFor(() => screen.getByRole("alert"));
    expect(screen.getByRole("alert")).toHaveTextContent("boom");
  });
});
```

- [ ] **Step 3: Mount in the shell (`src/app/App.tsx`) — NOT in a module (AGENTS.md §5)**

```tsx
import { createSignal, Show } from "solid-js";
import { QueryClient, QueryClientProvider } from "@tanstack/solid-query";
import { NotesView } from "~/modules/notes/NotesView";
import { AskBox } from "~/modules/qa/AskBox";

const queryClient = new QueryClient();

export function App() {
  const [qaEnabled, setQaEnabled] = createSignal(localStorage.getItem("raki.qa.enabled") === "1");
  function toggleQa(on: boolean) {
    setQaEnabled(on);
    localStorage.setItem("raki.qa.enabled", on ? "1" : "0");
  }

  return (
    <QueryClientProvider client={queryClient}>
      <main class="container">
        <label>
          <input type="checkbox" checked={qaEnabled()} onChange={(e) => toggleQa(e.currentTarget.checked)} />
          Enable experimental retrieval diagnostics
        </label>
        <Show when={qaEnabled()}>
          <AskBox />
        </Show>
        <NotesView />
      </main>
    </QueryClientProvider>
  );
}
```

- [ ] **Step 4: Verify + commit**

Run (repo root): `bun run typecheck && bun run test && bun run build`
Expected: PASS — types resolve (the `Extract<…>` narrowing compiles), AskBox tests green, vite build clean.

```bash
git add src/modules/qa/AskBox.tsx src/modules/qa/AskBox.test.tsx src/app/App.tsx
git commit -m "Frontend: experimental ask-box (preview→consent→answer), mounted in the shell"
```

---

## Task 6: Verification + Definition of Done

- [ ] **Step 1: Library + app build clean (app is back in CI scope)**

Run: `cd src-tauri && cargo test --workspace --exclude raki && cargo build -p raki && cargo clippy --workspace -- -D warnings && cargo fmt --check`
Expected: all pass — clippy now runs **without** `--exclude raki`.

- [ ] **Step 2: Frontend checks**

Run (repo root): `bun run typecheck && bun run test && bun run build`
Expected: all green.

- [ ] **Step 3: Bindings present**

Run (repo root): `ls src/shared/ipc/bindings/`
Expected: `AnswerOutcome.ts`, `CitedNote.ts`, `EgressPreviewDto.ts` present.

- [ ] **Step 4: MANUAL `tauri dev` walkthrough (REQUIRED — not claimable from a test run)**

This slice touches the CI-excluded app + frontend. Per `verification-before-completion`, completion is **not** claimed until the user confirms this in the running app. Configure cloud env (`ckimi` / `RAKI_LLM_BASE_URL` + `ANTHROPIC_API_KEY` + `RAKI_LLM_MODEL`), then `bun run tauri dev`:

*QA flow:*
  1. Tick "Enable experimental retrieval diagnostics" → the ask-box appears.
  2. Ask a question answerable from a note → a **consent preview** appears ("…N sources, T tokens → kimi/<model>" + titles). **Nothing sent yet.**
  3. "Send to cloud" → a grounded answer renders with cited source notes.
  4. Ask again → sends directly (consent persisted), no second preview.
  5. Ask with no relevant notes → "No relevant notes found.", no egress.

*Notes regression (the AppState rewire must not have broken existing commands — review #10):*
  6. Create a note → it appears in the list.
  7. Search for it → it ranks.
  8. (Existing search/list/get all still function.)

**STOP. Do not mark this slice complete until the user reports the walkthrough passed.**

- [ ] **Step 5: DoD against the spec**

D5 (`answer_question` command, typed outcome) ✓ Task 3 · D6 (`AnswerOutcome` tagged union via `#[ts(tag)]`; side-effect-free answer command) ✓ Task 3 · D7 (`grant_cloud_consent`/`revoke_cloud_consent` separate commands; opt-in ask-box; preview→consent→re-submit, stateless) ✓ Tasks 3,5 · M4 (local embedder; preview makes no network call) ✓ Task 1. App-crate drift repaired ✓ Task 2.

---

## Self-Review

**Spec coverage:** D5→T3, D6→T3 (`AnswerOutcome` via ts-rs `#[ts(tag)]` union), D7→T3+T5 (consent commands + shell ask-box + stateless re-submit). Preview logic is CI-tested in `raki-generate` (T1), not stranded in the untested app. Opt-in is a documented shell `localStorage` decision.

**Placeholder scan:** none — every step has complete code or an exact command. The `qa.rs` "three labeled blocks, one commit" note is an explicit rationale (unused-fn warnings under `-D warnings`), not deferred work.

**Type/consistency:** `GenerateDeps`/`preview`/`answer_question`/`Answer`/`AnswerState::name()` (2a) used as-is. `AnswerOutcome::{NeedsConsent{preview}, Answer{state,text,cited}}` with `#[serde(tag="kind", rename_all="snake_case")]` + `#[ts(tag="kind", rename_all="snake_case")]` ↔ frontend `Extract<AnswerOutcome,{kind:"needs_consent"|"answer"}>`. `deps(state.inner()) -> GenerateDeps` (avoids the `&State` vs `&AppState` ambiguity). `needs_consent` is sync (no `.await` in a guard). `EgressError::{Completion,Audit,Denied}`, `EgressDenied::{LocalOnlyMode,ConsentRequired}`, `Mode::CloudAllowed` match Slice 1/2a. Handlers registered only in Task 3 (after the fns exist). `<AskBox>` mounted in `App.tsx` (shell), not imported by a module.

**Known confirmations (read-and-match at implementation time):** the emitted `AnswerOutcome.ts` shape (Task 3 Step 4 inspects it — if ts-rs 12's serde-compat already applies `rename_all`, the explicit `#[ts(rename_all)]` is redundant but harmless); the exact preserved lines of `lib.rs`'s `setup` closure (only the `AppState{…}` literal + provider wiring change); `@solidjs/testing-library` exports `render/fireEvent/screen/waitFor` (it does — used per its standard API).

---

## Execution Handoff

(Presented to the user after saving.)
