# R4 — Memory Lifecycle Signals Design

> **Status:** Implemented — R4.1 code, corpus, harness, and ADR-0009 are complete; production attach is blocked on real-model evaluation per ADR-0009.  
> **Date:** 2026-06-10  
> **Depends on:** ADR-0005, ADR-0006, ADR-0007, R0–R3 complete  
> **Roadmap:** Track A — R4.1 + R4-corpus + R4.2  
> **Companion ADR:** ADR-0009 — Memory-lifecycle ranking signals (to be written)

## 1. User stories

| Who | Job-to-be-done | Signal that serves it | Target outcome |
|---|---|---|---|
| Trip planner | "When I search 'japan trip', I want my 2025 itinerary above my 2023 one." | Recency | The more recently touched relevant note outranks an older, similarly titled note. |
| Project owner | "When I search 'project plan', I want my pinned Q3 plan above random notes that mention 'project'." | Pin | An explicitly pinned note outranks incidental keyword matches. |
| Returning reader | "When I search 'doctor', I want the health note I keep reopening, not one I wrote once." | Salience | Frequently accessed notes get a modest boost over rarely touched ones. |

## 2. Goal

Grow `raki-memory` beyond context assembly by adding **memory-lifecycle signals** that influence ranking: a second brain should know what the user has recently touched, what they have explicitly pinned, and what they access frequently.

R4 is split into three slices so engineering can proceed while the measurement gate (ADR-0007) is honored:

| Slice | Purpose | Exit |
|---|---|---|
| **R4.1** — Signal model & mixer | Build the signal data model, multiplicative mixer, storage seams, and scored-retrieval primitive. | Deterministic unit + storage tests green; mixer exists but is **not** the production default. |
| **R4-corpus** — Synthetic real-notes collection | Hand-author an anonymized seed corpus + query/relevance set so signals have something measurable to rescue. | A reproducible eval corpus where pure retrieval fails and memory signals lift ranking. |
| **R4.2** — Measure & attach decision | Run the mixer against the corpus and decide attach / tune / delete per ADR-0007, including per-signal ablations. | Measured lift on Success@3 / MAP@3; production wiring follows measurement, not before. |

## 3. Non-goals

- **No link-density signal in R4.1.** Raki has no link graph in Phase 1 (cross-module linking is Phase 2 T2). Link-density becomes a follow-up lever once `[[wiki-links]]` or task links exist.
- **No tag-affinity signal in R4.1.** Notes do not yet carry tags. Tag affinity is explicitly reserved for the next iteration.
- **No explicit pin UI in R4.1.** The `pinned` column is set via test seams and future settings commands; no editor toggle ships until R4.2 measurement justifies it.
- **No procedural generator in R4-corpus.** A procedural scale generator may follow after R4.2; the gate depends only on the hand-authored seed corpus.
- **No silent production wiring.** The mixer is only attached to live `hybrid_search` after R4.2 proves lift.

## 4. Principles

1. **Measurement gates attachment.** R4.1 is a buildable library feature; R4.2 is the honest verdict.
2. **Dependency rule is preserved.** The signal-math contract lives in `raki-domain` as a port (`SignalBooster`). `raki-retrieval` calls it through the trait; `raki-memory` may provide a default implementation but is not a direct dependency of `raki-retrieval`.
3. **Domain-first, IO-second.** Signal math is pure. Storage and retrieval only call it through traits.
4. **Multiplicative boosts preserve retrieval semantics.** A strong retrieval match is not dropped because it is old; a salient recent note can overtake a weakly relevant one.
5. **Synthetic corpus is a scaffold, not a replacement for real ground truth.** It lets us build and unit-test the mixer now; the real-data tier still matures via P1 dogfooding.
6. **Behavioral signals are local-only, user-owned, and deletable.** They never leave the device and can be cleared by the user.

## 5. Architecture

