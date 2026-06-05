# 6. Retrieval is staged: recall → rerank → generate

Date: 2026-06-05

## Status

Accepted

## Context

Slice #2 added a hybrid of keyword (FTS5/BM25) and vector (sqlite-vec) retrieval and
measured it against the golden set (ADR-0005). The measurement was decisive and
counter-intuitive:

- On the current 22-note corpus, the embedding model (bge-small-en-v1.5) scores
  **recall@3 = MAP@3 = 1.00 in every category** — it ranks the relevant note first
  essentially always.
- **Score-fusion (Reciprocal Rank Fusion) regressed against pure vector** (multi-relevant
  recall 1.00 → 0.83). With one retriever already near-perfect, RRF's additive score lets
  a keyword-matched non-relevant doc displace a vector-correct one. No weight fixes this
  without effectively deleting keyword (`VECTOR_WEIGHT > 64`).
- A real keyword bug surfaced and was fixed along the way: the FTS5 query builder joined
  terms with a space (implicit AND), so verbose natural-language queries matched nothing.
  Switching to OR lifted keyword overall recall 0.29 → 0.85.

The lesson: **rank-fusion is recall plumbing, not a precision lever.** It merges candidate
lists; it does not judge relevance. On a clean corpus it cannot beat the stronger single
retriever, and the eval cannot even *see* fusion's value because vector never fails. Chasing
a fusion formula or hand-tuned weights is guessing — the thing ADR-0005 exists to prevent.

## Decision

Production retrieval is a **staged pipeline**, and quality investment goes into the stages
in this order:

1. **Recall (now):** keyword ∪ vector, as a high-recall candidate union whose only job is
   "don't miss the right note." `hybrid_search` is **vector-primary**: vector's ranking is
   authoritative; keyword *backfills* ids vector did not return. This is provably never
   worse than vector alone, while keeping keyword as a live source for cold start (before
   background embedding finishes) and exact-token / out-of-vocab coverage.

2. **Rerank (next quality slice — the precision lever):** a local cross-encoder reranker
   (e.g. `bge-reranker-base`, ONNX via the existing `ort`/fastembed stack) scores each
   *(query, note)* pair jointly over the recall union's top ~50 and reorders. A
   cross-encoder reads query and note together, so it captures exact-match *and* semantics
   in one pass — a categorical jump over score-free fusion, and it replaces hand-tuned
   weights with a learned judgement.

3. **Generate (later):** LLM query understanding (rewriting, HyDE, multi-hop) and
   answer synthesis — the AI-native layer.

Two cross-cutting levers feed the same eval-gated process: **chunk-level embeddings**
(retire the whole-note deferral; the `buried-fact-in-long-note` category is the tripwire)
and **structure/recency signals** (a second brain knows time, links, and tags).

**Prerequisite for all of it:** grow the golden set to a realistic, noisy, multi-relevant
corpus where the embedding model actually *fails* somewhere. Until vector fails, neither
fusion, reranking, nor chunking can demonstrate measurable lift — so corpus realism is the
first dependency of every retrieval improvement, not an afterthought.

## Consequences

- `hybrid_search` is vector-primary backfill, not RRF. It re-measures to recall@3 = 1.00
  (parity with vector) and is gated as the production method. The `reciprocal_rank_fusion`
  primitive remains in `raki-retrieval` for the recall stage of a future multi-source union.
- The reranker is deferred but named, with its insertion point defined (reorder the recall
  union). It is the next retrieval quality slice, ahead of editor/notes UX if retrieval
  quality is the priority.
- We explicitly do **not** claim hybrid beats vector today. On this corpus it ties by
  construction; fusion/rerank value is unprovable until the corpus is harder. This is
  recorded honestly rather than masked by tuning the eval to make hybrid "win."
- Hand-tuned fusion weights are rejected as a quality mechanism: any future blend is either
  learned (reranker) or justified by a measured delta on a corpus that can show it.
