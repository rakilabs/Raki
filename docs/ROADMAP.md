# Raki Roadmap

> **Living tracking file — the step-by-step anchor.** Check this at the start of every slice.
> Sequencing is by **dependency**, not calendar dates (solo project). `AGENTS.md §1` defines the
> phases; this file sequences the concrete milestones inside them and records status.
>
> **The spine:** retrieval/memory quality is the platform's core differentiator. **Phase 1 is not
> "done" until that quality is driven to *best* and *measured* — not merely working.** Breadth
> (Tasks, Finance, …) waits behind a genuinely strong core. Every retrieval lever is gated on a
> corpus where today's retrieval *fails* (ADR-0005, ADR-0006, ADR-0007).

**Status legend:** ✅ done · ▶ active · ⬜ planned · 🔒 blocked (on the noted dependency)

---

## ✅ Done — Phase-1 foundation + first vertical slice

| Slice | What | Evidence |
|---|---|---|
| Foundation skeleton | domain kernel, workspace, dependency rule | `crates/raki-domain` |
| Storage | SQLite/FTS5/sqlite-vec, one-file truth, migrations | `crates/raki-storage` |
| Embedding pipeline | background embed, staleness, single-flight | `src-tauri/src/indexing.rs` |
| Hybrid recall | vector-primary + keyword backfill | `crates/raki-retrieval` (ADR-0006) |
| Eval harness | golden-set + adversarial regression, CI-gated | `crates/raki-eval` (ADR-0005) |
| Context assembly | token-budgeted greedy packing | `crates/raki-memory` |
| Egress substrate | gate, audit log, consent, mode (Slice 1) | `crates/raki-ai` egress |
| Grounded cloud QA | MessagesProvider, groundedness, AskBox (Slice 2) | `crates/raki-generate` |
| **Notes end-to-end** | body editor (draft), real content into the pipeline (Slice 3) | `src/modules/notes`, ADR-0004 |

---

## Track A — Retrieval & Memory Quality (completes Phase 1's core — the spine)

> Pursued in ADR-0006 order. Each milestone is a measured slice, not a guess.

### ✅ R0 — Measurement foundation *(the gate — benchmark-first)*
**Goal:** a corpus where current retrieval **measurably fails**, so any lift is provable.
**Decision (ADR-0007):** benchmark-first. Stand up a public IR benchmark tier (SciFact/BEIR subset)
— reproducible, statistically powered, CI-gateable, no private data — where the bi-encoder genuinely
fails. The real-data tier (faithful, from dogfooding) matures in parallel via **P1**.
**Exit:** the benchmark eval shows vector failing in ≥1 category; a benchmark gate runs in CI.
**Status:** ✅ Done. SciFact tier implemented: `raki-eval/src/benchmark.rs` (BEIR loader + aggregate IR
scorer), `bench` binary (`--write` gated), `#[ignore] benchmark_gate` (vector floor + reranker
plausibility). Spec: `docs/superpowers/specs/2026-06-08-r0-scifact-benchmark-tier-design.md`.
Plan: `docs/superpowers/plans/2026-06-08-r0-scifact-benchmark-tier.md`.
**Baseline recorded** (`docs/eval/scifact-baseline.md`, 300 queries, k=10): vector nDCG@10 **0.7127**
(calibrated vs published bge-small ≈0.65), reranked **0.7440**. The bi-encoder does not catastrophically
fail on SciFact, but the reranker shows measurable lift — the corpus serves R1.

