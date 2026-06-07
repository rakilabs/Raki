# Slice 2 — Grounded Cloud QA (behind the egress gate)

**Status:** Approved design, revised 2026-06-07 after adversarial review (C1–C5, M1–M11, m1–m4 triaged). Builds on Slice 1 (egress + context-assembly substrate, commits `1d6ef78`/`704d560`/`92ae934`).

**Goal:** Make the egress gate actually complete something — answer a user's question from their own notes via a cloud model (Kimi over the Anthropic Messages protocol), with every send passing through `GatedLlmProvider`, the answer constrained to retrieved context, and a per-answer state persisted for the graduation bar.

**One-line architecture:** A new composition crate `raki-generate` orchestrates `retrieve → assemble → gate → answer → verify`; the cloud adapter is a module in `raki-ai`; the Tauri command + ask-box are a thin, manually-verified shell.

---

## Crate placement (a choice, not a law)

`raki-memory`, `raki-ai`, `raki-retrieval` each depend on `raki-domain` **only** (verified in their `Cargo.toml`s), so none can compose a sibling. Orchestration needs retrieval *and* assembly *and* the gated provider, so it cannot live in any leaf.

Two crates *can* host it: `raki-app` (depends on all leaves — the composition root) or a new `raki-generate`. **`raki-app` could host it; we choose `raki-generate` for CI coverage.** The app crate is `--exclude`d from CI (GTK deps), so orchestration placed there would only run under `tauri dev`. `raki-eval` already establishes the precedent — a non-leaf crate that composes `raki-storage` + `raki-ai` + `raki-retrieval` + `raki-domain` precisely so its logic is CI-tested. `raki-generate` mirrors that. This is the Slice 1-line-179 tradeoff ("crate vs module decided in Slice 2; the open tradeoff is CI coverage") resolved toward testability. AGENTS.md's "prefer a module" guidance is honored in spirit — the module *would* have to live in `raki-app`, which forfeits the test guarantee that is this slice's whole point.

The one piece that belongs in a leaf is the `MessagesProvider` adapter (an `impl LlmProvider`, needs only `raki-domain` ports) → `raki-ai`, which already owns providers + the gate.

---

## Design decisions

### D1 — `CompletionRequest` gains `system` + `max_tokens` (`raki-domain`) — Slice 2a
Promised by Slice 1 (lines 86, 181). Today `CompletionRequest { prompt: String }`. Extend to:
```rust
pub struct CompletionRequest {
    pub system: Option<String>,   // grounding rules + numbered context
    pub prompt: String,           // the user's question
    pub max_tokens: Option<u32>,  // bound completion length/cost
}
```
`FakeLlmProvider` and Slice 1's four gate-proof tests construct it with the new fields (`system: None, max_tokens: None`); their assertions are unchanged. No behavioral change to the gate.

