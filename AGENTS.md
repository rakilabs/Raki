# AGENTS.md — Raki

> **Read this first, every time.** This file is the operating contract for any human or AI agent
> working in this repository. It encodes *how* we build Raki so that the codebase stays fast,
> private, reliable, and — above all — **safe to change** as it grows from a notes app into a
> full personal operating system.
>
> If a rule here conflicts with a habit, the rule wins. If a rule here is wrong, **change the rule
> in a PR** (and write an ADR) — don't quietly ignore it.

---

## 0. How an agent should operate in this repo

You are working in **Raki**, a local-first, AI-native "second brain." Before you touch code:

1. **Read the relevant module's boundary.** Every crate/slice has one job (Section 4). Find the one that owns the change.
2. **Stay inside the dependency rule.** Dependencies point *inward* toward `raki-domain`. If your change needs to violate this, you've found the wrong seam — stop and reconsider.
3. **Change the contract before the implementation.** Types and traits first; implementations follow the types.
4. **Make the smallest change that fully solves the task.** No drive-by refactors, no speculative abstraction.
5. **Prove it.** A change isn't done until the Definition of Done (Section 14) is green.

When in doubt, prefer the choice that keeps the **memory + retrieval layer** correct and the **user's data
local, owned, and recoverable**. Those two are the product.

---

## 1. Product context & long-term vision

**Raki is a local-first, AI-native second brain that runs on the user's machine.** It unifies notes,
tasks, personal finance, and (later) calendar, habits, reading, browser capture, and automation into
one coherent personal operating system — where an AI layer can connect anything to anything.

### The non-negotiable product values

| Value | What it means for the code |
|---|---|
| **Local-first** | Data lives in one SQLite file on the user's disk. The app is fully usable with no network. |
| **User owns the data** | Everything is exportable to open formats (Markdown, JSON, CSV). One file to back up. |
| **AI is provider-agnostic** | The user chooses, swaps, and mixes **local** (Ollama, fastembed) and **cloud** (OpenAI/Anthropic-compatible) providers. We never hard-code a provider. |
| **Privacy is auditable** | Cloud calls are explicit, consented, scoped, and logged. The user can always answer "what left my device?" |
| **Reliability over features** | A feature that can corrupt or lose data is worse than no feature. |

### What "AI-native" means here

The **AI Memory Layer and Retrieval Layer are the core of the product**, not an add-on. The whole
architecture is shaped so that notes, tasks, finance records, and activity can be embedded, retrieved,
linked, and assembled into context that local or cloud models reason over — privately and fast.

### The arc (do not build all of this now)

```
Phase 1 (NOW): Foundation — domain kernel, storage, hybrid retrieval, embedding pipeline,
               memory lifecycle skeleton, context assembly, provider abstraction, notes module.
Phase 2:       Tasks + cross-module linking graph maturity.
Phase 3:       Finance.
Phase 4+:      Calendar, habits, reading, browser capture, automation, agents.
```

> **Phase-1 discipline:** build the *foundation* deeply and the *first vertical slice* (notes) end-to-end.
> Do **not** scaffold tasks/finance/calendar modules speculatively. The architecture must *support* them;
> it must not *contain* them yet.
>
> **Phase 1 is not "done" when notes merely work.** Retrieval/memory quality is the platform's core
> differentiator; Phase 1 completes only when that quality is driven to *best* and *measured* — not just
> functional. Every retrieval lever is gated on a corpus where today's retrieval *fails* (ADR-0005,
> ADR-0006, ADR-0007). Breadth waits behind a genuinely strong core.
>
> **Sequencing lives in `docs/ROADMAP.md`** — the living, dependency-ordered tracking file. Read it at the
> start of every slice and pick the next milestone there. This "arc" is the phases; the roadmap is the steps.

---

## 2. Non-negotiable architecture principles

These are the laws. Everything else is detail.

1. **The dependency rule.** Dependencies point inward toward `raki-domain`. `raki-domain` depends on
   nothing app-specific. **Nothing** depends on `raki-app`. The Cargo workspace enforces this at compile
   time — a violation won't build.

2. **Deep modules, narrow interfaces.** Prefer one module that does a lot behind a small public API over
   many shallow modules that leak their internals. A module you can describe in one sentence and use without
   reading its internals is correct. (Ousterhout: deep modules; this is the spine of the whole repo.)

3. **Domain language, not framework folders.** We organize around `notes`, `memory`, `retrieval`,
   `finance` — not `controllers`, `services`, `utils`. The folder names are the product's vocabulary.

4. **Ports & adapters.** The domain defines **traits** (ports). Storage, AI, and OS integrations are
   **adapters** that implement them. Services depend on traits and receive concrete adapters by injection.
   This is what makes the memory layer testable without SQLite or a model.

5. **One source of truth per fact.** Relational rows, the FTS index, and vectors live in **one SQLite
   file** and move together in one transaction. There is never a "vector DB drifted out of sync" bug.

6. **Typed boundaries, generated where possible.** The Rust↔TypeScript IPC contract is generated from Rust
   (Section 5/6). No stringly-typed `invoke("...")`. Contract drift must be a compile error.

7. **Explicit over implicit.** Explicit IDs, explicit timestamps, explicit egress, explicit errors.
   No magic, no hidden global state, no silent network calls.

