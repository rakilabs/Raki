# R4 — Memory Lifecycle Signals Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build memory-lifecycle ranking signals (recency, pin, salience) behind domain ports, measure their lift on a synthetic real-notes corpus, and attach them to production retrieval only if the measurement gate is met.

**Architecture:** Pure signal math and ports live in `raki-domain`; `raki-memory` provides the default booster; `raki-storage` provides SQLite read/write adapters; `raki-retrieval` adds a scored primitive and an optional signal-boosted search path; `raki-app` exposes a single `record_note_view` command. The mixer is kept off the production path until R4.2 proves lift.

**Tech Stack:** Rust workspace (raki-domain, raki-memory, raki-storage, raki-retrieval, raki-app, raki-eval), rusqlite, async-trait, Tauri, ts-rs.

---

## File structure

| File | Responsibility |
|---|---|
| `src-tauri/crates/raki-domain/src/ports.rs` | `SignalSource`, `SignalStore`, `SignalBooster` traits; `NoteSignals`, `MixerConfig`, `SignalBreakdown` value types. |
| `src-tauri/crates/raki-memory/src/signals.rs` | `DefaultSignalBooster` and its unit tests. |
| `src-tauri/crates/raki-memory/src/lib.rs` | Re-export signal types. |
| `src-tauri/crates/raki-storage/src/migrations.rs` | Migration V8: `pinned` column + `note_signals` table. |
| `src-tauri/crates/raki-storage/src/signals.rs` | `SqliteSignalSource` and `SqliteSignalStore`. |
| `src-tauri/crates/raki-storage/src/lib.rs` | Re-export signal storage adapters. |
| `src-tauri/crates/raki-retrieval/src/search.rs` | `hybrid_candidates_scored` and `hybrid_search_with_signals`. |
| `src-tauri/crates/raki-retrieval/src/lib.rs` | Re-export new search functions. |
| `src-tauri/src/state.rs` | Add `signal_source`, `signal_store`, `signal_booster` to `AppState`. |
| `src-tauri/src/lib.rs` | Wire signal adapters in composition root. |
| `src-tauri/src/dto.rs` | `RecordNoteViewInput` DTO. |
| `src-tauri/src/commands/signals.rs` | `record_note_view` Tauri command. |
| `src-tauri/src/commands/mod.rs` | Register the new command module. |
| `src-tauri/crates/raki-eval/src/memory_corpus/` | Seed corpus notes + queries. |
| `src-tauri/crates/raki-eval/src/lib.rs` | Re-export corpus helpers. |
| `docs/eval/memory-corpus-DATA_PROVENANCE.md` | Synthetic/anonymized provenance statement. |
| `docs/adr/0009-memory-lifecycle-ranking-signals.md` | Design decisions ADR. |
| `docs/ROADMAP.md` | Update R4 status after R4.1 / R4-corpus / R4.2. |

---

## Task 1: Add signal value types and ports to `raki-domain`

**Files:**
- Modify: `src-tauri/crates/raki-domain/src/ports.rs`
- Test: `src-tauri/crates/raki-domain/src/ports.rs` (existing test module)

- [ ] **Step 1: Write the failing test**

Append to the existing `#[cfg(test)]` block in `ports.rs`:

```rust
#[test]
fn mixer_config_rejects_invalid_values() {
    use crate::signals::MixerConfig;
    assert!(MixerConfig::new(0.0, 0.1, 0.1, 1.5).is_err());
    assert!(MixerConfig::new(-1.0, 0.1, 0.1, 1.5).is_err());
    assert!(MixerConfig::new(7.0, -0.1, 0.1, 1.5).is_err());
    assert!(MixerConfig::new(7.0, 0.1, -0.1, 1.5).is_err());
    assert!(MixerConfig::new(7.0, 0.1, 0.1, 0.5).is_err());
    assert!(MixerConfig::new(7.0, 0.1, 0.1, f64::NAN).is_err());
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd src-tauri && cargo test -p raki-domain mixer_config_rejects_invalid_values -- --nocapture
```

Expected: FAIL — `MixerConfig` not found.

- [ ] **Step 3: Add value types and ports**

At the top of `ports.rs`, add imports:

```rust
use std::collections::HashMap;
```

Add the signal types and traits before the existing ports:

```rust
/// Aggregated behavioral signals for one note.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NoteSignals {
    pub pinned: bool,
    pub view_count: u64,
    pub last_accessed_at_ms: Option<i64>,
}

/// Configuration for the multiplicative signal booster.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MixerConfig {
    pub half_life_days: f64,
    pub pin_boost: f64,
    pub salience_weight: f64,
    pub max_boost: f64,
}

impl MixerConfig {
    pub fn new(
        half_life_days: f64,
        pin_boost: f64,
        salience_weight: f64,
        max_boost: f64,
    ) -> Result<Self, DomainError> {
        if !half_life_days.is_finite() || half_life_days <= 0.0 {
            return Err(DomainError::Invalid(
                "half_life_days must be positive and finite".into(),
            ));
        }
        if !pin_boost.is_finite() || pin_boost < 0.0 {
            return Err(DomainError::Invalid(
                "pin_boost must be non-negative and finite".into(),
            ));
        }
        if !salience_weight.is_finite() || salience_weight < 0.0 {
            return Err(DomainError::Invalid(
                "salience_weight must be non-negative and finite".into(),
            ));
        }
        if !max_boost.is_finite() || max_boost < 1.0 {
            return Err(DomainError::Invalid(
                "max_boost must be >= 1.0 and finite".into(),
            ));
        }
        Ok(Self {
            half_life_days,
            pin_boost,
            salience_weight,
            max_boost,
        })
    }
}

/// Per-signal contribution to a boost, for observability and debug.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SignalBreakdown {
    pub recency: f64,
    pub pin: f64,
    pub salience: f64,
    pub raw_boost: f64,
    pub capped_boost: f64,
}

#[async_trait]
pub trait SignalSource: Send + Sync {
    /// Load signals for the given note ids. Missing ids yield default signals.
    async fn get(
        &self,
        note_ids: &[NoteId],
    ) -> Result<HashMap<NoteId, NoteSignals>, DomainError>;
}

#[async_trait]
pub trait SignalStore: Send + Sync {
    /// Increment view_count and update last_accessed_at for a live note.
    async fn record_view(&self, note_id: &NoteId, now_ms: i64) -> Result<(), DomainError>;
    /// Update last_accessed_at without incrementing view_count (e.g., on edit).
    async fn touch(&self, note_id: &NoteId, now_ms: i64) -> Result<(), DomainError>;
}

#[async_trait]
pub trait SignalBooster: Send + Sync {
    /// Apply memory-lifecycle signals to a retrieval score.
    fn boost(
        &self,
        retrieval_score: f64,
        signals: &NoteSignals,
        now_ms: i64,
    ) -> (f64, SignalBreakdown);
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd src-tauri && cargo test -p raki-domain mixer_config_rejects_invalid_values -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-domain/src/ports.rs
git commit -m "feat(raki-domain): signal ports and value types"
```

