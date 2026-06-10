# R1 — Attach the Cross-Encoder Reranker to Production (attach-to-validate)

**Date:** 2026-06-08
**Status:** Design — approved, pending spec review
**Governing ADRs:** ADR-0005 (retrieval is measured), ADR-0006 (staged recall → rerank → generate),
ADR-0007 (retrieval quality is measurement-gated; benchmark-first). New: ADR-0008 (this attach).
**Roadmap:** `docs/ROADMAP.md` Track A — R1.

---

## Honesty clause (read first)

This attaches a retrieval lever whose **binding** verdict cannot be taken yet. The kill-switch
(`docs/eval/reranker-deletion-criteria.md`) requires the cross-encoder to beat hybrid by **+0.03
nDCG on ≥100 real-labeled queries** — real-notes ground truth that **does not exist yet**. The only
evidence in hand is **SciFact +0.0313 nDCG@10** (R0 baseline), which is **domain-shifted and
directional**, not a verdict (ADR-0007). Therefore R1 is explicitly **attach-to-_validate_**: we
wire the reranker into production on the strength of directional evidence + its local/no-egress cost
profile, keep the kill-switch armed, and defer the keep-or-delete decision to the real-notes tier
(roadmap P1). R1 does **not** build relevance-logging or the real-data capture — that is P1's job.

## What this is

Wire the existing local cross-encoder (`FastEmbedReranker`, jina-reranker-v1-turbo-en) into the
production `search_notes` path as a best-effort enhancement layered on the unchanged hybrid floor.

## What this is NOT

- Not the binding keep/delete verdict (that needs real-notes data — P1).
- Not relevance-logging / real-data capture (P1).
- Not a change to `hybrid_search`, `hybrid_candidates`, the metric primitives, or any `raki-storage`
  code — those are reused as-is. (The sole `raki-retrieval` change is a one-line defensive
  index-access hardening in the `rerank` wrapper — review #6, D3 — with no behavior change for the
  real model.)
- Not a new user-facing setting/toggle (YAGNI; reranking is local and always-on).

---

## Decisions

### D1 — `AppState` gains an optional reranker
Add `pub reranker: Option<Arc<dyn Reranker>>` to `AppState` (`src/state.rs`). Optional because the
reranker is a best-effort enhancement: search must still work when it is absent.

### D2 — Startup construction mirrors the embedder's degrade-don't-crash pattern
In `src/lib.rs`, next to the embedder block (`lib.rs:76`), construct the reranker:

```rust
let reranker: Option<Arc<dyn Reranker>> = match FastEmbedReranker::try_new() {
    Ok(r) => Some(Arc::new(r)),
    Err(e) => {
        eprintln!("reranker unavailable ({e}); search runs without reranking this session");
        None
    }
};
```

This matches the embedder's existing fallback (`FastEmbedProvider::try_new()` → fake on error): a
model that can't load degrades the feature, it does not block the app.

### D3 — `search_notes` reranks a faithful, bounded pool, with the hybrid floor as fallback
Rewrite `search_notes` (`src/commands/notes.rs:66`) to:

