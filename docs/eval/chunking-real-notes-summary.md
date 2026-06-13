<!-- Directional signal only. Not statistically powered; absolutes are an optimistic ceiling. -->
# Chunking real-notes comparison (aggregate-only, content-free)

- Run: `chunk-eval --with-real` (stdout-only; local-only).
- Queries: 31 · Notes: 52 · k=10
- Platform: macos / aarch64; embed model: `bge-small-en-v1.5`; reranker: `jina-reranker-v1-turbo-en`
- Corpus strata: short=47, medium=5, long=0

> The D8 binding gate (+0.05 Success@3 on the **long** stratum) cannot be evaluated yet because the
> current corpus contains no long notes.

| arm | ΔRecall@10 (vec) | ΔMAP@10 (vec) | ΔRecall@10 (hyb) | ΔMAP@10 (hyb) |
|---|---:|---:|---:|---:|
| bare/min-rank | -0.158 | -0.175 | -0.158 | -0.175 |
| bare/score-max | -0.204 | -0.173 | -0.158 | -0.175 |
| title/min-rank | +0.032 | +0.014 | +0.032 | +0.014 |
| title/score-max | +0.019 | -0.003 | +0.032 | +0.014 |
| title+head/min-rank | +0.032 | +0.018 | +0.032 | +0.018 |
| title+head/score-max | +0.035 | -0.026 | +0.032 | +0.018 |

**Directional read:** on this short/medium corpus, chunking without a prefix **hurts** recall; adding
a title (+ heading) prefix **helps slightly** (+0.03 Recall@10). The best arm on this data is
`title+head/min-rank` for MAP, `title+head/score-max` for peak recall. The binding verdict remains
pending a long-note stratum.
