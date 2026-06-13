# ADR-0001: Provider-agnostic AI (local + cloud, user-selectable)

- **Status:** Accepted
- **Date:** 2026-06-04
- **Deciders:** Raki founding team
- **Tags:** ai, privacy, architecture

## Context

Raki's intelligence (embeddings + completions) could be implemented as: fully embedded local inference,
a single cloud provider, or a pluggable abstraction over both. The product is **local-first** for *data*,
but the team explicitly does **not** require fully-offline *AI*. Users should be able to choose, swap, and
mix providers — run everything locally for privacy, or use a powerful cloud model when they want it — and
that choice may differ per capability (e.g., local embeddings + cloud chat).

Forces:
- Privacy & data ownership are core values, but so is giving users the best model when they opt in.
- The local AI ecosystem (Ollama, fastembed, candle/mistral.rs) and cloud APIs both move fast; we must not
  couple the codebase to any one.
- The memory/retrieval layer must remain testable without a live model.

## Decision

We will define provider **ports** in `raki-domain` — `EmbeddingProvider` and `LlmProvider` — and implement
them as swappable **adapters** in `raki-ai` for both **local** (Ollama, fastembed) and **cloud**
(OpenAI/Anthropic-compatible) backends. The user selects the active provider **per capability** at runtime.
All model access flows through `raki-ai`, which owns a **registry**, **retries/backoff**, and an
**egress/consent policy** (local providers need no consent; cloud providers require per-provider consent and logging of what left the device).

## Consequences

**Positive**
- No vendor lock-in; new providers are additive (a new adapter), not invasive.
- Memory/retrieval services depend on traits, so they unit-test with fake providers (no model, no network).
- Privacy becomes a single enforceable choke point: every cloud call carries an approved `EgressDecision`.

**Negative / costs**
- An abstraction layer to maintain, and capability negotiation (streaming, tools, dimensions) across providers.
- Different embedding models have different dimensions/quality → we must track `model_version` and re-embed on change.

**Neutral / follow-ups**
- Default local embeddings via fastembed so retrieval works with zero setup; cloud is opt-in.
- Local providers are identified by `Locality::Local` and bypass consent/audit; cloud providers are identified by `Locality::Cloud` and require consent plus audit logging.

## Alternatives considered

- **Fully embedded, pure-Rust only** — truest offline, self-contained binary, but larger/slower to evolve and
  denies users powerful cloud models they may want. Rejected as the *only* option (still supported as one adapter).
- **Single cloud provider** — simplest to build, but violates privacy values and creates hard lock-in. Rejected.

## References

- `AGENT.md` §1 (values), §6 (ports/adapters), §8 (egress), §12 (privacy review).
