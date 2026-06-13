<!-- Directional signal only. Not statistically powered; absolutes are an optimistic ceiling. See Limitations in 2026-06-06-real-data-eval-substrate-design.md. -->
# Real-data eval baseline (aggregate-only, content-free)

- Queries: 31
- Platform: macos / aarch64; embed model: `bge-small-en-v1.5`; reranker: `jina-reranker-v1-turbo-en`; k=10

| method | Success@3 | Success@1 | Recall@3 | Recall@10 | MRR | Primary-Success@1 (denom) |
|---|---|---|---|---|---|---|
| kw | 0.68 | 0.52 | 0.61 | 0.78 | 0.61 | 0.45 (29/31) |
| vec | 0.90 | 0.71 | 0.81 | 0.90 | 0.79 | 0.66 (29/31) |
| hyb | 0.90 | 0.71 | 0.81 | 0.90 | 0.79 | 0.66 (29/31) |
| rr | 0.84 | 0.68 | 0.76 | 0.88 | 0.76 | 0.62 (29/31) |
