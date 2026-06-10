# SciFact benchmark (k=10, queries scored = 300)

| method    | nDCG@10 | Recall@10 | MAP |
|-----------|---------|-----------|-----|
| keyword   | 0.6573 | 0.7919 | 0.6095 |
| vector    | 0.7127 | 0.8362 | 0.6681 |
| hybrid    | 0.7127 | 0.8362 | 0.6681 |
| reranked  | 0.7440 | 0.8647 | 0.7000 |

**reranked − hybrid nDCG@10 = +0.0313** (R1 directional signal)
vector nDCG@10 = 0.7127 (sanity vs published bge-small ≈ 0.65)

model: bge-small-en-v1.5 · dataset: BEIR SciFact (CC BY-NC 2.0; downloaded, not redistributed)