```
raki-domain
  ├─ SignalSource trait (port): read signals for a set of note ids
  ├─ SignalStore trait (port): write signals (record view, touch, pin)
  └─ SignalBooster trait (port): pure fn (retrieval_score, signals, config, now_ms) → boosted_score + breakdown

raki-storage
  ├─ SqliteSignalSource: reads notes.pinned + note_signals.*
  └─ SqliteSignalStore: atomic, idempotent, tombstone-aware writes

raki-memory
  └─ DefaultSignalBooster: implements the multiplicative mixer + SignalBreakdown

raki-retrieval
  ├─ hybrid_candidates_scored: returns (note_id, retrieval_score) pairs
  └─ hybrid_search_with_signals: optional booster applied after recall ordering, before rerank/context assembly
```

New files:
- `raki-domain/src/ports.rs` (extend) — `SignalSource`, `SignalStore`, `SignalBooster` async traits + value types.
- `raki-memory/src/signals.rs` — `NoteSignals`, `MixerConfig`, `DefaultSignalBooster`, `SignalBreakdown`.
- `raki-storage/src/signals.rs` — `SqliteSignalSource`, `SqliteSignalStore`.
- `raki-eval/src/memory_corpus.rs` (new) — hand-authored seed corpus + query/relevance set.

## 6. Pipeline composition

The retrieval stage order is unchanged through R3. R4 inserts one optional stage:

```
query
  → (R3) query understanding / rewrite [optional]
  → (R0/R2) BM25 recall ∪ vector KNN recall
  → (R0) vector-primary ordering + keyword backfill → per-note retrieval_score derived from rank
  → (R4) signal boost: retrieval_score × memory_boost
  → (R1) cross-encoder rerank [optional, attach-to-validate]
  → context assembly
```

- Signal boosting applies to **per-note aggregate scores** after the recall union, not to per-chunk scores. Each note’s score is rolled up from its chunks before boosting.
- `retrieval_score` is derived from the vector-primary rank: `1.0 / (1 + rank)`, so the first vector result scores `1.0`, the second `0.5`, etc.; keyword-backfilled ids continue the decay after the vector block.
- The reranker (R1) still sees the boosted ordering as its input pool. If R4.2 shows reranker interactions hurt lift, we re-evaluate stage order in a follow-up.

## 7. Storage changes (Migration V8)

```sql
-- User-controlled pin flag. Lives on notes so it inherits sync-ready row conventions.
ALTER TABLE notes ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;

-- Aggregated access statistics. Follows ADR-0002 row conventions.
CREATE TABLE note_signals (
    id TEXT PRIMARY KEY,
    note_id TEXT NOT NULL UNIQUE,
    view_count INTEGER NOT NULL DEFAULT 0,
    last_accessed_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    deleted_at INTEGER,
    version INTEGER NOT NULL,
    FOREIGN KEY (note_id) REFERENCES notes(id) ON DELETE CASCADE
) STRICT;

CREATE INDEX idx_note_signals_note_id ON note_signals(note_id);
CREATE INDEX idx_note_signals_last_accessed ON note_signals(last_accessed_at) WHERE deleted_at IS NULL;
```

- `note_signals` is **one row per note**. Missing rows have defined semantics: recency falls back to `notes.updated_at`, pin is `false`, salience is `0`.
- `deleted_at` is set when the parent note is soft-deleted. `SqliteSignalStore::record_view` rejects deleted notes.
- `ON DELETE CASCADE` ensures hard-deleted notes also lose signal state, but soft-delete is explicit so resurrected notes can recover their history if desired.
- `version` supports future optimistic concurrency / sync.

## 8. Domain ports