### ✅ R1 — Reranker decision *(precision lever)*
**Goal:** re-measure `reranked` vs `hybrid` on the failing corpus → **attach** the cross-encoder into
production `search_notes` (+ `AppState` wiring) **or delete** it per the committed kill-switch
(`docs/eval/reranker-deletion-criteria.md`: +0.03 nDCG ⇒ attach, else remove).
**Exit:** an honest measured outcome, wired or removed; eval gate reflects it.
**Note:** Full 5K-doc SciFact baseline: `reranked − hybrid` = **+0.0313 nDCG@10** (also +0.0285
Recall@10, +0.0319 MAP) — lift consistent across all three metrics, directionally toward **attach**.
(At k=10 `hybrid ≡ vector` — vector saturates the top-10 window — so this is also `reranked − vector`.)
The earlier 100-doc smoke test read +0.0566; the full corpus is the honest, harder number. This is
domain-shifted evidence per ADR-0007 — the binding +0.03 verdict is on **real-notes** ground truth, so
R1 carries the reranker as *attach-to-validate*, not yet attached.
**Status:** ✅ Done. Reranker attached to production `search_notes` as **attach-to-validate**
(ADR-0008): local cross-encoder reranks the 100-pool to top-20, best-effort with hybrid fallback
(timeout/error/missing/panic/OOB all degrade to hybrid). Binding keep/delete verdict pending
real-notes ground truth (P1); kill-switch armed. Spec/plan:
`docs/superpowers/specs/2026-06-08-r1-reranker-attach-design.md`.

### ✅ R2 — Chunk-level embeddings *(production migration complete)*
**Goal:** retire whole-note embedding; chunk notes. The `buried-fact-in-long-note` category is the
tripwire.
**Status:** **Done.** Production migration implemented and verified:
- `chunk_vectors` vec0 table active; `note_vectors` preserved as stale backup.
- ProseMirror-aware block chunking (`body_to_blocks`) → `cap_split` at 1600 chars → max 32 chunks.
- `hybrid_candidates` / `hybrid_search` roll up chunk IDs via min-rank (first occurrence wins).
- `soft_delete` cleans chunks via `LIKE 'note_id#%'`; re-index is delete-then-upsert atomic.
- V7 migration: additive, forward-only, clears `embedded_hash` to trigger re-index.
- **SQLite verification:** Kyoto test note (2597 chars) → **8 chunks**; Grocery List (181 chars) → **1 chunk**.
- Deterministic suite green: `cargo test`, `clippy -D warnings`, `fmt --check`, `tsc --noEmit`.
- Spec/plan: `docs/superpowers/specs/2026-06-10-r2-chunking-production-migration-design.md`.

> **Binding verdict (D8):** the real-notes corpus Success@3 gate is still open — the migration is
> shipped, but the +0.05 lift on the long stratum must be validated once real notes accumulate.
> Chunking is feature-flagged (`use_contextual_prefix`, default OFF) for safe rollback.

> **Cross-cutting:** R2 unblocks **R3** (Track A). The real-notes corpus that feeds R0's faithful tier
> and R2's binding verdict is still the enabler for measurement — P1 (Track B) remains the path to
> trustworthy real-note dogfooding.

### ✅ R3 — Generate-stage query understanding
**Goal:** LLM query rewriting / HyDE / multi-hop feeding the recall stage (ADR-0006 stage 3).
**Exit:** measured lift over the R2 baseline.
**Status:** ✅ Done. `CloudQueryRewriter` implemented in `raki-ai`, wired through `raki-retrieval`/
`raki-generate`/`raki-app` for Ask only, with caching, timeout, and Kimi thinking-disabled path.
Errors surface to the user instead of silent fallback. Eval integration (`RuleBasedRewriter`) and
live-model smoke test added. Spec/plan:
`docs/superpowers/specs/2026-06-10-r3-query-understanding-design.md`.

### ✅ R4.1 / R4-corpus / R4.2-harness — Memory lifecycle signals
**Goal:** grow `raki-memory` beyond context assembly — recency / salience / pinning signals feeding
ranking ("a second brain knows time, links, and tags").
**Exit:** measured contribution to ranking; not a guess.
**Status:** ✅ Harness complete. R4.1 signal ports + `DefaultSignalBooster`, migration V8,
`SqliteSignalSource`/`SqliteSignalStore`, `record_note_view` command, and seed corpus (26 notes,
26 queries) are implemented under `src-tauri/crates/raki-*` and `raki-eval`.
- **R4.1** — Signal model & multiplicative mixer (`raki-memory`), `SignalSource` port +
  `SqliteSignalSource` storage, migration V8. Build + unit-test; **not** production default yet.
