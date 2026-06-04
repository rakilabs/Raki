# Vector Retrieval + Embeddings + Eval Harness — Design

**Date:** 2026-06-04
**Status:** Approved (design), revised after adversarial review
**Roadmap slice:** #1 of the Retrieval/Memory core priority, **split into 1a + 1b**

---

## Context & Motivation

Raki's differentiator is *retrieval and memory quality*, not feature count. The
foundation ships notes CRUD; the previous slice shipped FTS5 keyword search. This
slice adds **semantic vector retrieval** (the second half of hybrid search) and an
**evaluation harness**, so retrieval quality becomes measurable before the
architecture calcifies around vibes.

### Honest framing of what the eval harness is (and is not)

A small, hand-curated golden set is a **harness bootstrap and a coarse regression
tripwire** — *not* proof of retrieval quality, and *not* statistically meaningful
at 15–30 queries.

- **Determinism buys a gate, not validity.** A stable golden set makes
  "did this change make retrieval worse?" answerable. It does *nothing* to prove
  the labels are correct, representative, or hard to game. Deterministic garbage is
  still garbage.
- **Statistical significance is unachievable at bootstrap scale**, so we do not
  claim it. The set earns statistical meaning later, when synthetic expansion grows
  it to hundreds (ADR 0005). Until then it is a tripwire that catches gross
  regressions and exposes *per-category failure patterns* — which is its real job.
- What makes a tiny set defensible is **taxonomy, not size**: 30 well-distributed,
  category-tagged, adversarial queries beat 300 clean ones, because per-category
  breakdown turns "is retrieval good?" (unanswerable now) into "where does each
  method fail?" (answerable, actionable).

> We still ship eval-first — vector search does not land *untuned and unmeasured* —
> but v1 eval is a bootstrap, not a quality verdict.

## Goals

1. A real `EmbeddingProvider` (fastembed, `bge-small-en-v1.5`, 384-dim) replacing
   `FakeEmbeddingProvider` as the runtime default.
2. A real `VectorIndex` (sqlite-vec `vec0`, same `raki.sqlite`) with correct
   write/delete/race semantics.
3. A decoupled, idempotent embedding pipeline keyed on a content hash, with
   compare-and-stamp race protection and defined operational semantics.
4. A taxonomy-driven eval harness: a versioned golden set, metrics
   (recall@k + MAP@k; nDCG only once graded), a regression gate, a report binary.

## Non-Goals (named deferrals, not gaps)

- **RRF fusion of keyword + vector** → roadmap slice **#2**. This slice measures
  keyword-only *and* vector-only baselines so #2 can *prove* fusion beats both.
  (Fusion also closes the vector-coverage gap — see Risks — so it should follow
  soon.)
- **Chunking** → deferred, but **falsifiably**: the eval taxonomy *must* include
  buried-fact-in-long-note cases; if vector recall on them is poor, chunking is the
  next slice. The deferral is a tested hypothesis, not a bet.
- **Multilingual model** → *not* a one-line swap (see Risks). Deferred behind the
  port, with the real blast radius documented.
- **Synthetic / implicit-feedback labels** → later sources, with strict provenance
  separation from judged labels (ADR 0005).