```rust
// raki-domain/src/ports.rs
#[async_trait]
pub trait SignalSource: Send + Sync {
    async fn get(&self, note_ids: &[NoteId]) -> Result<HashMap<NoteId, NoteSignals>, DomainError>;
}

#[async_trait]
pub trait SignalStore: Send + Sync {
    /// Idempotent: increments view_count and updates last_accessed_at, but only for live notes.
    async fn record_view(&self, note_id: &NoteId, now_ms: i64) -> Result<(), DomainError>;
    /// Update last_accessed_at without incrementing view_count (used on edit).
    async fn touch(&self, note_id: &NoteId, now_ms: i64) -> Result<(), DomainError>;
}

pub struct SignalBreakdown {
    pub recency: f64,
    pub pin: f64,
    pub salience: f64,
    pub raw_boost: f64,
    pub capped_boost: f64,
}

#[async_trait]
pub trait SignalBooster: Send + Sync {
    fn boost(&self, retrieval_score: f64, signals: &NoteSignals, now_ms: i64) -> (f64, SignalBreakdown);
}
```

## 9. Scored retrieval primitive

`hybrid_search` currently returns `Vec<NoteId>`. R4 introduces a scored primitive:

```rust
// raki-retrieval
pub struct ScoredNote {
    pub note_id: NoteId,
    pub retrieval_score: f64,
}

pub async fn hybrid_candidates_scored(
    keyword_index: &dyn KeywordIndex,
    vector_index: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    rewriter: Option<&dyn QueryRewriter>,
    query: &str,
    top_k: usize,
) -> Result<Vec<ScoredNote>, RetrievalError>;
```

- `retrieval_score` is derived from the vector-primary recall rank: `1.0 / (1 + rank)`.
- `hybrid_search` can remain as a thin wrapper that drops scores, or be replaced by callers that need scores.
- `hybrid_search_with_signals` calls `hybrid_candidates_scored`, loads signals, applies `SignalBooster`, and returns ranked note ids.

## 10. Signal math

```rust
pub struct MixerConfig {
    pub half_life_days: f64,   // e.g. 7.0
    pub pin_boost: f64,        // e.g. 0.25
    pub salience_weight: f64,  // e.g. 0.15
    pub max_boost: f64,        // safety cap, e.g. 2.0
}

impl MixerConfig {
    pub fn new(half_life_days: f64, pin_boost: f64, salience_weight: f64, max_boost: f64) -> Result<Self, DomainError> {
        if !half_life_days.is_finite() || half_life_days <= 0.0 {
            return Err(DomainError::Invalid("half_life_days must be positive and finite".into()));
        }
        if !pin_boost.is_finite() || pin_boost < 0.0 {
            return Err(DomainError::Invalid("pin_boost must be non-negative and finite".into()));
        }
        if !salience_weight.is_finite() || salience_weight < 0.0 {
            return Err(DomainError::Invalid("salience_weight must be non-negative and finite".into()));
        }
        if !max_boost.is_finite() || max_boost < 1.0 {
            return Err(DomainError::Invalid("max_boost must be >= 1.0 and finite".into()));
        }
        Ok(Self { half_life_days, pin_boost, salience_weight, max_boost })
    }
}

pub struct NoteSignals {
    pub pinned: bool,
    pub view_count: u64,
    pub last_accessed_at_ms: Option<i64>,
}

fn memory_boost(signals: &NoteSignals, now_ms: i64, cfg: &MixerConfig) -> SignalBreakdown {
    let elapsed_days = signals.last_accessed_at_ms.map(|t| {
        let ms = (now_ms - t).max(0);
        ms as f64 / 86_400_000.0
    });
    let recency = elapsed_days.map(|d| 2.0_f64.powf(-d / cfg.half_life_days)).unwrap_or(0.0);

    let pin = if signals.pinned { 1.0 } else { 0.0 };

    // Normalize view_count: 0 → 0, 9+ → ~1.0, clamped to [0, 1].
    let salience = ((1.0 + signals.view_count as f64).ln() / 10.0_f64.ln()).clamp(0.0, 1.0);

    let raw = 1.0 + recency + cfg.pin_boost * pin + cfg.salience_weight * salience;
    let capped = raw.min(cfg.max_boost);

    SignalBreakdown {
        recency,
        pin: cfg.pin_boost * pin,
        salience: cfg.salience_weight * salience,
        raw_boost: raw,
        capped_boost: capped,
    }
}
```

