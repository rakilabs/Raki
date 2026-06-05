# Architecture Decision Records

This directory records the **why** behind Raki's important architectural choices. Code shows *what* we did;
ADRs explain *why*, what we traded off, and what would make us revisit.

## How we use ADRs

- Write an ADR for any decision that is **important, contested, or expensive to reverse** (data model, storage,
  AI strategy, retrieval, security/privacy boundaries, major dependency choices).
- One decision per file. Copy `0000-template.md`, give it the next number, and open it as part of the PR that
  makes the decision.
- ADRs are **append-only history**: don't rewrite an accepted ADR — supersede it with a new one and mark the old
  as `Superseded by ADR-XXXX`.
- Reference the relevant `AGENT.md` section so the rule and its rationale stay linked.

## Index

| ADR | Decision | Status |
|---|---|---|
| [0001](0001-provider-agnostic-ai.md) | Provider-agnostic AI (local + cloud, user-selectable) | Accepted |
| [0002](0002-single-device-sync-ready-data-model.md) | Single-device now, sync-ready data model | Accepted |
| [0003](0003-sqlite-vec-single-file-vectors.md) | Vectors in one SQLite file via sqlite-vec | Accepted |
| [0004](0004-prosemirror-json-canonical-note-format.md) | ProseMirror JSON as canonical note format | Accepted |
| [0005](0005-retrieval-quality-measured.md) | Retrieval quality is measured, not vibed | Accepted |

_Template: [`0000-template.md`](0000-template.md)_
