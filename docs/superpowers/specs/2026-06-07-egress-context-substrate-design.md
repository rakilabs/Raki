# Egress + Context-Assembly Substrate — Design

Date: 2026-06-07

Status: **Approved after three adversarial reviews (2026-06-07). Slice 1 of 2.** Split from the
grounded-Q&A design once it was established that a cloud completion is forbidden without an approved,
logged `EgressDecision` through `raki-ai` (`AGENTS.md` 486–488) and that the enabling substrate did
not exist. This slice builds **only** that substrate — fake-tested, **no model adapter, no command,
no UI**. The grounded-Q&A feature is **Slice 2** (previewed at the end). The design below incorporates
the review corrections (gate takes `&EgressDecision`; live consent; post-call logging; `SourceId`
newtype; domain `EgressLogId`; empty-egress guard; metadata-only, format-stable log).

## Premise

`AGENTS.md` mandates that every model call go through `raki-ai`'s egress policy and carry an
`AssembledContext` whose `egress: EgressDecision` was approved and logged ("what left my device?",
line 41). Today there is no `egress` field, no policy/consent/gate, and no log. Building generation on
that void either violates the policy or smuggles the substrate in as feature code. So the substrate
ships first, exercised end-to-end with a `FakeLlmProvider`; every future model call inherits an
enforced privacy floor instead of reinventing one.

## What this is

- `EgressDecision` + `SourceId` + `EgressLogId` + the `EgressLog`/`EgressSettings` ports in
  **`raki-domain`** (the dependency-correct home).
- `AssembledContext` in `raki-memory` gains a `domain::EgressDecision`.
- A locality-aware private `approve()` + a `GatedLlmProvider` wrapper in `raki-ai` — the **only** way
  to obtain a completion, and the **only** thing the app layer is handed.
- An `egress_log` + `cloud_consent` in `raki-storage` (migration V4), behind the domain ports.
- Exhaustive tests with fakes. **No** `MessagesProvider`, command, or frontend.

## What this is NOT

- **Not a model call.** `GatedLlmProvider` is driven by a `FakeLlmProvider`; the real adapter is Slice 2.
- **Not the consent UI.** The persisted settings *store* and the live-reading policy are built here; the
  disclosure screen + `grant_cloud_consent` command are Slice 2.
- **Not the generate logic** (`build_prompt`/`parse_answer`/`groundedness`) — Slice 2.
- **Not rate-limiting.** The gate is the chokepoint where backpressure will later live; it is a named
  future enhancement, not built now.
- **Not the memory lifecycle**, and **not** the full async `assemble_context(req, deps)` signature.

## Decisions

- **D1 — Contracts live in `raki-domain` (`crates/raki-domain/src/egress.rs`).** The leaf crates
  `raki-ai` and `raki-memory` are blind to each other, so every shared type goes in the kernel:
  - `SourceId(String)` — an opaque, block-ready id newtype (note id today, block id when chunking ships;
    consistent with the existing `NoteId` newtype). Prevents mixing ids with `provider`/`model`.
  - `EgressDecision { provider: String, model: String, source_ids: Vec<SourceId>, total_tokens: usize }`
    — *what would leave the device*, **metadata only** (no note text, no keys). A `summary()` **method**
    derives "N notes, T tokens" for display; it is **not** a stored field (no format-migration risk).
  - `EgressLogId(Uuid)` — UUID v7 (ADR-0002), so storage-generated ids never cross the trait as `i64`.
  - `EgressRecord { id: EgressLogId, decision: EgressDecision, completed_at: i64, success: bool }` — the
    *persisted* form: the decision **plus** the outcome.
  - `trait EgressLog { async fn record(&self, rec: &EgressRecord) -> Result<(), DomainError>; }` and
    `trait EgressSettings { async fn consented(&self) -> Result<HashSet<String>, DomainError>; async fn
    grant(provider); async fn revoke(provider); }`.

- **D2 — `AssembledContext` carries the egress decision; assembly stays pure.** Add `pub egress:
  EgressDecision` to the struct in `raki-memory/src/context.rs` (legal: `raki-memory → raki-domain`). A
  pure `egress_of(items, provider, model) -> EgressDecision` derives it deterministically from the
  included items (their `SourceId`s and summed `token_estimate`). The budgeted assembler is otherwise
  unchanged. **Empty assembly is representable** (zero items) and is rejected at the gate (D4), not here.

