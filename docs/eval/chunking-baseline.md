# Chunking design baseline (synthetic, k=10)

> **Directional, design-settling only.** The synthetic corpus is small and recall saturates, so the ranking signal lives in **MAP**. The **binding** chunking verdict is real-notes-gated (chunking spec D8: +0.05 Success@3 on the long stratum, by 2026-09-06) — its enabler is roadmap Track B **P1**. This file records *which chunk design* to carry, not whether to ship.

models: bge-small-en-v1.5 / jina-reranker-v1-turbo-en

**Winning arm (buried-fact vector ΔMAP): `bare/min-rank` (Δ +0.200)**
> *Tie-break note:* 5 other arm(s) share the same ΔMAP on this corpus; the winner above is the first in iteration order, not a measured preference.

| arm | vec ΔMAP | reranked ΔMAP | hybrid ΔMAP (deploy-risk) |
|-----|---------:|--------------:|-------------------------:|
| bare/min-rank | +0.125 | +0.000 | +0.125 |
| bare/score-max | +0.125 | +0.000 | +0.125 |
| title/min-rank | +0.125 | +0.000 | +0.125 |
| title/score-max | +0.125 | +0.000 | +0.125 |
| title+head/min-rank | +0.125 | +0.000 | +0.125 |
| title+head/score-max | +0.125 | +0.000 | +0.125 |

> *Rerank invisibility:* reranked ΔMAP is +0.000 across every arm — the cross-encoder recovers the buried fact from the recall pool regardless of chunking, so the lever's signal lives at the **vector/recall stage**, not end-to-end. The real-notes gate (D8) must read the recall stratum, not reranked, to detect chunking's contribution.

### bare/min-rank — per-category ΔMAP (vs whole)
- [buried-fact-long-note] vec +0.200 | reranked +0.000
- [buried-list-item] vec +0.000 | reranked +0.000
- [code-heavy] vec +0.000 | reranked +0.000
- [coreference] vec +0.000 | reranked +0.000

### bare/score-max — per-category ΔMAP (vs whole)
- [buried-fact-long-note] vec +0.200 | reranked +0.000
- [buried-list-item] vec +0.000 | reranked +0.000
- [code-heavy] vec +0.000 | reranked +0.000
- [coreference] vec +0.000 | reranked +0.000

### title/min-rank — per-category ΔMAP (vs whole)
- [buried-fact-long-note] vec +0.200 | reranked +0.000
- [buried-list-item] vec +0.000 | reranked +0.000
- [code-heavy] vec +0.000 | reranked +0.000
- [coreference] vec +0.000 | reranked +0.000

### title/score-max — per-category ΔMAP (vs whole)
- [buried-fact-long-note] vec +0.200 | reranked +0.000
- [buried-list-item] vec +0.000 | reranked +0.000
- [code-heavy] vec +0.000 | reranked +0.000
- [coreference] vec +0.000 | reranked +0.000

### title+head/min-rank — per-category ΔMAP (vs whole)
- [buried-fact-long-note] vec +0.200 | reranked +0.000
- [buried-list-item] vec +0.000 | reranked +0.000
- [code-heavy] vec +0.000 | reranked +0.000
- [coreference] vec +0.000 | reranked +0.000

### title+head/score-max — per-category ΔMAP (vs whole)
- [buried-fact-long-note] vec +0.200 | reranked +0.000
- [buried-list-item] vec +0.000 | reranked +0.000
- [code-heavy] vec +0.000 | reranked +0.000
- [coreference] vec +0.000 | reranked +0.000