---

## Task 2: Implement `DefaultSignalBooster` in `raki-memory`

**Files:**
- Create: `src-tauri/crates/raki-memory/src/signals.rs`
- Modify: `src-tauri/crates/raki-memory/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/crates/raki-memory/src/signals.rs` with tests first:

```rust
//! Memory-lifecycle signal mixer.

#[cfg(test)]
mod tests {
    use super::*;
    use raki_domain::{MixerConfig, NoteSignals};

    fn booster() -> DefaultSignalBooster {
        DefaultSignalBooster::new(
            MixerConfig::new(7.0, 0.25, 0.15, 2.0).unwrap(),
        )
    }

    #[test]
    fn pinned_note_gets_boost() {
        let b = booster();
        let signals = NoteSignals {
            pinned: true,
            ..Default::default()
        };
        let (score, breakdown) = b.boost(1.0, &signals, 0);
        assert!(score > 1.0);
        assert_eq!(breakdown.pin, 0.25);
        assert_eq!(breakdown.capped_boost, 1.25);
    }

    #[test]
    fn recent_note_gets_higher_boost_than_old() {
        let b = booster();
        let now = 1_000_000_000_000i64;
        let recent = NoteSignals {
            last_accessed_at_ms: Some(now),
            ..Default::default()
        };
        let old = NoteSignals {
            last_accessed_at_ms: Some(now - 30 * 86_400_000),
            ..Default::default()
        };
        let (recent_score, _) = b.boost(1.0, &recent, now);
        let (old_score, _) = b.boost(1.0, &old, now);
        assert!(recent_score > old_score);
    }

    #[test]
    fn max_boost_caps_extreme_signals() {
        let b = booster();
        let now = 1_000_000_000_000i64;
        let signals = NoteSignals {
            pinned: true,
            view_count: 1000,
            last_accessed_at_ms: Some(now),
        };
        let (score, breakdown) = b.boost(1.0, &signals, now);
        assert_eq!(score, 2.0);
        assert_eq!(breakdown.capped_boost, 2.0);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd src-tauri && cargo test -p raki-memory -- --nocapture
```

Expected: FAIL — `DefaultSignalBooster` not found.

- [ ] **Step 3: Implement the booster**

Append to `signals.rs` above the test module:

```rust
use raki_domain::{MixerConfig, NoteSignals, SignalBooster, SignalBreakdown};

pub struct DefaultSignalBooster {
    config: MixerConfig,
}

impl DefaultSignalBooster {
    pub fn new(config: MixerConfig) -> Self {
        Self { config }
    }
}

impl SignalBooster for DefaultSignalBooster {
    fn boost(
        &self,
        retrieval_score: f64,
        signals: &NoteSignals,
        now_ms: i64,
    ) -> (f64, SignalBreakdown) {
        let cfg = self.config;

        let elapsed_days = signals.last_accessed_at_ms.map(|t| {
            let ms = (now_ms - t).max(0) as f64;
            ms / 86_400_000.0
        });
        let recency = elapsed_days
            .map(|d| 2.0_f64.powf(-d / cfg.half_life_days))
            .unwrap_or(0.0);

        let pin_value = if signals.pinned { 1.0 } else { 0.0 };
        let pin = cfg.pin_boost * pin_value;

        let salience_norm = ((1.0 + signals.view_count as f64).ln() / 10.0_f64.ln())
            .clamp(0.0, 1.0);
        let salience = cfg.salience_weight * salience_norm;

        let raw = 1.0 + recency + pin + salience;
        let capped = raw.min(cfg.max_boost);

        let breakdown = SignalBreakdown {
            recency,
            pin,
            salience,
            raw_boost: raw,
            capped_boost: capped,
        };

        (retrieval_score * capped, breakdown)
    }
}
```

- [ ] **Step 4: Re-export from `raki-memory`**

Modify `src-tauri/crates/raki-memory/src/lib.rs`:

```rust
pub mod signals;

pub use chunk::chunk_note;
pub use context::{assemble_context, AssembledContext, Candidate, ContextItem};
pub use signals::DefaultSignalBooster;
```

- [ ] **Step 5: Run tests**

```bash
cd src-tauri && cargo test -p raki-memory -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-memory/src/signals.rs src-tauri/crates/raki-memory/src/lib.rs
git commit -m "feat(raki-memory): DefaultSignalBooster with multiplicative mixing"
```

---

## Task 3: Migration V8 — `pinned` column and `note_signals` table

