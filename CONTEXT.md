# Raki domain context

Raki is a local-first, AI-native second brain. This file records the product's domain language so that architecture discussions use one vocabulary.

## Language

**Second brain**:
A personal knowledge system that unifies notes, tasks, and eventually other life data, with an AI layer that retrieves and connects information while keeping data local and user-owned.
_Avoid_: knowledge base, vault, notebook.

**Block**:
A ProseMirror node with a stable ID; the atomic unit of editing, chunking, and block-level linking.

**Chunk**:
A retrieval unit derived from one or more blocks; what gets embedded and indexed.

**Memory**:
An AI-derived atomic fact, preference, or entity with provenance, confidence, and lifecycle state.

**Entity / Link**:
Nodes and edges of the cross-module knowledge graph that connect notes, tasks, finance records, and other domain objects.

**Provider**:
A swappable source of embeddings or completions, which may be local or cloud. Adapters implement the provider ports defined in `raki-domain`.
_Avoid_: model, backend, vendor.

**Egress**:
Any data that would leave the device on a cloud provider call. It is assembled, approved, and logged before sending.

**AssembledContext**:
The single, deterministic, token-budgeted bundle of source material a model is allowed to see.

**Grounded answer**:
A response to the user's question that is constrained to the AssembledContext and cites its sources. The grounded answer flow is retrieve → assemble → gate → answer → verify.

**Groundedness**:
The verdict on whether an answer actually cites sources present in the AssembledContext. Possible states: Grounded, Ungrounded, NothingMatched.

**AnswerService**:
The `raki-memory` module that orchestrates the grounded answer flow. It depends only on domain ports and is testable with fake adapters.
_Avoid_: QA service, question-answering service.

**Gated provider**:
The domain port that wraps an `LlmProvider` with the egress policy: check consent, record what would leave the device, then complete. Implemented in `raki-ai`; consumed by `AnswerService` and `CloudQueryRewriter`.
_Avoid_: gate as a verb without naming the seam.

**Port / Adapter**:
A domain trait (port) and its concrete implementation (adapter). The basis of testability and provider swapping.

## Flagged ambiguities

- **Answer** without qualification can mean either the raw model text or the verified grounded answer. Prefer "grounded answer" for the product outcome and "completion" for the raw model output.
- **QA** is shorthand for question answering but hides the grounding constraint. Prefer "grounded answer" when talking about the user-facing feature.

## Example dialogue

> Dev: The user asks "how do I pay at the inn?" What happens?
>
> Domain expert: `AnswerService` retrieves chunks, builds an `AssembledContext`, checks `Egress` consent, asks the `Provider` for a completion, then runs `Groundedness` verification on the reply.
>
> Dev: Where does the gating happen?
>
> Domain expert: Inside `AnswerService`, using the `EgressLog`, `EgressSettings`, and `LlmProvider` ports. No crate outside `raki-ai` knows how consent is stored.
