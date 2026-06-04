# ADR-0002: Single-device now, sync-ready data model

- **Status:** Accepted
- **Date:** 2026-06-04
- **Deciders:** Raki founding team
- **Tags:** data, storage, architecture

## Context

"Local-first" classically implies eventual multi-device sync; "fully offline" does not require it. We must
choose how much sync machinery to build now. Options range from a bare single-device schema (cheapest now,
rewrite later) to full CRDTs from day one (Automerge/Yjs — powerful, but heavy for a v1). The team wants to
avoid both a future data rewrite *and* premature complexity.

## Decision

We will build for a **single device now**, but make the data model **sync-ready**. Every user-data row carries:
`id` (UUID **v7**), `created_at`, `updated_at`, `deleted_at` (**soft delete**), and a monotonic `version`.
A `change_log` table records every mutation. We will **not** add a sync runtime or CRDTs yet (YAGNI), but these
conventions are the seam that makes file-based or CRDT-based sync addable later **without a data rewrite**.

## Consequences

**Positive**
- Cheap now; the schema discipline is light (a handful of standard columns + a log table).
- Future sync, undo/history, and audit features become additive.
- Soft-delete protects user data from accidental hard deletion.

**Negative / costs**
- Slightly more bookkeeping on every write (timestamps, version bump, change-log row).
- Queries must consistently filter `deleted_at IS NULL`; we enforce this in repositories.

**Neutral / follow-ups**
- When sync is built, it gets its own ADR (CRDT vs file-based vs server-relay) and reuses these IDs/logs.

## Alternatives considered

- **Pure single-device forever** — simplest schema, but a near-certain future rewrite. Rejected.
- **Full CRDT sync now (Automerge/Yjs)** — great end state, but large upfront cost and complexity for a v1
  with no second device yet. Deferred, not rejected — the seams above keep the door open.

## References

- `AGENT.md` §7 (row conventions, migrations, change-log).