8. **YAGNI, ruthlessly — but never paint into a corner.** Don't build for imagined futures. *Do* leave
   the seams (traits, IDs, change-log) that make the known futures cheap. Abstraction is earned by a
   *second* real use case, not predicted by the first.

9. **Surgical changes.** A change should touch the fewest files necessary and be obviously correct.
   If a change ripples across many modules, the boundary is wrong — fix the boundary, then make the change.

10. **Document the "why."** Every important decision gets an ADR (`docs/adr/`). Code says *what*; ADRs say *why*.

---

## 3. Repository layout

```
Raki/
├── AGENTS.md                  ← you are here (the operating contract)
├── README.md
├── docs/
│   ├── adr/                  ← Architecture Decision Records (the "why")
│   │   ├── 0000-template.md
│   │   ├── 0001-provider-agnostic-ai.md
│   │   ├── 0002-single-device-sync-ready-data-model.md
│   │   ├── 0003-sqlite-vec-single-file-vectors.md
│   │   └── 0004-prosemirror-json-canonical-note-format.md
│   └── architecture/         ← deeper design notes when an ADR isn't enough
│
├── src/                      ← FRONTEND (SolidJS) — see Section 5
│   ├── app/                  ← shell, routing, global providers, layout
│   ├── shared/               ← design system, ipc client, GENERATED types, hooks, lib
│   │   ├── ui/               ← design-system primitives (Button, Dialog, …) — no business logic
│   │   ├── ipc/              ← typed IPC client + GENERATED bindings (do not edit generated files)
│   │   ├── hooks/
│   │   └── lib/
│   ├── modules/              ← VERTICAL SLICES (one folder per product domain)
│   │   ├── notes/            ← Tiptap editor, note list, note signals, notes/api.ts
│   │   ├── search/           ← command palette + hybrid search UI
│   │   └── memory/           ← memory inspector / context preview
│   └── styles/
│
└── src-tauri/                ← BACKEND (Rust) — see Sections 4, 6, 7
    ├── Cargo.toml            ← workspace manifest
    ├── tauri.conf.json
    ├── src/                  ← raki-app role: main.rs, lib.rs (run()), state.rs, error.rs, dto.rs, commands/
    └── crates/
        ├── raki-domain/      ← pure types + PORT TRAITS. No IO. No tauri. No SQL.
        ├── raki-storage/     ← rusqlite (bundled SQLite + FTS5 + sqlite-vec). THE ONLY SQL.
        ├── raki-retrieval/   ← hybrid search: BM25 ⊕ vector KNN, RRF fusion, chunking, rerank
        ├── raki-ai/          ← provider abstraction (local + cloud) + egress/consent policy
        └── raki-memory/      ← embedding pipeline, memory extraction/lifecycle, context assembly, grounded answer orchestration; depends on raki-retrieval for recall

> Note: `raki-app` from the dependency graph is realized as the **`src-tauri` root package** itself — Tauri's `generate_context!` must run in the crate that owns `tauri.conf.json`. The 5 crates under `crates/` are the inward layers; nothing depends on the `src-tauri` package, so the dependency rule still holds.
```

### The dependency graph (memorize this)

```
            ┌─────────────────────────────────────────────┐
            │                  raki-app                    │  (composition root, tauri commands)
            └───────┬───────────┬──────────┬──────────┬────┘
                    │           │          │          │
              raki-memory   raki-retrieval raki-ai  raki-storage
                    │           │          │          │
                    └───────────┴────┬─────┴──────────┘
                                     ▼
                                raki-domain   (types + traits; depends on ~nothing)
```

- `raki-memory` depends on **`raki-domain`** and **`raki-retrieval`** (for recall). It receives
  `Arc<dyn Trait>` adapters (storage, ai) injected by `raki-app`. It does **not** depend on
  `raki-storage`, `raki-ai`, or `raki-app` directly.
- `raki-retrieval` depends on **`raki-domain` only**.
- `raki-storage` & `raki-ai` depend on **`raki-domain` only** (they implement its ports).
- `raki-app` depends on everything and wires it together. **Nothing depends on `raki-app`.**

---

## 4. Domain boundaries (who owns what)

Each crate is a **deep module**: a clear purpose, a public interface (`pub` items in `lib.rs`), and an
ownership boundary. If you can't say what a crate does in one sentence, it's wrong.