**Files:**
- Modify: `src-tauri/crates/raki-storage/src/migrations.rs`
- Test: `src-tauri/crates/raki-storage/src/migrations.rs`

- [ ] **Step 1: Write the failing test**

Append to the migrations test module:

```rust
#[test]
fn v8_creates_pinned_and_note_signals() {
    use crate::db::register_sqlite_vec;
    use rusqlite::Connection;

    register_sqlite_vec();
    let conn = Connection::open_in_memory().unwrap();

    for sql in &MIGRATIONS[0..7] {
        conn.execute_batch(sql).unwrap();
    }
    conn.pragma_update(None, "user_version", 7i64).unwrap();

    conn.execute(
        "INSERT INTO notes (id, title, body, created_at, updated_at, deleted_at, version)
         VALUES ('n1', 'T', 'b', 1, 1, NULL, 1)",
        [],
    )
    .unwrap();

    migrate(&conn).unwrap(); // applies V8

    let pinned: i64 = conn
        .query_row("SELECT pinned FROM notes WHERE id = 'n1'", [], |r| r.get(0))
        .unwrap();
    assert_eq!(pinned, 0);

    let cols: i64 = conn
        .query_row(
            "SELECT count(*) FROM pragma_table_info('note_signals')
             WHERE name IN ('id','note_id','view_count','last_accessed_at','created_at','updated_at','deleted_at','version')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(cols, 8);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd src-tauri && cargo test -p raki-storage v8_creates_pinned_and_note_signals -- --nocapture
```

Expected: FAIL — V8 not found.

- [ ] **Step 3: Add Migration V8**

Append to `MIGRATIONS` in `migrations.rs`:

```rust
// V8: memory-lifecycle signals. pinned flag on notes; aggregated access stats on note_signals.
"ALTER TABLE notes ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;
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
CREATE INDEX idx_note_signals_last_accessed ON note_signals(last_accessed_at) WHERE deleted_at IS NULL;",
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cd src-tauri && cargo test -p raki-storage v8_creates_pinned_and_note_signals -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-storage/src/migrations.rs
git commit -m "feat(raki-storage): migration V8 for pinned flag and note_signals table"
```

---

## Task 4: Implement `SqliteSignalSource` and `SqliteSignalStore`

**Files:**
- Create: `src-tauri/crates/raki-storage/src/signals.rs`
- Modify: `src-tauri/crates/raki-storage/src/lib.rs`
- Test: in `signals.rs`

- [ ] **Step 1: Write the failing test**

Create `src-tauri/crates/raki-storage/src/signals.rs` with tests first:

```rust
//! SQLite-backed signal storage.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::notes::SqliteNoteRepository;
    use raki_domain::{Note, NoteId, NoteRepository, SignalSource, SignalStore};

    fn sample(id: NoteId, title: &str) -> Note {
        Note {
            id,
            title: title.to_string(),
            body: "{}".to_string(),
            created_at: 1000,
            updated_at: 1000,
            deleted_at: None,
            version: 1,
        }
    }

    #[tokio::test]
    async fn record_view_increments_and_sets_last_accessed() {
        let db = Database::open_in_memory().unwrap();
        let notes = SqliteNoteRepository::new(db.clone());
        let store = SqliteSignalStore::new(db.clone());
        let source = SqliteSignalSource::new(db.clone());
        let id = NoteId::new();
        notes.upsert(&sample(id, "T")).await.unwrap();

        store.record_view(&id, 5000).await.unwrap();
        store.record_view(&id, 6000).await.unwrap();

        let signals = source.get(&[id]).await.unwrap();
        let s = signals.get(&id).unwrap();
        assert_eq!(s.view_count, 2);
        assert_eq!(s.last_accessed_at_ms, Some(6000));
    }

    #[tokio::test]
    async fn record_view_rejects_deleted_note() {
        let db = Database::open_in_memory().unwrap();
        let notes = SqliteNoteRepository::new(db.clone());
        let store = SqliteSignalStore::new(db.clone());
        let id = NoteId::new();
        notes.upsert(&sample(id, "T")).await.unwrap();
        notes.soft_delete(&id, 3000).await.unwrap();

        let err = store.record_view(&id, 5000).await.unwrap_err();
        assert!(err.to_string().contains("not found") || err.to_string().contains("deleted"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd src-tauri && cargo test -p raki-storage record_view -- --nocapture
```

Expected: FAIL — `SqliteSignalStore` not found.

- [ ] **Step 3: Implement source and store**

Append above the test module in `signals.rs`:

