# ADR-0004: ProseMirror JSON as the canonical note format

- **Status:** Accepted
- **Date:** 2026-06-04
- **Deciders:** Raki founding team
- **Tags:** frontend, data, editor, retrieval

## Context

Notes are edited with TipTap (v3, headless, ProseMirror-based). We must choose the **canonical** on-disk
representation: ProseMirror JSON (structured), Markdown (portable, git-friendly), or a hybrid synced vault.
The decision affects rich features, block-level linking, retrieval chunking, and the data-ownership value.

## Decision

We will treat **ProseMirror JSON** as the **canonical** note content, with **Markdown export/import** as a
first-class projection (for portability and ownership). Every block carries a **stable block ID** (assigned by
a TipTap extension), which is the unit of retrieval chunking and of block-level cross-module linking.

## Consequences

**Positive**
- Structured content enables block-level linking, embeds, tasks-in-notes, and stable chunk → source mapping
  for retrieval provenance.
- Stable block IDs survive edits, so embeddings and links remain anchored to real locations.
- Markdown export preserves user ownership/portability without making the lossy format the source of truth.

**Negative / costs**
- ProseMirror JSON is less human-portable than `.md`; we mitigate with robust Markdown export/import.
- TipTap schema changes become **migrations** (they alter stored documents) — must be versioned and tested.

**Neutral / follow-ups**
- Round-trip fidelity (JSON ↔ Markdown) needs tests; some rich constructs degrade gracefully on MD export.

## Alternatives considered

- **Markdown-first (CommonMark on disk)** — maximal portability and git/Obsidian friendliness, but block-level
  linking, stable IDs, and rich structured features become harder; chunking loses structure. Rejected as canonical
  (still supported as export/import).
- **Hybrid (JSON + continuously-synced MD vault)** — best of both, but the most moving parts to keep consistent.
  Deferred as premature for v1.

## References

- `AGENT.md` §5 (editor rules), §7 (schema changes are migrations), §8 (block-aware chunking).