| Crate | One-sentence purpose | Owns | Must NOT contain |
|---|---|---|---|
| **`raki-domain`** | The vocabulary and contracts of Raki. | Entity types (`Note`, `Block`, `Task`, `Memory`, `Chunk`, `Entity`, `Link`), value objects, IDs, error enums, and **port traits** (`Repository`, `VectorIndex`, `KeywordIndex`, `EmbeddingProvider`, `LlmProvider`, `Clock`). | Any IO, SQL, HTTP, `tauri`, or `tokio` runtime. (Traits may be `async` via `async-trait`.) |
| **`raki-storage`** | Persist and retrieve domain entities in one SQLite file. | rusqlite connections (WAL), migrations, repositories, FTS5 + `sqlite-vec` primitives, the change-log. | Business rules, ranking/fusion logic, AI calls, UI concerns. |
| **`raki-retrieval`** | Turn a query into ranked, provenance-tagged results. | Chunking strategy, BM25 + vector KNN orchestration, **RRF fusion**, optional rerank, query planning. | SQL strings (calls storage ports), model loading (calls ai ports). |
| **`raki-ai`** | Provide embeddings & completions from any provider, safely. | `EmbeddingProvider`/`LlmProvider` implementations (Ollama, fastembed, OpenAI/Anthropic-compat), provider registry, **egress policy & consent gate**, retries/backoff. | Persistence, retrieval ranking, memory rules. |
| **`raki-memory`** | The brain: ingest, remember, assemble context, and orchestrate grounded answers. | The embedding **pipeline** (orchestration), memory **extraction & lifecycle**, **context assembly** (`AssembledContext`), **grounded answer orchestration** (`AnswerService`). | SQL, HTTP, model loading — all via injected ports. |
| **`raki-app`** | Wire everything and expose it to the UI. | App state, the composition root (construct adapters, inject into services), `#[tauri::command]` adapters, IPC type generation. | Business logic (commands are thin), SQL, ranking, provider details. |

> **Rule of ownership:** if you're adding logic and you're not sure which crate owns it, ask: *"Could I unit-test
> this without a database, without a network, and without tauri?"* If yes → it belongs in `raki-domain` or a
> service (`raki-memory`/`raki-retrieval`). If it needs SQLite → `raki-storage`. If it needs a model/HTTP →
> `raki-ai`. If it needs `tauri` → `raki-app` (and it should be a thin adapter, not logic).

---

## 5. Frontend architecture rules (SolidJS)

**Stack:** SolidJS `1.9` · @solidjs/router `0.16` · TipTap `3.25` (+ `solid-tiptap`) · @tanstack/solid-query
`5.x` · Vite `8` · TypeScript `6` · Vitest `4`.

### Structure: vertical slices over a clean shared core

- A **module** (`src/modules/<name>/`) is a self-contained product slice. Standard shape:
  ```
  modules/notes/
    components/      ← Solid components for this slice only
    signals.ts       ← local reactive state (createSignal/createStore)
    api.ts           ← typed IPC calls for this slice (wraps shared/ipc client)
    types.ts         ← slice-local view types (NOT the generated IPC types)
    index.ts         ← the slice's public surface
  ```
- `src/shared/` is the **only** place modules share code: the design system (`ui/`), the IPC client,
  generated types, and generic hooks. **Modules never import from each other.** Cross-module needs go
  through `shared/` or through a Tauri command — never a direct `modules/finance` → `modules/notes` import.

### Reactivity & state rules

1. **Fine-grained, not virtual-DOM habits.** This is Solid. Components run **once**. Never destructure props
   (`const { x } = props` breaks reactivity) — read `props.x` in the JSX/effect. Don't reach for `createMemo`
   until you measure a real recompute cost.
2. **Server state ≠ UI state.** Async data from Rust commands belongs in **`@tanstack/solid-query`** (caching,
   invalidation, loading/error states). Ephemeral UI state (open/closed, input text) belongs in signals.
   Don't hand-roll `createSignal` + `createEffect(fetch)` cascades for server data.