### D2 — `MessagesProvider` cloud adapter (`raki-ai`) — Slice 2a
`pub struct MessagesProvider` implementing `LlmProvider`, using `reqwest` (allowed in `raki-ai` per AGENTS.md line 488). Speaks the **Anthropic Messages** wire protocol → one adapter covers Anthropic + Kimi by config:
- **Config (constructor, from env):** base URL = `RAKI_LLM_BASE_URL` **falling back to** `ANTHROPIC_BASE_URL` (the `ckimi` shell fn — the team's Kimi-for-Coding shim — sets the latter to `https://api.kimi.com/coding/`); API key = `ANTHROPIC_API_KEY`; model = `RAKI_LLM_MODEL`. Secrets are read at construction, **never logged or echoed**.
- **`locality() -> Locality::Cloud`.**
- **`complete(req)`:** one POST to `/v1/messages` with `system` = `req.system`, `messages = [{role:"user", content: req.prompt}]`, `max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS)`. Returns `Completion { text }` from the first text block.
- **Timeout + retries (M7):** a **required** request timeout (`REQUEST_TIMEOUT`, e.g. 30s) on the `reqwest::Client`; **one** retry on a transport error (not on 4xx/5xx). Anything still failing → `DomainError::Provider(...)`. (Richer backoff is deferred; the timeout is the hard requirement so the command can't hang.)
- **Tests:** request-building and response/error parsing are unit-tested against canned bytes (no network in CI). The live endpoint is an `#[ignore]` test, like the bge gate.

### D3 — Orchestration `raki-generate::answer_question` — Slice 2a
Real signature (C1), over injected ports — fully fake-testable:
```rust
pub struct GenerateDeps<'a> {
    pub keyword: &'a dyn KeywordIndex,
    pub vectors: &'a dyn VectorIndex,
    pub embedder: &'a dyn EmbeddingProvider,   // assumed LOCAL — see M4 note
    pub notes:   &'a dyn NoteRepository,
    pub gate:    &'a GatedLlmProvider,
    pub model:   &'a str,
    pub budget:  usize,                         // context-assembly token budget
    pub k:       usize,                         // recall depth
}

pub enum GenerateError {                        // C2 — non-egress failures stay distinguishable
    Egress(EgressError),                        // includes Denied(LocalOnlyMode|ConsentRequired)
    Domain(DomainError),                        // retrieval/storage/provider failure
}

pub async fn answer_question(
    query: &str,
    deps: &GenerateDeps<'_>,
) -> Result<Answer, GenerateError>;

pub struct Answer {
    pub state: AnswerState,                      // M6 — see D4
    pub text: String,
    pub cited_ids: Vec<SourceId>,
    pub egress_log_id: Option<EgressLogId>,      // None when no send occurred
}
```
Flow: `hybrid_search(query, k)` → for each id `NoteId::parse` then `notes.get(&id)` → `Vec<Candidate>` →
- **Zero candidates (C5):** short-circuit **before the gate** → `Answer { state: NothingMatched, text: "No relevant notes found.", cited_ids: [], egress_log_id: None }`. No egress, no `EmptyContext` round-trip.
- Else `assemble_context(candidates, budget, "kimi", model)` (carries `.egress`) → build the grounding prompt into `CompletionRequest { system: Some(rules+context), prompt: query, max_tokens: Some(DEFAULT_MAX_TOKENS) }` → `gate.complete_gated(&ctx.egress, req)` (the **only** send) → parse + verify (D4).

A `Denied` from the gate propagates as `GenerateError::Egress` — the command turns it into the consent prompt (D6); `raki-generate` never prompts or mutates settings.

### D4 — Groundedness: rich `AnswerState`, deterministic verdict, one call — Slice 2a
The model is instructed to answer **only from the numbered context** and reply with JSON `{ "answer": "...", "cited_source_ids": ["n3"], "insufficient_context": false }`.

**JSON enforcement (C4):** no reliance on `response_format` (the Messages protocol / Kimi endpoint does not guarantee it). Instead: prompt for bare JSON, then **tolerant extraction** — strip a leading/trailing ```json … ``` fence and take the first balanced `{…}` object — then `serde_json` parse. **Parse-or-fail-closed** (Slice 1 line 177): anything that doesn't parse → `ParseFailed`.

`AnswerState` (M6), computed with no second model call (citations deduped first, m3):
| Condition | State | `grounded` bit (D5) |
|---|---|---|
| 0 retrieval candidates | `NothingMatched` | false (or NULL — no send) |
| JSON did not parse | `ParseFailed` | false |
| `insufficient_context == true` | `NotAnswerable` | false |
| parsed, **0** citations (M10) | `Ungrounded` | false |
| any cite ∉ context `source_ids` | `Ungrounded` | false |
| parsed, ≥1 cite, all present | `Grounded` | true |

`NotAnswerable` is the retrieval-failure signal the graduation bar targets. `NothingMatched`/`ParseFailed`/`Ungrounded` stay first-class so the UI and a future `qa-report` can distinguish them — they do **not** collapse into one bucket.

### D5 — Telemetry: reuse `egress_log`, add a derived `grounded` bit — Slice 2a
One cloud answer = one egress = one verdict (1:1), so no new table:
- Migration **V5**: `ALTER TABLE egress_log ADD COLUMN grounded INTEGER;` (nullable — NULL = not-a-QA-answer or no send).
- `GatedLlmProvider::complete_gated` returns its minted id on success: `Result<(Completion, EgressLogId), EgressError>` (Slice 1 returned `Result<Completion, EgressError>` — a small, intentional breaking change). The denied path carries **no** id and writes **no** row, preserving "no send, no log row." **Verified: the only callers of `complete_gated` are Slice 1's four gate-proof tests** (`grep` finds none in `raki-eval` or the app); those four destructure the tuple, assertions unchanged.
- New port method `EgressLog::set_grounded(&self, id: &EgressLogId, grounded: bool) -> Result<(), DomainError>`; `SqliteEgressLog` updates the row. `raki-generate` calls it after the verdict, mapping `AnswerState::Grounded → true`, else `false`.
- The graduation bar is later a read query: `SELECT avg(grounded = 0) FROM egress_log WHERE grounded IS NOT NULL AND created_at > :cutoff`. Data lands now; the read-side/`qa-report` (and a maintainable named view, m4) are a later slice.

### D6 — Tauri command (thin shell, side-effect-free) — Slice 2b
`answer_question(query)` command in `src-tauri/src`. Returns a **typed serde enum** (M2 — the project uses plain serde DTOs, not `specta`; a tagged enum gives the frontend a clean discriminated union):
```rust
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AnswerOutcome {
    NeedsConsent { provider: String, summary: String, source_titles: Vec<String> },
    Answer { state: String, text: String, cited: Vec<CitedNote> }, // state = AnswerState name
}
```
- Builds `GenerateDeps` from app state (existing indexes/repo/clock + a `GatedLlmProvider` wrapping `MessagesProvider`), calls `raki-generate`.
- On `GenerateError::Egress(Denied(LocalOnlyMode | ConsentRequired))`: re-runs **retrieve+assemble locally** (pure, no send) to build the preview → `NeedsConsent { summary: ctx.egress.summary(), source_titles, provider }`. **The command never mutates consent** (M3).
- On `Ok(Answer)`: `Answer { state, text, cited }`.
- On `GenerateError::Domain(_)`: surfaced as `AppError` (the existing command error type), distinct from the consent path.
- This is the CI-blind surface; verified **manually** in `tauri dev`, never claimed from a test run.

### D7 — Consent flow + commands — Slice 2b
Consent mutation lives in its **own** command (M3, Slice 1 line 183): `grant_cloud_consent(provider)` → `EgressSettings::grant(provider)` + `set_mode(CloudAllowed)`; and `revoke_cloud_consent(provider)` for symmetry.

UX, default mode `LocalOnly`, **behind an opt-in** "Enable experimental retrieval diagnostics" setting (M5, Slice 1 line 184 — the ask-box is **not** in the default `NotesView`; it appears only when the user has enabled the experimental setting):
1. Ask in local-only → backend returns `NeedsConsent` with the egress preview (`summary` + source titles). **Nothing has left the device.**
2. UI shows preview + "Send to cloud" (and "stay local"). Confirm → calls `grant_cloud_consent(provider)`, **then re-submits** the same query → the gate now approves → answer renders with cited source notes inline. Backend stays stateless (no pending-request cache).

### D8 — The gate stays the only send path
`raki-generate` is handed a `GatedLlmProvider`, never a raw `MessagesProvider`. The command constructs `MessagesProvider` and immediately wraps it in `GatedLlmProvider::new(...)`. `raki-ai` continues to re-export only the gate. Enforced by convention + AGENTS.md (as corrected in Slice 1's gate doc-comment), not the type system.

---

## Sub-slice split (two implementation plans)

- **Slice 2a — library core (fully CI-tested):** D1 (`CompletionRequest` fields), D2 (`MessagesProvider` + protocol/timeout unit tests), D3 (`raki-generate` flow with fakes, `NothingMatched` short-circuit), D4 (`AnswerState` verdict + tolerant JSON parse), D5 (V5 column, `complete_gated` returns id, `set_grounded`). **DoD:** `cargo test --workspace --exclude raki` green, with `FakeLlmProvider`-driven cases for every `AnswerState` branch (Grounded / NotAnswerable / Ungrounded-0-cite / Ungrounded-bad-cite / ParseFailed) + `NothingMatched`, plus a spy asserting `set_grounded` gets the right bit.
- **Slice 2b — app shell (manually verified):** D6 (`answer_question` command + `AnswerOutcome`), D7 (`grant_cloud_consent`/`revoke_cloud_consent` + ask-box behind the opt-in). **DoD:** manual `tauri dev` walkthrough — enable the experimental setting → ask in local-only → see egress preview → confirm → grounded answer with cited notes; revoke → back to preview. No completion claim without that manual confirmation.

Build 2a first; it is provable on its own and leaves the app untouched.

---

## Testing strategy
- `raki-generate`: `FakeLlmProvider` returns canned JSON per case; in-memory/fake retrieval + repo. Assert the `AnswerState` for each branch and that `set_grounded` is called with the derived bit (spy on `EgressLog`). Assert `answer_question` sends **exactly once** and only via the gate.
- `MessagesProvider`: build-request + parse-response + error-mapping unit tests against canned bytes; `#[ignore]` live test.
- Gate: Slice 1's four gate-proof tests remain the egress guarantee (updated only to destructure the new tuple).
- App/UI: manual only, explicitly flagged.

## Limitations (acknowledged, not gaps)
- **Online groundedness is a proxy**, not claim-level faithfulness. Mitigation: offline LLM-judge calibration on a sample of logged ⟨query, context, answer⟩ triples during the D8 eval cadence — no extra live egress. (The judge runner is out of scope here.)
- **`insufficient_context` is a single boolean (M11):** an over-conservative model that marks `NotAnswerable` when the answer *is* in context would inflate the graduation bar (a false retrieval alarm). Accepted for v1; the offline judge calibration is precisely what detects this skew. Splitting "notes-silent" vs "model-unsure" is deferred (YAGNI for one telemetry bit).
- **Retrieval is assumed local (M4):** the only `EmbeddingProvider` is `FastEmbedProvider` (on-device); the "local-only re-run" preview makes no network call. If a cloud embedder is ever added, the preview path would itself egress and must be revisited.
- **Coarse consent** — per-provider, not per-note or per-question (after the first).
- **Gate enforcement is by convention** (re-export discipline + AGENTS.md), not the type system; `LlmProvider::complete` is public in the domain.
- **Metadata-only logging unchanged** — `egress_log` stores no note text or keys; `grounded` is a single derived bit.
- **No streaming, multi-turn, history, retrieval auto-tuning, `qa-report`, or bar read-side UI** in this slice.

## Out of scope
Streaming responses, conversation history/multi-turn, the offline judge runner, the `qa-report` summarizer + graduation-bar dashboard, per-note consent, automatic retrieval tuning, richer backoff policies. **Local-model QA** (Slice 1 line 186's "named weakest local-model target") is deferred — this slice is cloud-first (Kimi); a local generate path is a separate future slice, and the gate already treats local-only as the safe default.