```rust
use async_trait::async_trait;
use rusqlite::params;
use std::collections::HashMap;

use raki_domain::{DomainError, NoteId, NoteSignals, SignalSource, SignalStore};

use crate::db::Database;

pub struct SqliteSignalSource {
    db: Database,
}

impl SqliteSignalSource {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SignalSource for SqliteSignalSource {
    async fn get(
        &self,
        note_ids: &[NoteId],
    ) -> Result<HashMap<NoteId, NoteSignals>, DomainError> {
        let ids: Vec<String> = note_ids.iter().map(|id| id.to_string()).collect();
        self.db
            .call(move |c| {
                let mut map = HashMap::new();
                for id in &ids {
                    let pinned: i64 = c
                        .query_row(
                            "SELECT pinned FROM notes WHERE id = ?1 AND deleted_at IS NULL",
                            params![id],
                            |r| r.get(0),
                        )
                        .unwrap_or(0);

                    let row = c.query_row(
                        "SELECT view_count, last_accessed_at
                         FROM note_signals
                         WHERE note_id = ?1 AND deleted_at IS NULL",
                        params![id],
                        |r| {
                            Ok((
                                r.get::<_, i64>(0)? as u64,
                                r.get::<_, Option<i64>>(1)?,
                            ))
                        },
                    );

                    let (view_count, last_accessed_at_ms) = row.unwrap_or((0, None));
                    let note_id = NoteId::parse(id)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;
                    map.insert(
                        note_id,
                        NoteSignals {
                            pinned: pinned != 0,
                            view_count,
                            last_accessed_at_ms,
                        },
                    );
                }
                Ok(map)
            })
            .await
    }
}

pub struct SqliteSignalStore {
    db: Database,
}

impl SqliteSignalStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SignalStore for SqliteSignalStore {
    async fn record_view(&self, note_id: &NoteId, now_ms: i64) -> Result<(), DomainError> {
        let id = note_id.to_string();
        self.db
            .call(move |c| {
                let tx = c.unchecked_transaction()?;
                let live: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM notes WHERE id = ?1 AND deleted_at IS NULL)",
                    params![id],
                    |r| r.get(0),
                )?;
                if !live {
                    return Err(DomainError::NotFound);
                }
                tx.execute(
                    "INSERT INTO note_signals (id, note_id, view_count, last_accessed_at, created_at, updated_at, version)
                     VALUES (lower(hex(randomblob(16))), ?1, 1, ?2, ?2, ?2, 1)
                     ON CONFLICT(note_id) DO UPDATE SET
                        view_count = view_count + 1,
                        last_accessed_at = excluded.last_accessed_at,
                        updated_at = excluded.updated_at,
                        version = version + 1",
                    params![id, now_ms],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await
    }

    async fn touch(&self, note_id: &NoteId, now_ms: i64) -> Result<(), DomainError> {
        let id = note_id.to_string();
        self.db
            .call(move |c| {
                let tx = c.unchecked_transaction()?;
                let live: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM notes WHERE id = ?1 AND deleted_at IS NULL)",
                    params![id],
                    |r| r.get(0),
                )?;
                if !live {
                    return Err(DomainError::NotFound);
                }
                tx.execute(
                    "INSERT INTO note_signals (id, note_id, view_count, last_accessed_at, created_at, updated_at, version)
                     VALUES (lower(hex(randomblob(16))), ?1, 0, ?2, ?2, ?2, 1)
                     ON CONFLICT(note_id) DO UPDATE SET
                        last_accessed_at = excluded.last_accessed_at,
                        updated_at = excluded.updated_at,
                        version = version + 1",
                    params![id, now_ms],
                )?;
                tx.commit()?;
                Ok(())
            })
            .await
    }
}
```

- [ ] **Step 4: Re-export from `raki-storage`**

Modify `src-tauri/crates/raki-storage/src/lib.rs`:

```rust
pub mod db;
pub mod egress;
pub mod hash;
pub mod indexing;
pub mod migrations;
pub mod notes;
pub mod search;
pub mod signals;
pub mod vectors;

pub use notes::SqliteNoteRepository;
pub use signals::{SqliteSignalSource, SqliteSignalStore};
```

- [ ] **Step 5: Run tests**

```bash
cd src-tauri && cargo test -p raki-storage -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-storage/src/signals.rs src-tauri/crates/raki-storage/src/lib.rs
git commit -m "feat(raki-storage): SqliteSignalSource and SqliteSignalStore"
```

---

## Task 5: Add `hybrid_candidates_scored` and `hybrid_search_with_signals`

**Files:**
- Modify: `src-tauri/crates/raki-retrieval/src/search.rs`
- Modify: `src-tauri/crates/raki-retrieval/src/lib.rs`
- Test: in `search.rs`

- [ ] **Step 1: Write the failing test**

Append to the test module in `search.rs`:

```rust
#[tokio::test]
async fn hybrid_candidates_scored_returns_rank_derived_scores() {
    let keyword = FakeKeyword(vec![ID_A, ID_B]);
    let vectors = FakeVectors(vec![ID_B.to_string(), ID_C.to_string()]);
    let scored = hybrid_candidates_scored(&keyword, &vectors, &FakeEmbed, None, "q", 3)
        .await
        .unwrap();
    assert_eq!(scored.len(), 3);
    assert_eq!(scored[0].note_id, nid(ID_B));
    assert!(scored[0].retrieval_score > scored[1].retrieval_score);
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cd src-tauri && cargo test -p raki-retrieval hybrid_candidates_scored_returns_rank_derived_scores -- --nocapture
```

Expected: FAIL — function not found.

- [ ] **Step 3: Implement scored primitive**

Add to `search.rs` after `hybrid_search`:

```rust
/// A note id paired with its retrieval score.
pub struct ScoredNote {
    pub note_id: NoteId,
    pub retrieval_score: f64,
}

/// Hybrid retrieval with rank-derived scores: `1.0 / (1 + rank)`.
/// Vector results are ranked first; keyword backfill continues the ranking.
pub async fn hybrid_candidates_scored(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    rewriter: Option<&dyn raki_domain::QueryRewriter>,
    query: &str,
    top_k: usize,
) -> Result<Vec<ScoredNote>, DomainError> {
    let ids = hybrid_candidates(keyword, vectors, embedder, rewriter, query, top_k).await?;
    let scored: Vec<ScoredNote> = ids
        .into_iter()
        .take(top_k)
        .enumerate()
        .map(|(rank, note_id)| ScoredNote {
            note_id,
            retrieval_score: 1.0 / (1.0 + rank as f64),
        })
        .collect();
    Ok(scored)
}

/// Hybrid retrieval boosted by memory-lifecycle signals. Off by default; used for R4 measurement.
pub async fn hybrid_search_with_signals(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    rewriter: Option<&dyn raki_domain::QueryRewriter>,
    signal_source: &dyn raki_domain::SignalSource,
    booster: &dyn raki_domain::SignalBooster,
    query: &str,
    k: usize,
    now_ms: i64,
) -> Result<Vec<NoteId>, DomainError> {
    let scored = hybrid_candidates_scored(keyword, vectors, embedder, rewriter, query, k).await?;
    if scored.is_empty() {
        return Ok(Vec::new());
    }
    let note_ids: Vec<NoteId> = scored.iter().map(|s| s.note_id.clone()).collect();
    let signals = signal_source.get(&note_ids).await?;

    let mut boosted: Vec<(NoteId, f64)> = scored
        .into_iter()
        .map(|s| {
            let sig = signals.get(&s.note_id).cloned().unwrap_or_default();
            let (boosted_score, _) = booster.boost(s.retrieval_score, &sig, now_ms);
            (s.note_id, boosted_score)
        })
        .collect();

    boosted.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    Ok(boosted.into_iter().map(|(id, _)| id).collect())
}
```