3. **No business logic in components.** Components render and dispatch. Decisions (what to embed, how to rank,
   what's a valid task) live in Rust. The frontend is a *view* of the second brain, not a second copy of its rules.

```tsx
// GOOD — server state via query, command via typed client, no logic in the component
import { createQuery } from "@tanstack/solid-query";
import { commands } from "~/shared/ipc";

function NoteList() {
  const notes = createQuery(() => ({
    queryKey: ["notes"],
    queryFn: () => commands.listNotes(),   // typed, generated — not invoke("list_notes")
  }));
  return <For each={notes.data ?? []}>{(n) => <NoteRow note={n} />}</For>;
}
```

```tsx
// BAD — stringly-typed invoke, destructured props, fetch-in-effect, ranking logic in the UI
function NoteList(props) {
  const { filter } = props;                       // ✗ breaks Solid reactivity
  const [notes, setNotes] = createSignal([]);
  createEffect(async () => {
    const all = await invoke("list_notes");        // ✗ stringly-typed, untyped result
    setNotes(all.sort((a,b) => score(b)-score(a))); // ✗ ranking belongs in raki-retrieval
  });
  return <For each={notes()}>{(n) => <NoteRow note={n} />}</For>;
}
```

### Editor rules (TipTap / ProseMirror)

- **Canonical content is ProseMirror JSON** (see ADR-0004). Markdown is an *export/import projection*, never the
  source of truth on disk.
- Every block carries a **stable block ID** (a TipTap extension assigns a UUID on creation). Block IDs are the
  unit of retrieval chunking and of block-level linking — they must survive edits.
- The editor is **headless**: styling lives in the design system. Don't fork TipTap's schema casually — schema
  changes are migrations (Section 7) because they change stored documents.

### Frontend ⇄ backend

- All calls go through **`src/shared/ipc/`** — the typed client and **generated** bindings. Treat
  `shared/ipc/bindings.ts` (or equivalent) as read-only build output.
- A command returns a **DTO**, not a domain entity verbatim. The frontend type is whatever Rust generates;
  never re-declare it by hand.

---

## 6. Rust / Tauri backend architecture rules

**Stack:** tauri `2.11` · rusqlite `0.35` (bundled SQLite, FTS5) · `sqlite-vec` · fastembed `5.15` ·
candle/mistralrs (optional embedded) · reqwest `0.13` · tokio `1.52` · serde `1.0.228` · uuid `1.23`
(v7) · jiff `0.2` · thiserror `2.0` · tracing `0.1`.

### Workspace & boundaries

- **Six crates, one workspace.** New cross-cutting capability → usually a new module *inside* an existing
  crate, not a new crate. Add a crate only when a capability has a genuinely distinct purpose + ownership.
- **The dependency rule is law** (Section 2/3). `raki-domain` has zero app-specific deps. If you reach for
  `raki-storage` from `raki-memory`, stop — inject the trait instead.

### Ports live in the domain; adapters implement them

```rust
// raki-domain/src/ports.rs — the contract. Pure. No SQL, no HTTP.
#[async_trait::async_trait]
pub trait NoteRepository: Send + Sync {
    async fn get(&self, id: NoteId) -> Result<Option<Note>, DomainError>;
    async fn upsert(&self, note: &Note) -> Result<(), DomainError>;
    async fn list(&self, page: Page) -> Result<Vec<Note>, DomainError>;
}

#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    fn model(&self) -> EmbeddingModel;        // id + dimension + version
    fn locality(&self) -> Locality;           // Local | Cloud
    async fn embed(&self, inputs: &[String]) -> Result<Vec<Embedding>, AiError>;
}
```

```rust
// raki-storage/src/notes.rs — an adapter. The ONLY place SQL for notes exists.
pub struct SqliteNoteRepository { db: Database }

#[async_trait::async_trait]
impl NoteRepository for SqliteNoteRepository {
    async fn upsert(&self, note: &Note) -> Result<(), DomainError> {
        self.db.write(move |conn| { /* INSERT ... ON CONFLICT ... */ Ok(()) }).await
    }
    // ...
}
```

```rust
// raki-app/src/lib.rs — the composition root wires concrete adapters into services.
let db = Database::open(&data_dir.join("raki.sqlite"))?;     // WAL + sqlite-vec loaded
let notes = Arc::new(SqliteNoteRepository::new(db.clone()));
let embedder = ai::registry::active_embedding_provider(&config)?;  // local or cloud, user's choice
let memory = MemoryService::new(notes.clone(), index.clone(), embedder.clone());  // gets TRAITS
```

### Tauri commands are thin adapters — never logic

```rust
// raki-app/src/commands/notes.rs
#[tauri::command]
#[specta::specta]                       // generates the TS type for this command
pub async fn save_note(
    state: tauri::State<'_, AppState>,
    input: SaveNoteInput,               // a DTO, derives Serialize/Deserialize/Type
) -> Result<NoteDto, AppError> {
    state.notes.save(input.into()).await   // delegate; the rule is in the service/domain
        .map(NoteDto::from)
        .map_err(AppError::from)
}
```

> **Bad:** a `#[tauri::command]` that opens a DB connection, runs SQL, ranks results, and calls a model.
> That's four boundary violations in one function. Commands **translate and delegate**, nothing more.

### Typed IPC contract (kill `invoke("string")`)

- Generate the TypeScript command bindings + types **from Rust** using **`tauri-specta`** (Tauri-v2 line;
  currently on the `2.x` RC channel) with `specta`/`specta-typescript`. If you want a fully-stable, type-only
  generator instead, use **`ts-rs`** to emit the DTO types and keep a thin hand-written command map.
- Either way: **Rust is the single source of truth for the contract.** Regenerate bindings as part of the
  build; commit the generated file; never hand-edit it.

### Concurrency & async

- **SQLite is single-writer.** Model it honestly: one **writer** path (serialized) + a **reader pool**
  (WAL allows concurrent readers). Use `tokio-rusqlite` (background-thread serialization) **or** an `r2d2`
  reader pool + a dedicated writer behind `spawn_blocking`. Never share one `rusqlite::Connection` across threads.
- Long work (embedding, extraction, indexing) runs **off the UI path** as background jobs. Commands enqueue and
  return fast; progress flows back via Tauri **events**, not blocking calls.
- Hold **no locks across `.await`** that a command also needs. Prefer message-passing to shared `Mutex` for the writer.

### Errors

- **Library crates** (`domain`, `storage`, `retrieval`, `ai`, `memory`) use **`thiserror`** — typed, matchable errors.
- **`anyhow` only at the edge** (rare). Commands map domain errors to a single serializable **`AppError`** the
  frontend can pattern-match. Never let a raw `rusqlite::Error` or `reqwest::Error` cross the IPC boundary.
- **No `unwrap()`/`expect()` in non-test code** except for genuinely impossible states, and then with a message
  explaining why it's impossible.

### Development tooling

The Rust workspace owns its own toolchain and quality gates. Don't bypass them locally.

- **Toolchain:** `src-tauri/rust-toolchain.toml` pins `stable` and the required components
  (`rustfmt`, `clippy`, `rust-analyzer`). New clones automatically pick this up.
- **Formatting:** `src-tauri/rustfmt.toml` is the source of truth. Run `cargo fmt` before committing.
- **Linting:** clippy is enforced. In VS Code, rust-analyzer runs `clippy --all-targets` on save.
  CI / pre-commit should reject `-D warnings` failures.
- **Build config:** `src-tauri/.cargo/config.toml` sets sparse registry, `sccache` as the rustc wrapper,
  and optimized `dev` / `release` profiles. Install `sccache` (`brew install sccache` on macOS) or
  comment the wrapper line locally if it isn't available on your machine.
- **Tests:** use `cargo nextest run` instead of `cargo test` for clearer output and faster execution.
- **Editor settings:** `.vscode/settings.json` keeps rust-analyzer, formatting, and clippy consistent
  across contributors.

---

## 7. Database & migration rules

**One file. One source of truth. Transactional. Recoverable.**

### The store

- **rusqlite with bundled SQLite** (ships FTS5). The **`sqlite-vec`** extension is registered as an
  auto-extension at startup so every connection has vector functions/virtual tables.
- **WAL mode** + `foreign_keys = ON` + a sane `busy_timeout`, set on every connection.
- The DB file lives in the Tauri app data dir: `<app_data>/raki.sqlite`. It is the **only** thing the user
  must back up. Export must reproduce all content in open formats.

### Row conventions (sync-ready — ADR-0002)

Every user-data table carries:

| Column | Type | Why |
|---|---|---|
| `id` | TEXT (UUID **v7**) | Stable, sortable, globally unique — safe for future multi-device merge. |
| `created_at` | INTEGER (epoch ms) | Provenance + ordering. |
| `updated_at` | INTEGER (epoch ms) | Last-write tracking. |
| `deleted_at` | INTEGER NULL | **Soft delete.** We tombstone, we don't hard-delete user data. |
| `version` | INTEGER | Monotonic per-row counter (optimistic concurrency + future sync). |

A `change_log` table records every mutation (entity id, op, timestamp, version). **No sync runtime today** —
but this is the seam that makes file-based or CRDT sync addable later without a data rewrite.

### Migrations

- Migrations are **forward-only, versioned, and embedded** (use **`refinery`**, or a hand-rolled versioned
  runner). They run automatically at startup, in a transaction, before the app serves any command.
- **Never edit a shipped migration.** Add a new one. A migration that has run on any user's machine is immutable.
- **Schema changes that touch stored documents (TipTap schema, chunking) are migrations too** — include a
  re-index/re-embed step where needed.
- Every migration is tested on a populated fixture DB (Section 10), not just an empty one.

```sql
-- GOOD: additive, reversible-in-spirit, indexed
ALTER TABLE notes ADD COLUMN archived_at INTEGER NULL;
CREATE INDEX idx_notes_updated ON notes(updated_at) WHERE deleted_at IS NULL;
```
```sql
-- BAD: destructive, irreversible, no backfill — this loses user data
ALTER TABLE notes DROP COLUMN body;          -- ✗ never drop user content in a migration
```

### Indexes (FTS5 + vectors live here)

- The **FTS5** virtual table and the **`sqlite-vec`** vector table are populated **in the same transaction**
  as the source row. A note, its searchable text, and its embedding commit or roll back together (Principle 5).
- Embeddings store their `embedding_model` + `model_version`. Switching models triggers a background re-embed;
  we never silently mix vector spaces.

---

## 8. AI Memory Layer design rules (the core priority)

This is the product. Build it deeply, test it hard, keep it boring and inspectable.

### Three data tiers

```
RAW          authored truth: notes, blocks, tasks, finance txns. The user typed/imported these.
DERIVED      AI-extracted MEMORIES: atomic facts, preferences, entities, relationships, summaries.
             Every memory has provenance (→ source block), confidence, created/last-seen, and a kind.
GRAPH        Entities + Links connecting everything (Section 9 / cross-module linking).
```

> **Hard rule: derived never overwrites raw.** Extraction *reads* raw and *writes* memories with a link back.
> If extraction is wrong, you delete the memory; the user's words are untouched.

### The embedding pipeline (orchestrated in `raki-memory`)

```
content change ─▶ enqueue IndexJob ─▶ block-aware chunker (stable block IDs)
   ─▶ EmbeddingProvider.embed(batch)            (local fastembed OR cloud — user's choice)
   ─▶ store {chunk, vector, model_version} in sqlite-vec  +  upsert FTS5   (ONE transaction with source)
   ─▶ mark indexed
```

- **Chunking is block-aware**, not fixed-window: chunks align to ProseMirror blocks so a retrieved chunk maps
  back to a real, linkable location. Long blocks are split with overlap; tiny blocks may be merged.
- The pipeline is **idempotent and resumable** (re-running a job is safe; a crash mid-index self-heals on restart).
- Embedding happens **in the background**, never on the save path's critical section. Saving a note is instant;
  indexing catches up.

### Memory lifecycle

```
extract ─▶ deduplicate/merge ─▶ score (confidence × recency × retrieval-frequency)
        ─▶ promote (frequently useful memories rank higher) ─▶ decay/forget (soft-delete low value)
```

- Extraction is **explainable**: each memory records *which model*, *which source*, *when*. The user (and the
  memory inspector UI) can audit and correct it.
- Lifecycle policies (decay rate, dedup threshold) are **config**, not magic constants buried in code.

### Context assembly — the single bridge to any LLM

There is exactly **one** function that decides what a model sees:

```rust
// raki-memory/src/context.rs
pub struct AssembledContext {
    pub items: Vec<ContextItem>,   // each carries: source ref, text, token_count, why_included
    pub total_tokens: usize,
    pub budget: usize,
    pub egress: EgressDecision,    // what WOULD leave the device if this goes to a cloud provider
}

pub async fn assemble_context(req: &ContextRequest, deps: &MemoryDeps) -> Result<AssembledContext, MemoryError>;
```

- Assembly is **deterministic and unit-testable** with fake providers (no model, no DB). Given the same inputs
  and config, it returns the same context.
- It is **token-budgeted**: it composes recent + retrieved chunks + relevant memories + linked entities up to a
  budget, with a stable priority order, and records *why* each item was included.
- It produces an **`EgressDecision`** so the privacy layer (Section 12 of the brief / below) can show/log exactly
  what a cloud call would transmit **before** it's sent.

### Privacy & egress (because cloud providers are allowed)

- Every model call goes through `raki-ai`, which enforces a **locality-aware egress policy**:
  **local providers** (Ollama, etc.) run on-device and need no consent; **cloud providers** (Kimi,
  Claude, OpenAI, etc.) require per-provider **consent**, and every cloud call is **logged** so the
  user can always answer "what left my device?".
- A cloud completion **must** carry an `AssembledContext` whose `egress` was approved by policy. No ad-hoc
  `reqwest` calls to model APIs anywhere outside `raki-ai`.

---

## 9. Retrieval / search architecture rules

**Hybrid by default. Provenance always. Fusion over tuning.**

### The pipeline (`raki-retrieval`)

```
query ─┬─▶ FTS5 BM25 keyword search ─────┐
       └─▶ sqlite-vec vector KNN ─────────┤
                                          ▼
                       Reciprocal Rank Fusion (RRF)   score = Σ 1/(k + rank_i),  k ≈ 60
                                          ▼
                       optional rerank (cross-encoder / cloud) on top-N
                                          ▼
                       ranked results WITH provenance (note id + block id)
```

- **RRF, not hand-tuned score weighting.** Rank fusion is robust and needs no score normalization between BM25
  and cosine similarity. Tune `k` and `N`, not a fragile linear combination.
- **Keyword and vector run in parallel**, then fuse. Never make semantic search a *replacement* for keyword
  search — exact-match recall matters (IDs, names, code).
- **Every result is traceable** to its source block. A result the user can't click back to is a bug.
- Retrieval logic lives in `raki-retrieval` and calls **storage ports** (`KeywordIndex`, `VectorIndex`) — it
  contains **no SQL** and loads **no models** (it asks `raki-ai` via a port when reranking).

### Scale posture

- `sqlite-vec` **exact** search is correct and fast at personal scale (well into hundreds of thousands of
  chunks). We do **not** add ANN/quantization until a real corpus proves we need it — and when we do, it's a
  swap behind the `VectorIndex` trait, not a rewrite (ADR-0003).

---

## 10. Testing strategy

Test the **core** like the product depends on it — because it does. Test the **edges** lightly.

| Layer | What to test | How |
|---|---|---|
| `raki-domain` | Pure logic, invariants, value objects | Fast unit tests. No IO. |
| `raki-memory` / `raki-retrieval` | Pipeline, lifecycle, **context assembly**, fusion ranking | Unit tests with **fake adapters** (in-memory repo, fake embedder). This is where coverage must be highest. |
| `raki-storage` | Migrations, repositories, FTS5 + vec round-trips, soft-delete, change-log | Integration tests against a **real temp SQLite file**. |
| `raki-ai` | Provider adapters, egress policy, retries | Unit-test the policy/registry; mock HTTP for cloud adapters; mark live-model tests `#[ignore]`. |
| `raki-app` | Command (de)serialization, error mapping, wiring | Thin tests — commands are thin. |
| Frontend | Module signals, IPC client shape, critical components | Vitest; mock the IPC client. |

### Rules

1. **The memory/retrieval layer is tested with fakes, not a database or a model.** If you can't unit-test
   `assemble_context` without SQLite, the ports are wrong.
2. **Determinism:** embedding/model outputs are stubbed in tests. Use fixed fake vectors so fusion/ranking
   assertions are stable.
3. **Migrations are tested against a populated fixture**, not just an empty schema. A migration that works on
   an empty DB but corrupts real data is the nightmare we test against.
4. **Bugfix ⇒ regression test first.** Reproduce the bug as a failing test, then fix it. (See `superpowers:test-driven-development` / `systematic-debugging`.)
5. **No test hits the network or a live model in CI** unless explicitly `#[ignore]`d and run on demand.

---

## 11. Refactoring rules

1. **Refactor on a green bar.** Never refactor and change behavior in the same commit. Tests pass before and after.
2. **Refactor toward depth.** The goal is fewer, deeper modules with narrower interfaces — not more files.
   Merging two leaky modules into one clean one is progress; splitting one clean module into three is usually not.
3. **Boundary-respecting only.** A refactor may not introduce a dependency-rule violation to "save time."
   If a refactor wants to, the design is telling you the boundary is wrong — fix that first, in its own PR.
4. **No drive-by refactors in a feature PR.** Improve code you're *already* changing for the feature; file an
   issue for the rest. Mixed PRs hide bugs and bloat review.
5. **Rename to the domain.** If code uses a word the product doesn't (`manager`, `helper`, `util2`), renaming it
   to domain language *is* a valid, valuable refactor.
6. **Delete fearlessly, with proof.** Dead code goes. But first confirm it's dead (grep usages, check generated
   bindings) — and never delete user-facing data paths without an ADR.

