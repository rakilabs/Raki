# ADR-0005: Local-first data ≠ local-only LLM — Egress policy correction

- **Status:** Accepted
- **Date:** 2026-06-10
- **Deciders:** Raki founding team
- **Tags:** ai, privacy, architecture, correction

## Context

The initial egress design (ADR-0001, Slice-1 substrate spec) introduced a `Mode` enum with two states:
`LocalOnly` and `CloudAllowed`. In this design, `LocalOnly` hard-blocked all LLM calls, and
`CloudAllowed` was required before any provider could be used. This created a category error:

- **"Local-first"** in Raki means *user data lives on the user's device* (SQLite file, not a server).
- It does **not** mean "disable all LLM communication."
- LLM communication is *always* mediated through a **provider** — the only question is *where that provider runs*.

The correct model:
- **Local providers** (Ollama, fastembed, candle/mistral.rs) run on the user's own machine. Data never leaves the device. No explicit consent is needed because there is no egress.
- **Cloud providers** (Kimi, Claude, OpenAI, etc.) run on remote servers. Data leaves the device. Explicit per-provider consent is required, and every call is logged in the audit trail.

The old `Mode::LocalOnly` conflated "data is local" with "no AI communication," which made the app unusable for QA when the user had a local model configured. Conversely, `Mode::CloudAllowed` was a coarse global switch that did not distinguish between a local Ollama instance and a remote API.

## Decision

We will **remove the `Mode` enum and the global `local_only`/`cloud_allowed` switch** entirely.
The egress gate (`GatedLlmProvider`) will inspect the **inner provider's `locality()`** at call time:

- **`Locality::Local`** → the call is **always allowed** and **not logged in the egress audit log** (nothing left the device). The gate delegates directly to the inner provider.
- **`Locality::Cloud`** → the call is **allowed only if the provider is in the user's consented set**; on approval, it is executed and logged in the egress audit log. On denial, `EgressError::Denied(EgressDenied::ConsentRequired)` is returned.

`EgressSettings` retains only `consented()`, `grant(provider)`, and `revoke(provider)`.
The `app_settings` kv table's `egress_mode` row becomes unused (we do not delete it in a migration; we simply stop reading it).

## Consequences

**Positive**
- A user with Ollama configured can use QA immediately, with no consent flow, because nothing leaves the device.
- A user with a cloud provider configured sees the same consent flow as before, but without the misleading "mode" abstraction.
- The architecture correctly models the actual distinction: *where the provider runs*, not whether the app communicates at all.
- The `LlmProvider::locality()` port already carries this information; the gate now uses it instead of duplicating it in a separate settings layer.

**Negative / costs**
- Breaking change to `EgressSettings` trait and `GatedLlmProvider` internals. All test fakes must be updated.
- The frontend Settings panel loses the "mode" radio switch; it becomes a simpler "cloud provider consent" list.
- Old docs, specs, and the AGENTS.md egress section reference `Mode` and must be updated.

**Neutral / follow-ups**
- Future multi-provider support: a user may have *both* Ollama and Kimi configured. The gate handles this naturally — Ollama calls bypass consent, Kimi calls require it.
- The `app_settings` table may be repurposed later for actual app settings (theme, default provider, etc.). The orphaned `egress_mode` row is harmless.

## Alternatives considered

- **Keep `Mode` but rename it** (e.g., `NetworkOff` / `NetworkOn`) — Rejected. The locality of the provider already encodes whether network egress happens. A second switch is redundant and error-prone.
- **Log local calls too, with a `local` flag** — Rejected. The egress log's contract is "what left my device?" Logging local calls would make the log useless for answering that question.
- **Separate gate for local vs cloud** — Rejected. A single `GatedLlmProvider` that branches internally is simpler and preserves "all completions go through one type" as an architectural invariant.

## References

- ADR-0001: Provider-agnostic AI (local + cloud, user-selectable)
- `AGENTS.md` §1 (product values), §8 (AI Memory Layer design rules), §12 (privacy & egress)
- Slice-1 egress substrate spec (`docs/superpowers/specs/2026-06-07-egress-context-substrate-design.md`)
- Slice-2 grounded-QA design spec (`docs/superpowers/specs/2026-06-07-slice2-grounded-cloud-qa-design.md`)
