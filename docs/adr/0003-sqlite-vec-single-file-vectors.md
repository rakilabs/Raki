# ADR-0003: Vectors in one SQLite file via sqlite-vec

- **Status:** Accepted
- **Date:** 2026-06-04
- **Deciders:** Raki founding team
- **Tags:** storage, retrieval, ai

## Context

The retrieval layer needs vector search alongside FTS5 keyword search. The vectors can live in the same
SQLite database (`sqlite-vec`), in a separate embedded vector DB (`LanceDB`), or in a newer SQLite ANN
extension (`sqlite-vector` by sqliteai). The product is local-first and prizes data ownership and integrity:
ideally there is **one file** the user owns and backs up, and a note + its searchable text + its embedding
are always consistent.

Scale reality: a personal second brain is realistically tens of thousands to low-hundreds-of-thousands of
chunks, not billions. `sqlite-vec` exact SIMD search is fast and uses ~30MB RAM at that scale; LanceDB's
ANN advantages only matter at millions-of-vectors / multimodal scale.

## Decision

We will store embeddings in the **same SQLite file** as relational data and the FTS5 index, using the
**`sqlite-vec`** extension, behind a `VectorIndex` **port** in `raki-domain`. A note's row, its FTS5 entry, and
its vector are written in **one transaction** (one source of truth). The port boundary means we can swap the
implementation (e.g., to LanceDB or an ANN extension) later without touching retrieval or memory logic.

## Consequences

**Positive**
- One file to own, back up, and export. Transactional consistency eliminates the "vector store drifted out of
  sync with rows" bug class.
- Simpler ops, simpler backups, simpler portability — directly serves the data-ownership value.
- Exact search = perfect recall at personal scale.

**Negative / costs**
- Exact search is O(n) per query; at very large corpora this would need ANN. Acceptable now; bounded by the port.
- `sqlite-vec` must be bundled and registered as an auto-extension on every connection.

**Neutral / follow-ups**
- Revisit if a real corpus exceeds what exact search serves comfortably; the swap is an implementation detail
  behind `VectorIndex`, recorded as a follow-up ADR.

## Alternatives considered

- **LanceDB (separate store)** — fastest at millions of vectors + multimodal, but a second store to keep
  consistent with SQLite, undermining the one-file ownership story. Deferred behind the port.
- **`sqlite-vector` (sqliteai)** — single-file with built-in ANN and strong benchmarks, but less battle-tested
  than `sqlite-vec` today. Reconsider as it matures.

## References

- `AGENT.md` §5 (one source of truth), §7 (indexes), §9 (retrieval), §15 (scale posture).