---

## 12. Code review checklist

Reviewer (human or agent) confirms **all** of:

- [ ] **Boundary:** change lives in the crate/slice that owns it; **no dependency-rule violation** (it compiles, but *also* check imports).
- [ ] **Commands are thin:** no SQL/ranking/model calls inside `#[tauri::command]`.
- [ ] **Typed contract:** no new `invoke("string")`; IPC types are generated, not hand-declared; bindings regenerated.
- [ ] **One source of truth:** note/text/vector mutations are in **one transaction**; FTS + vec stay consistent with the row.
- [ ] **Data safety:** migrations are additive/forward-only; soft-delete used; no shipped migration edited; no raw user content dropped.
- [ ] **Privacy:** any model call goes through `raki-ai`; cloud calls carry an approved `EgressDecision`; nothing leaks PII outside the policy.
- [ ] **Errors:** typed (`thiserror`) in libs; mapped to `AppError` at the edge; no `unwrap()` in non-test code.
- [ ] **Tests:** core logic tested with fakes; bugfix has a regression test; migrations tested on a populated fixture; CI is green.
- [ ] **Reactivity (frontend):** props not destructured; server state in solid-query; no business logic in components.
- [ ] **Surgical:** smallest change that solves it; no speculative abstraction; no unrelated refactors.
- [ ] **Documented:** an ADR exists for any important/contested decision; public items have doc comments.