1. Pull the recall-union pool: `hybrid_candidates(keyword, vectors, embedder, &query, POOL)` where
   `POOL = 100` — the exact function and depth `bench` reranked, so the production path is **measured
   under the same configuration as the SciFact baseline (directly comparable)**. This is *not* a
   promise of identical lift on real notes (see Honesty clause; review #9).
2. Hydrate each pool id to a `Note`; build `(id, text)` candidate pairs where
   `text = format!("{}\n\n{}", note.title, body_to_text(&note.body))` — the same representation
   `run_benchmark` used — then **truncate `text` to `MAX_RERANK_DOC_BYTES = 4096` at a char boundary**
   before reranking (review #3). The cross-encoder consumes only ~512 tokens, so the cap bounds
   per-search memory (100 × body) and discards nothing the model would have read.
3. Rank to `K = 20` for display via a small `Duration`-parameterized helper
   `rerank_top_k(reranker, &query, &candidates, K, timeout) -> Option<Vec<String>>` that wraps the
   call in `tokio::time::timeout` and returns `Some(ids)` on success, or `None` (→ caller uses hybrid
   order) on **timeout or `Err`** (review #1). Parameterizing the timeout lets a test exercise the
   timeout arm with a 1 ms budget instead of waiting `RERANK_TIMEOUT`.
   - `Some(reranker)` → `rerank_top_k(...).unwrap_or_else(|| hybrid_order)`.
   - `None` (no reranker) → the hybrid order.
   - **Hybrid fallback order** = the pool truncated to `K` (`hybrid_candidates(POOL).truncate(K)` is
     identical to today's `hybrid_search(K)` for `POOL ≥ K`, so the fallback is bit-for-bit current
     behavior).
4. Map the final ranked ids to `NoteDto`s, reusing the already-hydrated notes (no second fetch).

Consts in `commands/notes.rs`: `POOL = 100`, `K = 20`, `MAX_RERANK_DOC_BYTES = 4096`,
`RERANK_TIMEOUT = Duration::from_secs(5)`.

**Runtime-blocking is already handled (review #2 — refuted):** `FastEmbedReranker::rerank`
(`raki-ai/src/rerank.rs:50`) runs the CPU-bound forward pass inside `tokio::task::spawn_blocking`, so
a rerank never stalls the tokio worker or other IPC commands. Reranks serialize on the reranker's
internal `Arc<Mutex<TextRerank>>`, acceptable for a single-user desktop app. The timeout in step 3
bounds the one case `spawn_blocking` does *not* cover: a genuinely hung inference stalling that single
search.

**Panic safety (review #6):** a panic inside the inference closure is already captured by
`spawn_blocking` and surfaced as a `JoinError` → `DomainError::Provider` (`rerank.rs:57`) → the `Err`
arm → hybrid fallback. The one unguarded site is `candidates[s.index]` in the `raki-retrieval` `rerank`
wrapper (`rerank.rs`): harden it to `candidates.get(s.index)` and skip out-of-range indices — a
one-line defensive change (`FastEmbedReranker` returns in-range indices, so real-model behavior is
unchanged). No `catch_unwind` is added; `spawn_blocking` already provides the unwind boundary for the
expensive path.

### D4 — Best-effort failure posture (search never breaks)
Reranking only ever improves or no-ops. Every reranker failure mode falls back to the hybrid top-`K`,
never surfacing as a search failure:
- missing reranker at startup (D2) → hybrid;
- `rerank` returns `Err` → hybrid;
- `rerank` exceeds `RERANK_TIMEOUT` → hybrid (review #1);
- inference panics → caught as `JoinError` → `Err` → hybrid (D3 panic safety);
- out-of-range score index → skipped by the hardened wrapper (D3).

### D5 — The reranker is gated by SciFact `benchmark_gate`, not CI; eval tier unchanged
- `benchmark_gate` (`tests/benchmark_gate.rs`, `#[ignore]`) is the reranker's quality sentinel —
  already asserts vector calibration + reranker plausibility. Unchanged.
- The 30-note `eval_gate` / `run_eval` stay **unchanged**. Production's method is now `reranked`, but
  the synthetic set is saturated (≈0.98) and cannot gate the reranker — exactly why SciFact exists.
  The recall floor it asserts is a property reranking preserves (reranking reorders/pulls from the
  recalled pool; it does not lower pool recall), so it still guards the recall stage. We do **not**
  add a `reranked` method to `run_eval` (no signal to gain on a saturated corpus — YAGNI).
- Consequence (stated honestly): the reranker has **no automated CI gate**, by design — its sentinel
  is `#[ignore]` and its verdict is manual/real-data. Attaching it to production does not sneak it
  past the measurement discipline; the kill-switch remains the binding test.

### D6 — Tests prove the wiring and the fallbacks (deterministic, no model)
Integration tests for `search_notes` (or the `rerank_top_k` helper) constructing an `AppState` with:
- `Some(FakeReranker)` → results returned in the fake's deterministic reranked order (rerank path
  reached and applied).
- `None` → hybrid top-`K` order (graceful degradation when no reranker).
- A test-only erroring reranker (returns `Err` from `rerank`) → hybrid top-`K` order (per-search
  error arm, distinct from the `None` arm).
- A test-only **hanging** reranker (sleeps), helper called at a **1 ms** timeout → hybrid top-`K`
  (timeout arm, review #1 — fast because the timeout is a parameter).
- A note with body ≥ `MAX_RERANK_DOC_BYTES` → candidate text truncated at a char boundary; search
  succeeds without panic (size-cap, review #3).
- Unit test in `raki-retrieval/src/rerank.rs`: a reranker returning an **out-of-range index** → that
  entry is skipped, no panic (hardened wrapper, review #6).

These run model-free via `FakeReranker` / `FakeEmbeddingProvider` (the substrate `run_eval`'s tests
already use). The existing command-test harness pattern is followed.

### D7 — Documentation
- **ADR-0008** (`docs/adr/0008-reranker-attached-attach-to-validate.md`): records the attach decision,
  its local/no-egress nature, SciFact +0.0313 as the directional basis, that
  `docs/eval/reranker-deletion-criteria.md` remains the binding real-notes test, and the best-effort
  failure posture. Follows ADR-0006/0007; supersedes nothing.
- **`docs/ROADMAP.md`** R1 → ✅, noted "attached; binding keep/delete verdict pending real-notes
  data (P1)."
- **`docs/eval/reranker-deletion-criteria.md`**: one-line status update — reranker is now
  *attached-pending-validation*, not unshipped.

### D8 — Latency awareness, fallback logging, rollback (reviews #4, #5, #7)
This is a local-first, single-user, no-egress desktop app with **no telemetry infrastructure**, so
metrics/histograms/SLO/alerting (review #5) are **out of scope** — building them would be scope creep
against the product's privacy posture. The *validation* in "attach-to-validate" is the real-notes
kill-switch (P1), measured via the eval harness, not production telemetry. Proportionate measures:
- **Latency (review #4):** in the manual `tauri dev` smoke, record search latency before vs after the
  attach on a representative corpus (a few hundred notes). Soft target: the reranked path stays
  interactive (no perceptible lag); if it doesn't, `POOL` is the dial to turn down. No numeric SLO is
  committed pre-real-data.
- **Fallback logging (review #5, lightweight):** a single `eprintln!` (matching the embedder log
  idiom) when reranking falls back, distinguishing `None` (startup), `Err`, and `timeout`. Makes
  fallback visible in dogfooding with no telemetry stack.
- **Rollback (review #7):** because the reranker is an `Option` enhancement, rollback is trivial —
  revert the `search_notes` change (or force `reranker = None`) to return to hybrid-only. Triggers:
  any reranker-caused crash, or clearly perceptible search slowness in dogfooding.

### D9 — Deferred: bulk hydration (review #8)
The 100-candidate pool hydrates with `POOL` single-row `get`s. On local indexed SQLite this is
low-millisecond (reviewer downgraded to MINOR). A bulk `get_many` would widen scope into the
`NoteRepository` trait + `raki-storage`. **Deferred and measurement-gated:** if D8's latency check
shows hydration (not the forward pass) dominates, add `get_many` then; otherwise keep the simple `get`
loop (YAGNI).

---

## Components touched

```
src/state.rs                 MODIFY  + reranker: Option<Arc<dyn Reranker>>
src/lib.rs                   MODIFY  construct reranker (degrade-on-error); pass to AppState
src/commands/notes.rs        MODIFY  search_notes: pool → hydrate(+size cap) → timeout(rerank)/fallback → DTOs;
                                     rerank_top_k helper; consts POOL/K/MAX_RERANK_DOC_BYTES/RERANK_TIMEOUT; fallback eprintln
src/commands/notes.rs(tests) MODIFY  search_notes/helper tests: Some / None / erroring / hanging(timeout) / large-body
crates/raki-retrieval/src/rerank.rs  MODIFY  harden candidates[s.index] → candidates.get(s.index) (review #6) + OOB unit test
docs/adr/0008-*.md           CREATE  attach-to-validate ADR
docs/ROADMAP.md              MODIFY  R1 → done
docs/eval/reranker-deletion-criteria.md  MODIFY  status line
```

Reused unchanged: `raki-retrieval` `hybrid_candidates` (and `rerank`'s logic, save the one index-access
hardening above), `raki-ai` (`FastEmbedReranker`, `FakeReranker`), `raki-domain` (`Reranker`,
`RerankScore`, `body_to_text`).

## Data flow

```
query
  → hybrid_candidates(keyword, vectors, embedder, query, 100)   // recall union (unchanged fn)
  → hydrate 100 notes; build (id, cap("title\n\ntext", 4096B)) via body_to_text
  → reranker?  Some → timeout(5s, rerank(query, candidates, 20))   // rerank runs in spawn_blocking
                        [timeout | Err | OOB-index → hybrid top-20]
               None → hybrid top-20
  → NoteDto[]  (reuse hydrated notes)
```

## Error handling

| Failure | Behavior |
|---|---|
| Reranker can't load at startup | `None`; search runs hybrid (logged once) |
| `rerank` errors mid-search | Fall back to hybrid top-`K` (logged); search succeeds |
| `rerank` exceeds 5 s (hang) | Timeout → hybrid top-`K` (logged); search succeeds |
| Inference panics | `JoinError` → `Err` → hybrid top-`K` (spawn_blocking unwind boundary) |
| Out-of-range score index | Skipped by hardened wrapper; remaining order used |
| Oversized note body | Truncated to 4 KB at char boundary before rerank |
| A pool id has no note (deleted mid-flight) | Skipped during hydration (as today) |
| Embedder/keyword/vector error | Propagates as today (unchanged — recall stage) |

## Testing

- Deterministic (CI): the `search_notes`/helper tests (D6 — Some / None / erroring / hanging-timeout /
  large-body) + the `raki-retrieval` OOB-index unit test; full workspace `cargo test --exclude raki`;
  `clippy -D warnings`; `fmt --check`. The 30-note `eval_gate` unchanged and green.
- Manual / model (not CI): `benchmark_gate --ignored` still passes (reranker unchanged); a manual
  `tauri dev` smoke of search confirms reranked results render and that with the model absent search
  still returns hybrid results; record search latency before vs after the attach (review #4).

## Definition of Done

1. `search_notes` reranks the 100-pool (each candidate text capped at `MAX_RERANK_DOC_BYTES`) to
   top-20 when a reranker is present; falls back to the exact current hybrid order when it is absent,
   errors, or exceeds `RERANK_TIMEOUT`.
2. All deterministic D6 tests pass (Some / None / erroring / hanging-timeout / large-body + the
   `raki-retrieval` OOB-index unit test); full deterministic suite + clippy + fmt green.
3. `benchmark_gate --ignored` still passes (no reranker regression).
4. The `raki-retrieval` `rerank` wrapper indexes via `candidates.get(s.index)` (no panic on OOB);
   reranking runs under a timeout; fallback events are logged.
5. ADR-0008 written; ROADMAP R1 marked done; kill-switch status line updated; before/after search
   latency recorded in the manual smoke.
6. The reranker is attached **as attach-to-validate** — kill-switch armed, binding verdict explicitly
   deferred to real-notes data (P1). No relevance-logging or telemetry added (reviews #5, #8 scoped out).