- **D3 — `GatedLlmProvider` policy: provider locality + per-provider consent.** The gate reads the
  inner provider's `Locality` at call time and the live `consented` set from `EgressSettings`.
  `approve(decision, locality, consented) -> Result<(), EgressDenied>` is a **private** method:
  empty `source_ids` ⇒ `EmptyContext`; `Locality::Local` ⇒ always `Ok` (nothing leaves the device,
  no audit row); `Locality::Cloud` + provider not in `consented` ⇒ `ConsentRequired`; else `Ok`.
  No global mode exists; the provider itself declares whether it runs locally.

- **D4 — `GatedLlmProvider` makes an un-gated call unrepresentable, reads consent live, logs the
  truth.** `GatedLlmProvider { inner: Arc<dyn LlmProvider>, settings: Arc<dyn EgressSettings>, log:
  Arc<dyn EgressLog> }`. Its sole completion method:
  `complete_gated(&self, egress: &EgressDecision, req: CompletionRequest) -> Result<Completion,
  EgressError>` — it takes the **`EgressDecision`** (never `AssembledContext`; the gate is blind to
  context internals), so the caller passes `ctx.egress`. Flow:
  1. read `inner.locality()` and `settings.consented()` (**live, every call**);
  2. `approve(egress, locality, &consented)?` — on denial, return `EgressError` with **no send and no log row**;
  3. `let result = inner.complete(req).await;`
  4. `log.record(EgressRecord { id: EgressLogId::new(), decision: egress.clone(), completed_at: now,
     success: result.is_ok() })` — logged **after** the call, recording what **did** (or did not) leave;
  5. return `result`.
  The raw `dyn LlmProvider` is constructed only inside `raki-ai` and the app holds a `GatedLlmProvider`,
  so "call the model without the gate" does not type-check outside `raki-ai`. (`CompletionRequest` is
  unchanged in this slice; Slice 2 adds `system` + `max_tokens`.)