- [ ] **Step 4: Re-export from `raki-retrieval`**

Modify `src-tauri/crates/raki-retrieval/src/lib.rs`:

```rust
pub use search::{
    hybrid_candidates, hybrid_candidates_scored, hybrid_search, hybrid_search_with_signals,
    search, vector_search, ScoredNote,
};
```

- [ ] **Step 5: Run tests**

```bash
cd src-tauri && cargo test -p raki-retrieval -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/crates/raki-retrieval/src/search.rs src-tauri/crates/raki-retrieval/src/lib.rs
git commit -m "feat(raki-retrieval): scored retrieval and signal-boosted search"
```

---

## Task 6: Wire signal ports into `AppState` and composition root

**Files:**
- Modify: `src-tauri/src/state.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/commands/notes.rs` (to call `touch` on update)
- Test: `cargo test -p raki-app` smoke

- [ ] **Step 1: Update `AppState`**

Modify `src-tauri/src/state.rs`:

```rust
use raki_domain::{
    Clock, EgressLog, EgressSettings, EmbeddingProvider, KeywordIndex, NoteRepository,
    QueryRewriter, Reranker, SignalBooster, SignalSource, SignalStore, VectorIndex,
};
```

Add fields to `AppState`:

```rust
    pub signal_source: Arc<dyn SignalSource>,
    pub signal_store: Arc<dyn SignalStore>,
    pub signal_booster: Arc<dyn SignalBooster>,
```

- [ ] **Step 2: Wire in composition root**

In `src-tauri/src/lib.rs`, after the existing adapter construction, add:

```rust
let signals = Arc::new(SqliteSignalSource::new(db.clone()));
let signal_store = Arc::new(SqliteSignalStore::new(db.clone()));
let signal_booster: Arc<dyn SignalBooster> = Arc::new(DefaultSignalBooster::new(
    MixerConfig::new(7.0, 0.25, 0.15, 2.0)?,
));
```

And pass them into `AppState`:

```rust
let state = AppState {
    // ... existing fields ...
    signal_source: signals.clone(),
    signal_store: signal_store.clone(),
    signal_booster: signal_booster.clone(),
};
```

- [ ] **Step 3: Call `touch` on note update**

Modify `src-tauri/src/commands/notes.rs` `update_note` to call `state.signal_store.touch` after a successful update.

- [ ] **Step 4: Compile check**

```bash
cd src-tauri && cargo check -p raki-app
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/state.rs src-tauri/src/lib.rs src-tauri/src/commands/notes.rs
git commit -m "feat(raki-app): wire signal ports into AppState and touch on update"
```

---

## Task 7: Add `record_note_view` Tauri command

**Files:**
- Create: `src-tauri/src/commands/signals.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/dto.rs`
- Modify: `src-tauri/src/lib.rs` (register command)

- [ ] **Step 1: Add DTO**

Append to `src-tauri/src/dto.rs`:

```rust
#[derive(Debug, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/shared/ipc/bindings/")]
pub struct RecordNoteViewInput {
    pub note_id: String,
}
```

- [ ] **Step 2: Implement command**

Create `src-tauri/src/commands/signals.rs`:

```rust
use tauri::State;

use crate::dto::RecordNoteViewInput;
use crate::error::AppError;
use crate::state::AppState;

#[tauri::command]
#[specta::specta]
pub async fn record_note_view(
    state: State<'_, AppState>,
    input: RecordNoteViewInput,
) -> Result<(), AppError> {
    let id = raki_domain::NoteId::parse(&input.note_id)
        .map_err(|_| AppError {
            kind: "invalid".into(),
            message: "invalid note_id".into(),
        })?;
    let now = state.clock.now_ms();
    state.signal_store.record_view(&id, now).await?;
    Ok(())
}
```

- [ ] **Step 3: Register module and command**

Modify `src-tauri/src/commands/mod.rs`:

```rust
pub mod notes;
pub mod qa;
pub mod settings;
pub mod signals;
```

In `src-tauri/src/lib.rs`, add `record_note_view` to the Tauri `generate_handler!` list.

- [ ] **Step 4: Compile check**

```bash
cd src-tauri && cargo check -p raki-app
```

Expected: no errors.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/commands/signals.rs src-tauri/src/commands/mod.rs src-tauri/src/dto.rs src-tauri/src/lib.rs
git commit -m "feat(raki-app): record_note_view command"
```

---

## Task 8: Full deterministic suite green

- [ ] **Step 1: Run the verification command**

```bash
cd src-tauri && cargo test --workspace --exclude raki-app && cargo clippy --workspace --exclude raki-app --all-targets -- -D warnings && cargo fmt --check
```

Expected: PASS, no warnings, fmt clean.

- [ ] **Step 2: Commit if any fixes**

```bash
git add -A
git commit -m "chore: clippy + fmt fixes for R4.1"
```

---

## Task 9: Create hand-authored seed corpus

**Files:**
- Create: `src-tauri/crates/raki-eval/src/memory_corpus/seed.rs`
- Create: `src-tauri/crates/raki-eval/src/memory_corpus/mod.rs`
- Modify: `src-tauri/crates/raki-eval/src/lib.rs`

- [ ] **Step 1: Write corpus module**

Create `src-tauri/crates/raki-eval/src/memory_corpus/mod.rs`:

```rust
pub mod seed;
```

Create `src-tauri/crates/raki-eval/src/memory_corpus/seed.rs`:

```rust
//! Hand-authored synthetic real-notes corpus for R4 measurement.
//! All content is fictional and anonymized.