Final score:

```rust
let (boosted, breakdown) = booster.boost(retrieval_score, signals, now_ms);
// retrieval_score > 0; boosted_score > retrieval_score only when boost > 1.
```

- All inputs validated at `MixerConfig` construction.
- Negative elapsed time is clamped to zero.
- `SignalBreakdown` is recorded in eval artifacts for debuggability.

## 11. Frontend contract for `record_note_view`

```rust
#[tauri::command]
#[specta::specta]
pub async fn record_note_view(
    state: tauri::State<'_, AppState>,
    input: RecordNoteViewInput,
) -> Result<(), AppError> {
    // Delegates to SignalStore port.
}

pub struct RecordNoteViewInput {
    pub note_id: String,
}
```

- Fires **once per note session** when the user opens a note and it remains the active note for **≥ 2 seconds**.
- Does **not** fire for search previews, list hovers, background indexing reads, context assembly reads, or tab switches < 2s.
- Debounced: multiple rapid opens of the same note within a 5-second window count as one view.
- Backend validates that the note exists and `deleted_at IS NULL`; returns `AppError::NotFound` otherwise.
- Per-session rate limit: max one recorded view per note per minute (enforced in storage).
- The frontend does not increment view count optimistically; it awaits the command result.

## 12. Privacy / retention / export policy

- **Local-only:** `note_signals` never leaves the device and is never included in egress logs.
- **User-cleared:** A future settings command will allow the user to clear all `note_signals` history (reset view_count and last_accessed_at) or per-note history.
- **Retention:** Signal history persists until the parent note is hard-deleted or the user clears it. There is no automatic expiry in R4.1.
- **Export:** `note_signals` is **excluded** from Markdown exports. JSON export may include it optionally and explicitly (default off).
- **Backup:** Because the table is in the same SQLite file as notes, it travels with the user’s single-file backup. The user is informed that behavioral signals are part of that backup.
- **Soft-delete:** When a note is trashed, its signal row is marked `deleted_at = now`; it is not visible to ranking or settings until restored.

## 13. Corpus design

### 13.1 Hand-authored seed corpus

- **~25 notes**, all explicitly synthetic and anonymized. No real names, addresses, account numbers, or medical identifiers.
- Domains: projects, people (fictional), trips, finances (rounded/fake figures), health (generic symptoms), ideas, recipes, books.
- **Intentional ambiguity:** similar titles such as “Japan Trip 2023” and “Japan Trip 2025”.
- **Mixed access patterns:** some notes recent + high view count, some old + untouched.
- **Pinned anchors:** a few notes explicitly pinned (e.g., “Q3 Project Plan”).
- **Buried facts:** older notes that contain the correct answer but would be outranked by newer, less relevant notes under pure retrieval.
- Committed with a `DATA_PROVENANCE.md` stating: all content is synthetic, no PII, generated by hand for evaluation.

### 13.2 Query / relevance set

~20 labeled queries, each with one or more expected note ids. Examples:

| Query | Expected | Why signals matter |
|---|---|---|
| “japan trip budget” | Japan Trip 2025 | Disambiguates from Japan Trip 2023 via recency. |
| “project plan” | Q3 Project Plan | Pinned note beats notes that merely mention “project”. |
| “doctor’s advice” | Health Checkup 2025 | Recent health note beats older one. |

### 13.3 Baseline pinning

The pure-retrieval baseline recorded in R4-corpus must carry this signature:

- Provider: `kimi`
- Model: `kimi-k2-5`
- Embedding model: `bge-small-en-v1.5` (or current active) + model version
- Chunking algorithm version: current `body_to_blocks` + `cap_split` at 1600 chars / max 32 chunks
- Retrieval ordering: vector-primary + keyword backfill
- Retrieval-score derivation: `1.0 / (1 + rank)`
- Seed corpus commit hash

