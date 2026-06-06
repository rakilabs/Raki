# Eval baseline artifact

Date: 2026-06-06

Reproducible baseline for the retrieval eval (D10). The gate floors cite these
numbers; the per-query lock is `snapshot.json` (D5).

## Environment

- Model id: `bge-small-en-v1.5`
- Embedding dimension: 384 (fixed by bge-small-en-v1.5; pinned by model id)
- Platform: macos / aarch64
- Fixture fingerprint (FNV-1a, non-security): `17e38ebdf94a1354`
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
| coverage | 1 | 0.14/0.14/  - /0.29 | 0.43/0.43/  - /1.00 | 0.43/0.43/  - /1.00 |
| dense-near-duplicate | 2 | 1.00/1.00/0.94/  -  | 1.00/1.00/1.00/  -  | 1.00/1.00/1.00/  -  |
| lexical-cluster | 2 | 1.00/1.00/0.73/  -  | 1.00/1.00/0.92/  -  | 1.00/1.00/0.92/  -  |
| lexical-overlap | 3 | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| messy | 1 | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| multi-relevant | 3 | 0.50/0.36/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| named-entity | 2 | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| paraphrase-distractor | 2 | 1.00/0.75/0.81/  -  | 1.00/1.00/0.91/  -  | 1.00/1.00/0.91/  -  |
| polysemy | 2 | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| semantic-paraphrase | 3 | 1.00/0.83/  - /  -  | 1.00/0.83/  - /  -  | 1.00/0.83/  - /  -  |
| temporal | 1 | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  | 1.00/1.00/  - /  -  |
| **OVERALL** |  | 0.86/0.80/0.83/0.29 | 0.98/0.96/0.95/1.00 | 0.98/0.96/0.95/1.00 |

Unscored categories: ["negative"]