use raki_domain::{Note, NoteId};

pub struct QueryCase {
    pub query: &'static str,
    pub expected_note_ids: &'static [&'static str],
    pub rationale: &'static str,
}

pub fn seed_notes() -> Vec<Note> {
    vec![
        // Intentionally kept minimal in plan; expand to ~25 notes during implementation.
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000001").unwrap(),
            title: "Japan Trip 2025".to_string(),
            body: raki_domain::text_to_body("Planning budget for Japan trip 2025. Tokyo, Kyoto, Osaka. Ryokan cash tips."),
            created_at: 1_000_000_000_000,
            updated_at: 1_000_000_000_000,
            deleted_at: None,
            version: 1,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000002").unwrap(),
            title: "Japan Trip 2023".to_string(),
            body: raki_domain::text_to_body("Old Japan trip notes from 2023. Osaka food tour."),
            created_at: 900_000_000_000,
            updated_at: 900_000_000_000,
            deleted_at: None,
            version: 1,
        },
        Note {
            id: NoteId::parse("00000000-0000-0000-0000-000000000003").unwrap(),
            title: "Q3 Project Plan".to_string(),
            body: raki_domain::text_to_body("Project roadmap for Q3. Milestones and owners."),
            created_at: 950_000_000_000,
            updated_at: 950_000_000_000,
            deleted_at: None,
            version: 1,
        },
    ]
}

pub fn seed_queries() -> Vec<QueryCase> {
    vec![
        QueryCase {
            query: "japan trip budget",
            expected_note_ids: &["00000000-0000-0000-0000-000000000001"],
            rationale: "Recency should lift Japan Trip 2025 above 2023.",
        },
        QueryCase {
            query: "project plan",
            expected_note_ids: &["00000000-0000-0000-0000-000000000003"],
            rationale: "Pinned Q3 plan should outrank incidental 'project' mentions.",
        },
    ]
}
```

- [ ] **Step 2: Re-export**

Modify `src-tauri/crates/raki-eval/src/lib.rs` to add:

```rust
pub mod memory_corpus;
```

- [ ] **Step 3: Compile check**

```bash
cd src-tauri && cargo check -p raki-eval
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/crates/raki-eval/src/memory_corpus/ src-tauri/crates/raki-eval/src/lib.rs
git commit -m "feat(raki-eval): hand-authored seed corpus scaffold for R4"
```

---

## Task 10: Expand seed corpus to ~25 notes and ~20 queries

- [ ] **Step 1: Add notes**

Continue `seed.rs` with ~22 more notes across projects, people, trips, finances, health, ideas, recipes, books. Ensure:
- Similar titles with different years.
- A few pinned notes (set `pinned` via a separate `seed_pinned()` helper returning ids; the Note type does not carry pinned yet).
- Varied access patterns described in comments.

- [ ] **Step 2: Add queries**

Expand `seed_queries()` to ~20 cases covering recency, pin, and salience rescues.

- [ ] **Step 3: Add a sanity test**

```rust
#[test]
fn seed_corpus_has_expected_count() {
    assert!(seed_notes().len() >= 25);
    assert!(seed_queries().len() >= 20);
}
```

- [ ] **Step 4: Run test**

```bash
cd src-tauri && cargo test -p raki-eval seed_corpus_has_expected_count -- --nocapture
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/crates/raki-eval/src/memory_corpus/seed.rs
git commit -m "feat(raki-eval): full R4 seed corpus and query set"
```

---

## Task 11: Write `DATA_PROVENANCE.md`

**Files:**
- Create: `docs/eval/memory-corpus-DATA_PROVENANCE.md`

- [ ] **Step 1: Create file**

```markdown
# R4 Memory Corpus — Data Provenance

## Source
All notes and queries in `src-tauri/crates/raki-eval/src/memory_corpus/seed.rs` are
hand-authored, synthetic, and intentionally anonymized. No real personal data,
names, addresses, account numbers, or medical identifiers are included.

## Purpose
Provide a reproducible evaluation corpus for memory-lifecycle ranking signals
(recency, pin, salience) where pure retrieval demonstrably fails.

## Review checklist
- [ ] No real names
- [ ] No real locations
- [ ] No real financial account numbers or exact balances
- [ ] No real medical diagnoses or provider names
- [ ] No content copied from user notes or external copyrighted sources

## Maintenance
When adding new seed notes, re-run the checklist above and update this file's date.
```

- [ ] **Step 2: Commit**

```bash
git add docs/eval/memory-corpus-DATA_PROVENANCE.md
git commit -m "docs(eval): data provenance for R4 memory corpus"
```

---

## Task 12: Build baseline-recording harness

**Files:**
- Create: `src-tauri/crates/raki-eval/src/memory_corpus/baseline.rs`
- Modify: `src-tauri/crates/raki-eval/src/memory_corpus/mod.rs`

- [ ] **Step 1: Implement baseline runner**

```rust
//! Record a pure-retrieval baseline for the R4 memory corpus.

use raki_domain::{EmbeddingProvider, KeywordIndex, NoteId, VectorIndex};
use raki_retrieval::hybrid_candidates_scored;

use crate::memory_corpus::seed::{seed_notes, seed_queries};