---

## 13. "Before coding" checklist

Before writing a line, the agent confirms:

- [ ] I can state the task in one sentence and name the **one** crate/slice that owns it.
- [ ] I've read that module's public interface and the relevant ADRs.
- [ ] I know which **trait/contract** changes (if any) — and I'll change the type/trait *before* the implementation.
- [ ] My change respects the **dependency rule** (inward-only). If it can't, I'll fix the boundary first (separate PR/ADR).
- [ ] This is the **smallest** change that fully solves it; I'm not building for an imagined future.
- [ ] I know how I'll **test** it (which fakes, which fixtures) and how I'll **prove** it works.
- [ ] If this is a contested/important decision, I'll write an **ADR**.

> For any non-trivial feature, run the `superpowers:brainstorming` flow first, then `superpowers:writing-plans`.
> Don't jump from idea to code.

---

## 14. Definition of Done

A change is **done** only when **all** are true:

- [ ] Builds clean: `cargo build` (workspace) and the frontend build both succeed.
- [ ] **Lints clean:** `cargo clippy -- -D warnings`, `cargo fmt --check`; frontend `tsc --noEmit` + linter pass.
- [ ] **Tests pass:** `cargo test --workspace` + `vitest` green; new logic has tests; bugfixes have regression tests. (Use `--workspace` — plain `cargo test` from the `src-tauri` root package runs only its own tests and silently skips every `crates/*` library test.)
- [ ] **IPC bindings regenerated & committed** if any command/DTO changed.
- [ ] **Migrations** (if any) run cleanly on a populated fixture and are forward-only.
- [ ] **No boundary violations**, no new `invoke("string")`, no `unwrap()` in non-test code.
- [ ] **Data-safe & privacy-safe:** soft-delete respected; egress policy honored; no secrets/PII logged.
- [ ] **Docs updated:** ADR added for important decisions; public APIs documented; `AGENTS.md` updated if a *rule* changed.
- [ ] Verified in the **real app** for user-facing changes (not just unit tests) — see `superpowers:verification-before-completion`.