- **D5 — Persisted log + settings (storage V4), audit-table shaped (ADR-0002's "user-data" qualifier).**
  These are **audit/system** tables, not user-data, so they carry `id` + timestamps but **not** the
  soft-delete/version suite (soft-deleting an audit row is nonsensical). Migration **V4**:
  - `egress_log (id TEXT PRIMARY KEY /*uuid v7*/, created_at INTEGER, provider TEXT, model TEXT,
    token_count INTEGER, source_ids TEXT /*JSON array of SourceId*/, success INTEGER /*bool*/)`.
  - `cloud_consent (provider TEXT PRIMARY KEY, granted_at INTEGER)`.
  `SqliteEgressLog` (`record` serializes `source_ids` to JSON) and `SqliteEgressSettings`
  (`consented`/`grant`/`revoke`) implement the ports. `source_ids` as JSON TEXT is
  `json_each`-queryable for the Slice-2 `qa-report`.

- **D6 — Metadata-only logging; no content, no secrets.** The log stores `SourceId`s, a token count, a
  model/provider, and the success flag — **never** note text, **never** a key. The user learns *what
  kind* and *how much* left without the log becoming a second copy of their notes. (No key exists in
  this slice; the rule is stated so Slice 2's adapter inherits it.)

- **D7 — The gate is the *only* path for every model call, now and forever.** Generation, summarization,
  extraction, entity linking — every future capability obtains completions through `GatedLlmProvider`.
  If the gate does not support a use case, the gate is extended; a new `reqwest`-to-a-model-API anywhere
  outside `raki-ai` is a defect. This is recorded as a standing rule (and should be echoed in
  `AGENTS.md`).

## Architecture / data flow (substrate only)

```
AssembledContext { items, total_tokens, budget, egress: EgressDecision }   [raki-memory]
  → GatedLlmProvider.complete_gated(&ctx.egress, req)                       [raki-ai]
        locality ← inner.locality(); consented ← settings.consented()      (live, per call)
        approve(egress, locality, &consented)?  ── Denied → EgressError (no send, no log)
        result ← inner.complete(req)
        log.record(EgressRecord{ id, decision, completed_at, success: result.is_ok() })
        → result
```

In this slice `inner` is a `FakeLlmProvider` and the loop is driven by tests.

## Components touched

- `crates/raki-domain/src/egress.rs` — CREATE: `SourceId`, `EgressDecision` (+ `summary()`),
  `EgressLogId`, `EgressRecord`, `EgressError`/`EgressDenied`, `EgressLog`, `EgressSettings`.
  `lib.rs` re-exports.
- `crates/raki-memory/src/context.rs` — MODIFY: `egress` field on `AssembledContext`; `egress_of(...)`.
- `crates/raki-ai/src/egress.rs` — CREATE: private `approve()` (locality-aware), `GatedLlmProvider`.
- `crates/raki-ai/src/lib.rs` — MODIFY: `pub mod egress;`; a `FakeLlmProvider` + spy `EgressLog`/
  `EgressSettings` test utilities (the `FakeReranker` pattern).
- `crates/raki-storage/src/migrations.rs` — MODIFY: migration **V4** (`egress_log`, `cloud_consent`,
  `app_settings` kv for future app settings).
- `crates/raki-storage/src/egress.rs` — CREATE: `SqliteEgressLog`, `SqliteEgressSettings`.
- `crates/raki-storage/src/lib.rs` — MODIFY: re-export the two stores.

No `src-tauri/src` (app) or `src/` (frontend) changes.

## Testing & verification

- **`raki-memory`:** `egress_of` yields `source_ids`/`total_tokens` exactly matching the included items;
  assembly still budgets correctly.
- **`raki-ai` policy (pure):** empty egress ⇒ `EmptyContext`; `Locality::Local` ⇒ always `Ok` (no
  egress, no log); `Locality::Cloud` + unconsented ⇒ `ConsentRequired`; consented ⇒ `Ok`.
- **`raki-ai` gate (fakes) — the test that proves the gate is real:** with a spy `EgressLog` and a
  fake `LlmProvider`: (a) `Locality::Local` provider ⇒ inner called, **zero** log rows; (b)
  `Locality::Cloud` provider without consent ⇒ `EgressError`, inner **never called**, zero rows; (c)
  after `grant`, consent is read **live** (no reconstruction) ⇒ inner called once, **one**
  `EgressRecord` with `success:true`; (d) inner returns `Err` ⇒ still one record, `success:false`,
  error propagated; (e) empty egress ⇒ `EmptyContext`, no call, no row.
- **`raki-storage`:** V4 applies on a populated fixture; `record` then read-back round-trips
  `source_ids` JSON; `grant`/`consented`/`revoke` reflect correctly.
- **Workspace:** `cargo test --workspace --exclude raki` / `cargo fmt --check` / `cargo clippy
  --workspace --exclude raki --all-targets -- -D warnings` green; migration tested on a populated
  fixture (AGENTS.md DoD). Frontend untouched.

## Consequences

- The privacy invariant is *enforced*, not aspirational: no approved `EgressDecision` ⇒ no completion,
  by construction; consent changes take effect immediately (no restart); the log records what *actually*
  left, including failures.
- "What left my device?" is answerable from `egress_log`.
- Slice 2 and every future model call are thin layers over a working, tested egress floor.

## Limitations

- **Consent is coarse (per-provider).** No per-note/per-query egress veto yet.
- **Enforcement is at the crate boundary, not absolute.** A developer *inside* `raki-ai` could construct
  the raw provider; mitigation is a crate-private constructor exposing only `GatedLlmProvider`. Outside
  `raki-ai`, an un-gated call does not type-check.
- **Metadata-only log** (D6) cannot reconstruct the exact bytes sent — an intentional privacy trade.
- **No backpressure** (rate-limit/circuit-breaker) yet; the gate is the future home for it.

## Slice 2 preview (NOT this slice — for context only)

Grounded Q&A on the substrate: the generate logic (`build_prompt` / `parse_answer` / `groundedness` —
structured JSON, **parse-or-fail-closed**, default `grounded:false`, citations resolved against
`AssembledContext.source_ids`) in **a `generate` module whose placement (a `raki-generate` crate vs a
module in an existing crate) is decided in Slice 2** — the open tradeoff is CI coverage (the app crate
is `--exclude`d) vs not fragmenting the workspace for one feature. Plus: `CompletionRequest` gains
`system` + `max_tokens`; a `MessagesProvider` (Anthropic wire protocol, Kimi via
`ANTHROPIC_BASE_URL`/`ANTHROPIC_API_KEY`/`RAKI_LLM_MODEL`, key never logged) constructed **inside the
gate**; `answer_question` + `grant_cloud_consent` + an **informed-consent disclosure**; the ask-box
behind an opt-in **"Enable experimental retrieval diagnostics"** setting (not the default view); QA
telemetry folded into `egress_log` + a `qa-report` summarizer; an explicit answer-state enum
(`NothingMatched`/`NotAnswerable`/`ParseFailed`/`Ungrounded`/`Grounded`); a named weakest local-model
target; and a **graduation bar** (>30% `grounded:false` after 30 days dogfooding ⇒ fix retrieval, do
not graduate).