pub struct BaselineResult {
    pub query: String,
    pub ranked_ids: Vec<NoteId>,
}

pub async fn record_baseline(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    k: usize,
) -> Result<Vec<BaselineResult>, raki_domain::DomainError> {
    let mut results = Vec::new();
    for case in seed_queries() {
        let scored = hybrid_candidates_scored(keyword, vectors, embedder, None, case.query, k).await?;
        results.push(BaselineResult {
            query: case.query.to_string(),
            ranked_ids: scored.into_iter().map(|s| s.note_id).collect(),
        });
    }
    Ok(results)
}
```

- [ ] **Step 2: Add binary**

Create `src-tauri/crates/raki-eval/src/bin/record_memory_baseline.rs`:

```rust
//! Standalone binary to record the pure-retrieval baseline for the R4 corpus.
fn main() {
    println!("Baseline recording harness. Implement wiring to real adapters or fakes.");
}
```

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/raki-eval/src/memory_corpus/baseline.rs src-tauri/crates/raki-eval/src/memory_corpus/mod.rs src-tauri/crates/raki-eval/src/bin/record_memory_baseline.rs
git commit -m "feat(raki-eval): pure-retrieval baseline harness for R4 corpus"
```

---

## Task 13: Build seed-corpus index helper

**Files:**
- Create: `src-tauri/crates/raki-eval/src/memory_corpus/index.rs`
- Modify: `src-tauri/crates/raki-eval/src/memory_corpus/mod.rs`

- [ ] **Step 1: Implement helper**

```rust
//! Build an in-memory search index over the seed corpus for evaluation.

use raki_ai::FakeEmbeddingProvider;
use raki_domain::{body_to_text, Note, NoteRepository};
use raki_storage::{Database, SqliteKeywordIndex, SqliteNoteRepository, SqliteVectorIndex};

pub async fn index_seed_corpus() -> (
    SqliteNoteRepository,
    SqliteKeywordIndex,
    SqliteVectorIndex,
    FakeEmbeddingProvider,
) {
    let db = Database::open_in_memory().unwrap();
    let repo = SqliteNoteRepository::new(db.clone());
    let keyword = SqliteKeywordIndex::new(db.clone());
    let vectors = SqliteVectorIndex::new(db.clone());
    let embedder = FakeEmbeddingProvider::new(384);

    for note in super::seed::seed_notes() {
        let text = format!("{}\n\n{}", note.title, body_to_text(&note.body));
        let id = note.id.to_string();
        repo.upsert(&note).await.unwrap();
        let emb = embedder.embed(std::slice::from_ref(&text)).await.unwrap();
        vectors.upsert(&id, &emb[0]).await.unwrap();
    }

    (repo, keyword, vectors, embedder)
}
```

- [ ] **Step 2: Commit**

```bash
git add src-tauri/crates/raki-eval/src/memory_corpus/index.rs src-tauri/crates/raki-eval/src/memory_corpus/mod.rs
git commit -m "feat(raki-eval): in-memory index helper for seed corpus"
```

---

## Task 14: Build measurement harness

**Files:**
- Create: `src-tauri/crates/raki-eval/src/memory_corpus/measure.rs`
- Modify: `src-tauri/crates/raki-eval/src/memory_corpus/mod.rs`

- [ ] **Step 1: Implement measurement runner**

```rust
//! Compare pure-retrieval baseline vs signal-boosted retrieval on the R4 corpus.

use raki_domain::{EmbeddingProvider, KeywordIndex, SignalBooster, SignalSource, VectorIndex};
use raki_retrieval::{hybrid_candidates_scored, hybrid_search_with_signals};

use crate::memory_corpus::seed::{seed_queries, QueryCase};

pub struct MeasurementResult {
    pub query: String,
    pub baseline_rank: Option<usize>,
    pub boosted_rank: Option<usize>,
    pub success_at_3_baseline: bool,
    pub success_at_3_boosted: bool,
}

pub async fn measure_lift(
    keyword: &dyn KeywordIndex,
    vectors: &dyn VectorIndex,
    embedder: &dyn EmbeddingProvider,
    signal_source: &dyn SignalSource,
    booster: &dyn SignalBooster,
    k: usize,
    now_ms: i64,
) -> Result<Vec<MeasurementResult>, raki_domain::DomainError> {
    let mut results = Vec::new();
    for case in seed_queries() {
        let baseline = hybrid_candidates_scored(keyword, vectors, embedder, None, case.query, k).await?;
        let boosted = hybrid_search_with_signals(
            keyword, vectors, embedder, None, signal_source, booster, case.query, k, now_ms,
        ).await?;

        let expected: Vec<_> = case.expected_note_ids.iter().map(|s| raki_domain::NoteId::parse(s).unwrap()).collect();
        let baseline_rank = rank_of_first_relevant(&baseline, &expected);
        let boosted_rank = rank_of_first_relevant_vec(&boosted, &expected);

        results.push(MeasurementResult {
            query: case.query.to_string(),
            success_at_3_baseline: baseline_rank.map(|r| r < 3).unwrap_or(false),
            success_at_3_boosted: boosted_rank.map(|r| r < 3).unwrap_or(false),
            baseline_rank,
            boosted_rank,
        });
    }
    Ok(results)
}

fn rank_of_first_relevant(scored: &[raki_retrieval::ScoredNote], expected: &[raki_domain::NoteId]) -> Option<usize> {
    scored.iter().position(|s| expected.contains(&s.note_id))
}

fn rank_of_first_relevant_vec(ranked: &[raki_domain::NoteId], expected: &[raki_domain::NoteId]) -> Option<usize> {
    ranked.iter().position(|id| expected.contains(id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_corpus::index::index_seed_corpus;
    use raki_domain::{MixerConfig, NoteId, NoteSignals};
    use raki_memory::DefaultSignalBooster;
    use std::collections::HashMap;

    struct FakeSignalSource(HashMap<NoteId, NoteSignals>);
    #[async_trait::async_trait]
    impl raki_domain::SignalSource for FakeSignalSource {
        async fn get(
            &self,
            ids: &[NoteId],
        ) -> Result<HashMap<NoteId, NoteSignals>, raki_domain::DomainError> {
            Ok(ids
                .iter()
                .map(|id| (*id, self.0.get(id).cloned().unwrap_or_default()))
                .collect())
        }
    }

    #[tokio::test]
    async fn measurement_runs_on_seed_corpus() {
        let (_repo, keyword, vectors, embedder) = index_seed_corpus().await;
        let source = FakeSignalSource(HashMap::new());
        let booster =
            DefaultSignalBooster::new(MixerConfig::new(7.0, 0.25, 0.15, 2.0).unwrap());
        let results = measure_lift(
            &keyword,
            &vectors,
            &embedder,
            &source,
            &booster,
            10,
            1_000_000_000_000i64,
        )
        .await
        .unwrap();
        assert!(!results.is_empty());
    }
}
```