Regenerate the baseline whenever: provider, embedding model, chunker, retrieval ordering/scoring, or seed corpus changes.

## 14. Testing strategy

| Layer | What | How |
|---|---|---|
| `raki-memory` | Mixer math + validation + breakdown | Unit tests with fixed `NoteSignals` and known boost outputs; boundary/invalid-input tests. |
| `raki-storage` | Migration V8 + `SqliteSignalSource`/`SqliteSignalStore` | Integration test on populated temp DB; tombstone/soft-delete/atomicity tests. |
| `raki-retrieval` | Scored primitive + signal boost | Eval test comparing `hybrid_candidates_scored` baseline vs `hybrid_search_with_signals` on the corpus. |
| `raki-app` | `record_note_view` command | Thin command test mapping DTO to `SignalStore`. |
| CI | Deterministic suite | `cargo test`, `clippy -D warnings`, `fmt --check`. No live models. |

## 15. Exit criteria

### R4.1 — Signal model & mixer
- `SignalBooster`, `SignalSource`, `SignalStore` ports defined in `raki-domain`.
- `DefaultSignalBooster` + `SignalBreakdown` implemented in `raki-memory`.
- `SqliteSignalSource` + `SqliteSignalStore` implemented in `raki-storage`.
- `hybrid_candidates_scored` exists in `raki-retrieval`.
- Migration V8 applied and tested on populated DB.
- Deterministic suite green.

### R4-corpus
- Hand-authored seed corpus (~25 notes + ~20 queries) committed under `raki-eval`.
- `DATA_PROVENANCE.md` confirms synthetic, anonymized content.
- Pure-retrieval baseline recorded with full configuration signature.

### R4.2 — Measure & attach decision
Attach the mixer to production `hybrid_search` **only if all** of the following hold on the corpus:

1. Paired mean **Success@3** improves by **≥ 0.05 absolute** over the pure-retrieval baseline.
2. **MAP@3** does not regress by more than **0.02 absolute**.
3. Improvement is significant via paired permutation test with **p < 0.05** across **5 independent runs**.
4. **Per-signal ablation** shows each enabled signal contributes non-negative lift; any signal that drags overall lift is disabled by default.

If the first eval misses, allow at most **two tuning iterations** (adjust `MixerConfig` weights or signal definitions). Otherwise remove the mixer from the production path and record the decision in ADR-0009.

## 16. ADR

Write **ADR-0009 — Memory-lifecycle ranking signals** documenting:
- Why signals are multiplicative rather than additive.
- Why views are aggregated (not per-event logged).
- Why signal math lives in `raki-domain` as a port.
- Why R4 is split into R4.1 / R4-corpus / R4.2.
- Alternatives considered (additive mixer, link-density, tag-affinity, recency-only).
- Privacy and retention policy.

## 17. Decisions

1. **View counting is explicit.** A new Tauri command `record_note_view(note_id)` is called by the frontend when the user opens a note and it remains active for ≥ 2 seconds. Background reads do not increment `view_count`.
2. **Pin UI is deferred.** The `pinned` column is set via test seams and future settings commands in R4.1; no editor toggle ships until R4.2 measurement justifies it.
3. **Signal boosting applies after the recall union, at note-level.** Chunk scores are rolled up before signals are applied.
4. **Behavioral signals are local-only and user-cleared.** They are never transmitted, never exported by default, and can be cleared by the user.

## 18. Follow-up levers (not R4)

- **Tag affinity:** when notes carry tags, boost notes sharing tags with recently viewed notes.
- **Explicit star / pin UI:** surface the `pinned` flag to the user.
- **Link-density:** when a link graph exists (Phase 2), boost highly connected notes.
- **Procedural corpus generator:** scale/stress harness after R4.2.