- **R4-corpus** — Hand-authored synthetic real-notes seed corpus under
  `raki-eval/src/memory_corpus/seed.rs`, plus `DATA_PROVENANCE.md`.
- **R4.2** — Baseline + measurement + ablation harness implemented; baseline committed to
  `docs/eval/r4-memory-baseline.json`. Initial run with the deterministic fake embedder yields
  **no measurable Success@3 lift** (3/26 queries succeed at k=10 for baseline, all-signals, and each
  single-signal ablation), so the mixer **remains off the production path**.
**Decision:** Keep the signal infrastructure (ports, storage, booster, command) but do **not** wire
`hybrid_search_with_signals` into production `search_notes`. The binding attach/tune/delete verdict is
pending a real embedding-model evaluation on this corpus (ADR-0009).
**Note:** Link-density and tag-affinity signals are deferred until Phase 2 link graph / tags exist.
Spec: `docs/superpowers/specs/2026-06-10-r4-memory-lifecycle-signals-design.md`.
Plan: `docs/superpowers/plans/2026-06-12-r4-memory-lifecycle-signals.md`.

### ✅ R4 — Memory lifecycle — kill-switch invoked
**Goal:** grow `raki-memory` beyond context assembly — recency / salience / pinning signals feeding
ranking ("a second brain knows time, links, tags").
**Exit:** measured contribution to ranking; not a guess.
**Status:** **Done.** Real-model evaluation (`fastembed` / `bge-small-en-v1.5`) showed no measurable
Success@3 lift on the R4 seed corpus (whole-note 26/26 baseline, chunked 25/26 baseline; signals 0 lift).
Per ADR-0009 the kill-switch was invoked: the signal-boosted search path is **not attached** to
production `search_notes`, and the experimental `search_notes_with_signals` command was removed.
The signal infrastructure (ports, storage, `record_note_view`, `touch` on update) remains in place for
future re-testing.
**Decision recorded in:** ADR-0009.

---

## Track B — Product Trust & Ownership *(parallel enabler — runs alongside Track A)*

### ⬜ P1 — Privacy & data-ownership
**Goal:** note delete / trash + restore (over the built `soft_delete`); a Settings surface —
egress audit-log viewer, consent management, local-only/cloud mode toggle.
**Why parallel, not deferred:** ships Raki's non-negotiable values (private, owned, recoverable,
explicit egress — all currently only half-delivered: the audit log is *recorded but invisible*),
**and** makes the app trustworthy enough to hold real notes → which feeds R0's real-data tier.
**Substrate already built:** `egress_log`, `cloud_consent`, `EgressSettings`, `soft_delete` — this
slice exposes them; it is not new infrastructure.

---

## ⬜ Phase 2 — Tasks + cross-module linking *(only after Track A is strong + notes deep)*
- **T1** Tasks vertical slice (entity, storage, commands, UI) reusing the foundation.
- **T2** Cross-module linking graph (notes ↔ tasks); retrieval across modules.

## ⬜ Phase 3 — Finance.

## ⬜ Phase 4+ — Calendar, habits, reading, browser capture, automation, agents.

## 🔒 Cross-cutting — Sync
Data model is sync-ready (ADR-0002: `version`, `deleted_at`). The **engine** (transport, conflict
resolution) is a deliberate later slice, sequenced when multi-device actually matters (≈ Phase 3–4),
not before. No speculative sync code now.

---

## How we use this file
1. At the start of a slice, pick the next ▶/⬜ milestone (respecting 🔒 dependencies).
2. Brainstorm → spec (`docs/superpowers/specs/`) → plan (`docs/superpowers/plans/`) → implement.
3. Link the spec/plan here and flip status. A milestone is ✅ only when its **exit criterion** is met
   (for app/frontend slices, that includes the manual `tauri dev` walkthrough).