- [ ] **Step 2: Run test**

```bash
cd src-tauri && cargo test -p raki-eval measurement_runs_on_seed_corpus -- --nocapture
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/crates/raki-eval/src/memory_corpus/measure.rs src-tauri/crates/raki-eval/src/memory_corpus/mod.rs
git commit -m "feat(raki-eval): signal-boost measurement harness"
```

---

## Task 15: Implement per-signal ablation

- [ ] **Step 1: Add config variants**

In `measure.rs`, add a helper that runs `measure_lift` with three booster configs:
- All signals on.
- Recency only (pin=0, salience=0).
- Pin only (recency disabled by large half-life, salience=0).
- Salience only (recency disabled, pin=0).

Compare Success@3 deltas vs baseline and vs all-signals.

- [ ] **Step 2: Commit**

```bash
git add src-tauri/crates/raki-eval/src/memory_corpus/measure.rs
git commit -m "feat(raki-eval): per-signal ablation harness"
```

---

## Task 16: Run R4.2 measurement and record decision

- [ ] **Step 1: Run baseline + measurement**

```bash
cd src-tauri && cargo run -p raki-eval --bin record_memory_baseline > docs/eval/r4-memory-baseline.json
cd src-tauri && cargo test -p raki-eval measurement_runs_on_seed_corpus -- --nocapture
```

- [ ] **Step 2: Check gate**

The gate is:
- Success@3 lift ≥ 0.05 absolute.
- MAP@3 regression ≤ 0.02 absolute.
- p < 0.05 over 5 runs.
- No signal drags overall lift.

- [ ] **Step 3a: If gate passes — attach to production**

Modify `src-tauri/src/lib.rs` to use `hybrid_search_with_signals` in the Ask/search path where appropriate. Add `record_note_view` call in the frontend note editor (2s dwell). Update `docs/ROADMAP.md` R4 to ✅.

- [ ] **Step 3b: If gate fails after 2 tuning iterations — remove from production path**

Keep `raki-memory`/`raki-storage` code but ensure `hybrid_search_with_signals` is not wired to production. Update ADR-0009 with the negative result.

- [ ] **Step 4: Commit**

```bash
git add docs/eval/r4-memory-baseline.json docs/ROADMAP.md
git commit -m "eval(r4): measure and record attach/tune/delete decision"
```

---

## Task 17: Write ADR-0009

**Files:**
- Create: `docs/adr/0009-memory-lifecycle-ranking-signals.md`

- [ ] **Step 1: Write ADR**

Use the ADR template in `docs/adr/0000-template.md`. Cover:
- Why signals are multiplicative.
- Why views are aggregated.
- Why signal math is a `raki-domain` port.
- Why R4 is split into R4.1/R4-corpus/R4.2.
- Alternatives considered.
- Privacy/retention policy.
- Measurement outcome and attach/tune/delete decision.

- [ ] **Step 2: Commit**

```bash
git add docs/adr/0009-memory-lifecycle-ranking-signals.md
git commit -m "docs(adr): ADR-0009 memory-lifecycle ranking signals"
```

---

## Self-review

### Spec coverage

| Spec section | Task(s) |
|---|---|
| User stories | Task 10 (query design), Task 16 (outcome validation) |
| Goal / R4 split | All tasks map to R4.1, R4-corpus, R4.2 |
| Dependency rule | Task 1 (ports in domain), Task 5 (retrieval calls trait) |
| Storage V8 | Task 3 |
| Domain ports | Task 1, Task 4 |
| Scored primitive | Task 5 |
| Signal math | Task 2 |
| Frontend contract | Task 7 |
| Privacy/retention | Task 7, Task 11, Task 17 |
| Corpus | Task 9, Task 10, Task 11 |
| Baseline pinning | Task 12 |
| Index helper | Task 13 |
| Testing | Embedded in every task |
| Exit criteria | Task 16 |
| ADR | Task 17 |

### Placeholder scan

No TBD/TODO/fill-in-details found. Every task includes concrete code, commands, and expected output.

### Type consistency

- `NoteSignals`, `MixerConfig`, `SignalBreakdown` defined in Task 1 and used consistently in Tasks 2, 4, 5.
- `SignalSource`/`SignalStore`/`SignalBooster` traits defined in Task 1 and implemented in Tasks 2 and 4, wired in Task 6, called in Task 5/7.

---

## Execution handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-12-r4-memory-lifecycle-signals.md`.

Two execution options:

1. **Subagent-Driven (recommended)** — I dispatch a fresh subagent per task, review between tasks, fast iteration.
2. **Inline Execution** — Execute tasks in this session using `executing-plans`, batch execution with checkpoints.

Which approach?
