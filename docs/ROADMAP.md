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
Plan: `docs/superpowers/plans/2026-06-08-r0-scifact-benchmark-tier.md`. Full baseline pending
`cargo run -p raki-eval --bin bench -- --write`.

### ⬜ R1 — Reranker decision *(precision lever)* — unblocked by R0
**Goal:** re-measure `reranked` vs `hybrid` on the failing corpus → **attach** the cross-encoder into
production `search_notes` (+ `AppState` wiring) **or delete** it per the committed kill-switch
(`docs/eval/reranker-deletion-criteria.md`: +0.03 nDCG ⇒ attach, else remove).
**Exit:** an honest measured outcome, wired or removed; eval gate reflects it.
**Note:** Smoke-test delta on 100-doc subset was +0.0566 nDCG@10 (directional toward attach), but
binding verdict requires the full 5K-doc baseline + real-notes ground truth.

### 🔒 R2 — Chunk-level embeddings — blocked on R0
**Goal:** retire whole-note embedding; chunk notes. The `buried-fact-in-long-note` category is the
tripwire. Chunking substrate already exists in `raki-eval`; this wires it into production indexing.
**Exit:** measured lift on the tripwire category; prod indexing chunks notes.

### 🔒 R3 — Generate-stage query understanding — blocked on R2
**Goal:** LLM query rewriting / HyDE / multi-hop feeding the recall stage (ADR-0006 stage 3).
**Exit:** measured lift over the R2 baseline.

### 🔒 R4 — Memory lifecycle — blocked on R0
**Goal:** grow `raki-memory` beyond context assembly — recency / salience / pinning signals feeding
ranking ("a second brain knows time, links, tags"). Today the crate is context-assembly only.
**Exit:** measured contribution to ranking; not a guess.

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
