# Eval baseline artifact

Date: 2026-06-05

Reproducible baseline for the retrieval eval (D10). The gate floors cite these
numbers; the per-query lock is `snapshot.json` (D5).

## Environment

- Model id: `bge-small-en-v1.5`
- Embedding dimension: 384 (fixed by bge-small-en-v1.5; pinned by model id)
- Platform: macos / aarch64
- Fixture fingerprint (FNV-1a, non-security): `c6ee99850a8034a1`
- Pinned library versions: see committed `src-tauri/Cargo.lock` (fastembed, ort/onnxruntime, rusqlite/SQLite bundled, sqlite-vec).
- k = 3; coverage_k = 10.
- Command: `cargo run -p raki-eval --bin eval-report -- --write --date=<date>`
- Deterministic ordering: keyword is id-sorted in SQL (`ORDER BY score, note_id`);
  vector/hybrid order is deterministic on this pinned environment (see D5/D11).

`coverage_k = 10` rationale: top-10 spans ~45% of the 22-note corpus — a sensible
"find most" horizon. Revisit when the corpus grows (3b).

## Per-category (kw / vec / hyb)

| category | n | kw R/M/N/Cov | vec R/M/N/Cov | hyb R/M/N/Cov |
|---|---|---|---|---|
| buried-fact-in-long-note | 2 | 0.50/0.50/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| coverage | 1 | 0.29/0.24/  - /0.29 | 0.43/0.43/  - /1.00 | 0.43/0.43/  - /1.00 |
| lexical-cluster | 2 | 1.00/1.00/0.73/  -  | 1.00/1.00/0.92/  -  | 1.00/1.00/0.92/  -  |
| lexical-overlap | 3 | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| messy | 1 | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| multi-relevant | 3 | 0.50/0.50/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| named-entity | 2 | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| semantic-paraphrase | 3 | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| temporal | 1 | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| **OVERALL** |  | 0.82/0.82/0.73/0.29 | 0.97/0.97/0.92/1.00 | 0.97/0.97/0.92/1.00 |

Unscored categories: ["negative"]