- **Generation/RAG answer quality (Tier-2 eval)** → arrives with the RAG loop (#5).
- **Ollama / cloud embedding adapters** → opt-in providers in a later slice.

---

## Slice split

The original single slice was a pile of integration risk (native dep + virtual
table + migration + pipeline + races + deletion + taxonomy + metrics + gate +
report). Split into two back-to-back plans; eval still precedes any tuning/fusion.

- **Slice 1a — Vector mechanism.** `FastEmbedProvider`, `SqliteVectorIndex`,
  migration V3, the embedding pipeline (content hash, compare-and-stamp, deletion,
  operational semantics), app wiring. Verified with mechanism tests (fake embedder)
  plus a couple of real-model smoke checks.
- **Slice 1b — Eval harness.** Query taxonomy + golden set, metrics runner,
  regression gate (real model, tiny corpus), `eval-report` binary, ADR 0005. Built
  on the now-stable 1a mechanism.

---

## Key Decisions

| # | Decision | Rationale |
|---|----------|-----------|
| D1 | **Eval-first, but honest**: vector search ships measured; v1 eval is a bootstrap/tripwire, not a quality verdict | Effectiveness must be falsifiable early; overclaiming validity is its own failure. |
| D2 | Default model **fastembed `bge-small-en-v1.5` (384-dim)** | In-process ONNX, no separate service; crate default; menu of models for A/B. Product assumptions made explicit (Risks). |
| D3 | **Decoupled, content-hash-keyed** pipeline with **compare-and-stamp** | Instant save; one `embed_pending()` mechanism everywhere; CAS prevents stamping stale content as current. |
| D4 | **Whole-note embedding v1, falsifiable deferral of chunking** | Minimal baseline, but the eval taxonomy must include buried-fact-in-long-note cases that can *disprove* it. |
| D5 | **Fusion deferred to #2**; this slice = vector-only + baselines | Lets the harness show fused > max(keyword, vector) as a *measured* claim. |
| D6 | Eval set = **taxonomy-driven, versioned artifact** seeded by hand labels | Format + metrics are durable; label *validity* is a separate, explicitly-bounded claim. |
| D7 | **Split into 1a (mechanism) + 1b (eval)** | Lower integration risk per plan; eval still precedes tuning/fusion. |
| D8 | Gate floors **recall@k AND MAP@k**; nDCG only with graded labels | Recall-only lets ranking rot while green; MAP handles multi-relevant; binary-nDCG is a fake signal. |
| D9 | **CI: fake embedder for mechanism tests, real model for the quality gate** | Fake gate tests plumbing not quality; real gate is the point. Split by test layer. |

---

## Architecture

Hexagonal, following the existing crate layout. New code slots behind the ports
already defined in `raki-domain` (`EmbeddingProvider`, `VectorIndex`).

| Crate | Addition | Slice |
|-------|----------|-------|
| `raki-ai` | `FastEmbedProvider` (new dep: `fastembed`); `FakeEmbeddingProvider` retained for tests | 1a |
| `raki-storage` | `SqliteVectorIndex` (sqlite-vec `vec0`); migration **V3**; extension registration; vector delete on soft-delete | 1a |
| app (`src-tauri`) | wire both providers into `AppState`; embedding pipeline orchestration (startup + post-save passes, single-flight); `eval-report` bin (1b) | 1a/1b |
| `raki-retrieval` | vector search path; `eval/` taxonomy + fixtures; metrics runner; gate | 1a/1b |

### Embedding pipeline (D3) — Slice 1a

1. `upsert` writes `notes` + `notes_fts` synchronously (unchanged — save stays
   instant, note keyword-searchable immediately) and records `content_hash`.
2. **Staleness:** a live note needs (re)embedding iff `embedded_hash IS NULL`,
   `embedded_hash != content_hash`, or `embedded_model != <current model id>`.
3. `embed_pending()`:
   - selects stale live notes, capturing each note's `content_hash` as
     `embedded_target`;
   - runs `EmbeddingProvider::embed` (outside any write lock);
   - upserts the vector into `note_vectors` and **compare-and-stamps**:
     `UPDATE notes SET embedded_hash = :embedded_target, embedded_model = :model
      WHERE id = :id AND content_hash = :embedded_target`.
     If the note changed mid-flight, the guard fails, the note stays stale, and it
     re-embeds next pass. **No stamping old content as current.**
4. Reused for: first-index, post-save (fire-and-forget task), model-swap re-embed,
   eval corpus re-embedding.

**Operational semantics (D3):**
- **Crash/close recovery is free**: anything unembedded is, by definition, still
  stale and picked up on the next pass. The decoupling *is* the durability story.
- **Startup catch-up pass** runs `embed_pending()` on launch.
- **Single-flight guard**: at most one pass runs at a time (a mutex/flag).
- **Per-note failure isolation**: one failing note does not poison the batch; it is
  logged and left stale for bounded-backoff retry.

### Deletion (D3) — Slice 1a

`soft_delete` (and any future hard delete) **must remove the vector in the same
transaction**, mirroring the FTS5 pattern we already ship:

```sql
DELETE FROM note_vectors WHERE note_id = ?1;
```

Otherwise deleted notes leave orphan vectors → phantom hits. In `Definition of
Done`.

### Content hash (D3) — Slice 1a (specified, not "an implementation detail")

`content_hash` = a stable hash over **only the embeddable text** — `title` + `body`
— with:
- Unicode **NFC** normalization;
- whitespace collapsed/trimmed;
- explicit **exclusion** of volatile fields (`created_at`, `updated_at`, `version`,
  `id`, `deleted_at`).

A wrong hash silently breaks the cache in one of two directions (never re-embed →
stale vectors; always re-embed → wasted compute), so the exact function is pinned
in the 1a plan, not left loose.

### Vector storage (D2) — sqlite-vec, ctx7-verified — Slice 1a

Register once before opening connections (global auto-extension; no runtime `.so`
loading; works with bundled rusqlite):

```rust
use sqlite_vec::sqlite3_vec_init;
use rusqlite::ffi::sqlite3_auto_extension;
unsafe { sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ()))); }
```

vec0 table (migration V3):

```sql
CREATE VIRTUAL TABLE note_vectors USING vec0(
    note_id TEXT PRIMARY KEY,
    embedding float[384]
);
```

KNN (vectors as little-endian `f32` bytes via `zerocopy`):

```sql
SELECT note_id, distance
FROM note_vectors
WHERE embedding MATCH ?1 AND k = ?2
ORDER BY distance;
```

> ctx7 retired the **API/registration risk** (the pattern compiles against
> rusqlite). It did **not** retire **build/packaging risk** — `ort`/ONNX across the
> Tauri target matrix. The 1a plan includes a build/packaging spike before wiring.

### Embedding provider (D2) — fastembed, ctx7-verified — Slice 1a

```rust
use fastembed::{TextEmbedding, InitOptions, EmbeddingModel};
let model = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::BGESmallENV15))?;
let embeddings = model.embed(documents, None)?; // Vec<Vec<f32>>, dim 384
```

`dimension() -> 384`, `locality() -> Local`. **Asymmetric prefixing**: bge models
benefit from a query instruction prefix on the *query* side only; the 1a plan
encodes a documented prefix policy and the harness (1b) arbitrates whether it helps.

---

## Data Model (Migration V3) — Slice 1a

Append-only (never edit a shipped migration — `AGENT.md §7`):

1. `note_vectors` vec0 table (above).
2. Staleness tracking on `notes`:

   ```sql
   ALTER TABLE notes ADD COLUMN content_hash TEXT;
   ALTER TABLE notes ADD COLUMN embedded_hash TEXT;
   ALTER TABLE notes ADD COLUMN embedded_model TEXT;
   ```

   Backfill `content_hash` for existing rows (so they're picked up by the first
   pass); `embedded_hash`/`embedded_model` start NULL → stale → embedded next pass.

---

## Eval Harness (D1/D6/D8) — Slice 1b

A taxonomy-driven, versioned artifact. The *source* of labels is pluggable; the
*format and metrics* are the durable contract. Label **validity** is a separate,
bounded claim — not implied by schema stability.

### Query taxonomy (the centerpiece — fairness + adversarial coverage)

Every query is tagged with a category; the report breaks metrics down **per
category**. This is what makes keyword-vs-vector comparison fair (otherwise it just
measures which style the author favored) and what gives the tiny set teeth:

| Category | Purpose |
|----------|---------|
| `lexical-overlap` | keyword should win; sanity that FTS isn't broken |
| `semantic-paraphrase` | vector should win; the reason vectors exist |
| `buried-fact-in-long-note` | **mandatory** — the falsifiable chunking test (D4) |
| `multi-relevant` | several correct notes; exercises MAP, not just first-hit |
| `negative / no-relevant` | precision / false-positive control |
| `messy` | typos, fragments, half-remembered wording (real-user realism) |
| `named-entity` | exact names/terms |
| `temporal` | time-referenced recall |

Hand-authored queries are deliberately *not* kept clean; `messy` and `negative`
categories are required, not optional.

### Golden set (seed = hand-curated)

```
src-tauri/crates/raki-retrieval/eval/
  corpus.json    # [{ id, title, body }]                          seed notes
  queries.json   # [{ query, category, relevant_note_ids[], grade? }]
```

`grade?` is optional graded relevance (0–3); absent ⇒ binary. Later label sources
pour into the *same* schema (synthetic after #4; behavioral after the UI is used).

### Metrics & gate (D8)

- **recall@k** — fraction of relevant notes in top-k.
- **MAP@k** — mean average precision; accounts for *all* relevant ranks (fixes
  MRR's first-hit-only brittleness for multi-relevant memory retrieval).
- **MRR** — secondary "time-to-first-relevant" signal, reported not gated.
- **nDCG@k** — computed **only** where `grade` is present; never over binary labels
  (that would be a fake rank-quality signal).

**Regression gate** (`tests/eval_gate.rs`): floors **both recall@k and MAP@k**, so
ranking can't rot while green. Uses the **real model** on the tiny corpus (D9),
model cached in CI keyed on model id, run as a slower tagged integration test.

**Report** (`cargo run --bin eval-report`): keyword-vs-vector table, broken down
per taxonomy category — the artifact you read while tuning.

### ADR 0005 — Slice 1b

"Retrieval quality is measured, not vibed." Captures: eval set is a versioned
artifact with a stable schema; metrics are the bar; and **label provenance is
tiered** — *judged labels* (trusted ground truth: hand, then synthetic-verified)
are kept strictly separate from *behavioral signals* (opened-result telemetry —
biased, position/UI-dependent, used only as untrusted candidate hints, never merged
as equal-trust labels).

---

## Risks (surfaced, not hidden as implementation details)

- **Model choice carries product assumptions**, not just implementation ones:
  English-centric, short-text bias, ONNX viability on target platforms, install/
  download size, acceptable first-run behavior, runtime stability, sufficient
  quality. These are stated so they can be challenged, not buried.
- **Model swap is a migration, not a line.** Changing the model changes: vector
  dimension (→ `vec0(float[N])` schema change), cached embeddings (full re-embed —
  handled by the staleness model), score distributions/thresholds, query prefixing,
  binary size, runtime perf, and eval labels. The port abstracts the *call site*,
  not the *system impact*.
- **First-run model download** is a real UX/reliability surface ("no separate
  service" ≠ "no network"). Handling: download-on-first-use with progress; on
  failure or offline, **degrade to keyword-only** (already available); option to
  pre-bundle the model. The 1a plan defines cache location + failure behavior.
- **sqlite-vec build/packaging** across the Tauri target matrix (`ort`/ONNX native
  runtime) — a 1a plan spike before wiring.
- **Vector-coverage gap**: vector-only search silently misses a just-saved note
  until the next pass. Mitigated by eager + startup passes; *closed* by fusion (#2),
  since keyword covers the gap — a reason to do #2 soon. Documented as a known
  limitation of the vector-only slice.

---

## Success Criteria / Definition of Done

**Slice 1a (mechanism):**
- `embed_pending()` embeds stale live notes idempotently; re-running is a no-op.
- Compare-and-stamp: a note edited between select and stamp stays stale (no stale
  vector marked current) — covered by a concurrency test.
- `soft_delete` removes the vector in the same transaction (no orphan vectors) —
  covered by a test.
- Startup pass + single-flight guard + per-note failure isolation present.
- A note saved → keyword-searchable immediately, vector-searchable after a pass.
- Changing the model id re-embeds the whole corpus.
- Mechanism tests use the fake embedder; a couple of real-model smoke checks pass.
- `cargo test --workspace`, `clippy --workspace --all-targets -D warnings`,
  `fmt --check` green.

**Slice 1b (eval):**
- Golden set covers all taxonomy categories, including the mandatory
  buried-fact-in-long-note cases.
- `eval-report` prints keyword-vs-vector metrics per category.
- `tests/eval_gate.rs` floors recall@k **and** MAP@k with documented values, using
  the real model on the tiny corpus.
- ADR 0005 committed.
- Manual `tauri dev` smoke deferred to user (`verification-before-completion`).

## Open Questions (resolved in the plans, not blockers)

- Exact `content_hash` function (fields fixed above; algorithm TBD in 1a).
- Concrete gate floor values for recall@k / MAP@k (set from the first real-model
  report in 1b).
- Query-prefix policy for bge (encode in 1a; eval arbitrates in 1b).