---

## 15. Rules to avoid over-engineering

The hard part of "best long-term solution" is knowing when *less* is more long-term.

1. **Abstraction is earned by the second real use case, not the first.** One implementation of a thing is a
   concrete type, not a trait — *unless* the trait is a known seam we deliberately chose (the ports in Section 4).
2. **No speculative modules.** Don't create `raki-tasks`/`raki-finance` until you're building that slice. The
   architecture *supports* them; it doesn't *pre-contain* them.
3. **No premature performance work.** `sqlite-vec` exact search, synchronous-feeling APIs, simple chunking —
   ship them, measure, *then* optimize the proven hotspot. ANN, caching layers, and custom kernels are earned.
4. **No frameworks-within-the-framework.** Don't build a generic plugin system, event bus, or DI container until
   two concrete features demand it. Constructor injection at the composition root is enough for now.
5. **Config over knobs-everywhere.** Lifecycle/decay/budget values are config in one place — not parameters
   threaded through ten function signatures "for flexibility."
6. **Three strikes for helpers.** Don't extract a shared utility until the third duplication. Two is a coincidence.
7. **Prefer deleting to abstracting.** If a layer exists "just in case," and nothing uses it, delete it.

> The tension with "we choose the best long-term solution": *long-term* health comes from **fewer, deeper,
> well-bounded modules** — not from maximal abstraction. Over-engineering is itself a long-term liability.

---

## 16. Rules to stay scalable, maintainable & AI-agent-friendly

This codebase is navigated by **both** humans and AI agents. Optimize for "an agent can find the one place to
change, change it safely, and prove it."

1. **The crate graph is the map.** Boundaries enforced by the compiler mean an agent *cannot* get lost across
   layers. Keep the graph clean; never add an edge that points outward.
2. **Domain language everywhere.** File names, types, and functions use the product's words. An agent searching
   for "memory extraction" should find `raki-memory/src/extraction.rs`, not `service_impl_v2.rs`.
3. **One concept, one home.** Each concept (a Note, a Memory, the egress policy) has exactly one authoritative
   module. No duplicate definitions, no parallel half-implementations.
4. **Public interfaces are small and documented.** A crate's `lib.rs` re-exports its public surface with doc
   comments. Internals stay private. An agent should grok a crate from its `lib.rs` + this file.
5. **Generated contracts, not tribal knowledge.** The Rust↔TS boundary is generated; the schema is in code; the
   "why" is in ADRs. Nothing critical lives only in someone's head.
6. **Small, focused files.** A file that's grown to do many things is a signal to split along the domain seam.
   Agents (and humans) reason better about files they can hold in context at once.
7. **Tests as executable specification.** The fastest way for an agent to understand `assemble_context` is its
   tests. Keep them readable and behavior-focused.
8. **Leave seams, not scaffolding.** Stable IDs, soft-delete, the change-log, the provider traits — these are
   cheap now and make the known futures (sync, new modules, new providers) additive instead of disruptive.
9. **When you change a rule, change this file.** `AGENTS.md` is the contract. If reality and `AGENTS.md` disagree,
   one of them is a bug — and usually it's worth fixing both in the same PR.

---

## Appendix A — Tech baseline (current stable, June 2026)

> Pin these. Bumping a major (especially Vite/TS/Tauri) is a deliberate task with its own PR, not a silent `^`.
> The scaffold currently ships **Vite 6 / TS 5.6** — an early task is to bump to the baseline below.

**Frontend**

| Package | Version |
|---|---|
| solid-js | `1.9.13` |
| @solidjs/router | `0.16.1` |
| @tauri-apps/api | `2.11.0` |
| @tauri-apps/cli | `2.11.2` |
| @tiptap/core · /pm · /starter-kit | `3.25.0` |
| solid-tiptap | `0.8.0` |
| @tanstack/solid-query | `5.101.0` |
| vite | `8.0.16` |
| vite-plugin-solid | `2.11.12` |
| typescript | `6.0.3` |
| vitest | `4.1.8` |

**Backend (Rust)**

| Crate | Version | Role |
|---|---|---|
| tauri / tauri-build | `2.11.2` | desktop shell |
| rusqlite (`bundled`) | `0.35.0` | SQLite + FTS5 (pinned: `0.40` pulls a `libsqlite3-sys` build script using unstable `cfg_select`, which fails on stable Rust 1.93 — revisit when stabilized) |
| `sqlite-vec` | (bundled extension) | vector search |
| tokio-rusqlite / r2d2_sqlite | `0.7.0` / `0.34.0` | async/pool access |
| refinery | `0.9.1` | migrations |
| fastembed | `5.15.0` | local ONNX embeddings (+ reranker) |
| candle-core / mistralrs | `0.10.2` / `0.8.1` | optional embedded LLM |
| reqwest | `0.13.4` | cloud provider HTTP |
| tauri-specta / specta / specta-typescript | `2.x` RC / `1.0.5` / `0.0.12` | IPC type generation (or `ts-rs` as stable fallback) |
| serde / serde_json | `1.0.228` | serialization |
| tokio | `1.52.3` | async runtime |
| uuid (`v7`) | `1.23.2` | stable sortable IDs |
| jiff | `0.2.28` | datetime |
| thiserror | `2.0.18` | typed errors |
| async-trait | `0.1.89` | async port traits |
| tracing / tracing-subscriber | `0.1.44` / `0.3.23` | observability |

> **Version policy:** when adding or upgrading a dependency, confirm the current stable release first
> (the user's environment provides the `ctx7` CLI / Context7 for this) and record any major upgrade as an ADR.

## Appendix B — Glossary (domain language)

- **Block** — a ProseMirror node with a stable ID; the atomic unit of editing, chunking, and block-linking.
- **Chunk** — a retrieval unit derived from one or more blocks; what gets embedded and indexed.
- **Memory** — an AI-derived atomic fact/preference/entity with provenance, confidence, and lifecycle state.
- **Entity / Link** — nodes and edges of the cross-module knowledge graph (notes ↔ tasks ↔ finance ↔ …).
- **Provider** — a swappable source of embeddings or completions (local or cloud), behind a domain port.
- **Egress** — any data that would leave the device; always assembled, approved, and logged before a cloud call.
- **AssembledContext** — the single, deterministic, token-budgeted bundle a model is allowed to see.
- **Port / Adapter** — a domain trait (port) and its concrete implementation (adapter); the basis of testability.
