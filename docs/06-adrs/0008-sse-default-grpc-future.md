# ADR 0008. SSE as the only v0.1 streaming framing; NDJSON optional in v0.2+; gRPC bidi deferred to v1.0+

> **Date:** 2026-05-03
> **Status:** accepted
> **Options doc:** [008-stream-framing](../05-options/008-stream-framing.md)
> **Deciders:** Sriram Popuri

## Context

Riftgate must stream LLM tokens to clients. Full exploration of candidates (SSE, NDJSON, gRPC bidi, WebSockets, raw chunked) and the tradeoff matrix live in [Options 008](../05-options/008-stream-framing.md).

The forces summarized: OpenAI's `/v1/chat/completions` defines the de-facto streaming standard as SSE, and [FR-004](../01-requirements/functional.md) commits us to SSE in `v0.1`; HTTP/2 (the prerequisite for gRPC bidi) is a future-milestone deliverable per [ADR 0007](0007-handrolled-fsm-parser.md); WebSockets bring connection-state and deployment complexity without OpenAI compatibility; raw chunked loses the event boundaries we need for replayable WAL records ([NFR-OBS06](../01-requirements/non-functional.md)).

## Decision

**`v0.1` ships SSE (`text/event-stream`) as the only client-facing streaming framing.**

- `crates/riftgate-parser::sse::SseFramer` is a hand-rolled, table-driven FSM that produces `SseToken` events from chunked HTTP/1.1 body bytes. Per [ADR 0007](0007-handrolled-fsm-parser.md), this lives alongside `Http1Parser` in the same crate.
- The framer handles the W3C SSE edge cases: CRLF/LF normalization, comment lines (`:`-prefixed), multi-line `data:` events (joined by `\n`), the `[DONE]` sentinel that OpenAI uses, and heartbeat events (empty `data:` line).
- Upstream → client framing is zero-copy where possible: the upstream's chunked-encoded SSE bytes pass through to the client with framing massage only at the boundaries (e.g. for inserting a synthesized event such as a Riftgate-emitted error mid-stream).
- Bounded backpressure: the framer never buffers more than `RIFTGATE_SSE_EVENT_BUFFER` events ahead of the slowest client (default 16). When the buffer fills, the framer applies backpressure upstream.
- WAL records ([NFR-OBS06](../01-requirements/non-functional.md)) store SSE event boundaries explicitly so `riftgate-replay` can reconstruct the original event sequence.

**`v0.2`+** adds NDJSON optionally:

- `crates/riftgate-parser::ndjson::NdjsonFramer` ships behind `--features ndjson-framing`.
- The trait surface is the same `StreamFramer` shape used by `SseFramer`.

**`v1.0+`** revisits gRPC bidirectional streaming:

- Gated on HTTP/2 landing (a future Options doc + ADR pair, captured as [Options 027](../05-options/README.md) optional).
- Captured as a known extension point in [Vision §8](../00-vision.md).

**`v0.x`** does not ship WebSockets, raw chunked-without-events, or any other client-facing streaming framing.

## Consequences

- **Positive:**
  - Riftgate is OpenAI-compatible by default; clients pointed at OpenAI work against Riftgate without any configuration change.
  - The framer is a small, contained FSM that fits naturally into the parser model from [ADR 0007](0007-handrolled-fsm-parser.md).
  - Per-event framing overhead is small (~6 bytes for `data:` prefix), well within the latency budgets in [NFR-P05](../01-requirements/non-functional.md) and [NFR-P06](../01-requirements/non-functional.md).
  - WAL replay is straightforward: SSE event boundaries are explicit on the wire and in the WAL.
  - `tcpdump` of a Riftgate ↔ client stream is human-readable, which is a real operability win.
- **Negative / accepted tradeoffs:**
  - We cannot serve gRPC-bidi clients in `v0.x`. Operators with bidirectional streaming requirements must wait for HTTP/2 + gRPC integration in a future milestone.
  - We cannot serve WebSocket clients without an explicit WebSocket framer impl, which is not on the roadmap.
  - Per-event framing overhead vs NDJSON is small but non-zero; users who care will need to wait for NDJSON in `v0.2+`.
  - The SSE edge cases (CRLF, comments, `[DONE]`, heartbeats) require explicit test coverage; we accept the maintenance.
- **Future work this enables:**
  - NDJSON in `v0.2+` as a small contained addition.
  - gRPC bidi in `v1.0+` as a larger deliverable gated on HTTP/2.
  - MCP-aware framing in `v0.5` per [Options 026](../05-options/026-mcp-orchestration.md), reusing the same `StreamFramer` trait shape.
  - Per-stream framing metrics (events-emitted, framer-errors-by-class, backpressure-events).
- **Future work this forecloses (until superseded):**
  - We will not ship WebSockets as a Riftgate framing in `v0.x`.
  - We will not ship raw chunked (no event layer) as a Riftgate framing.
  - We will not invent a Riftgate-specific framing extension that would break OpenAI compatibility.

## Compliance

- `crates/riftgate-parser::StreamFramer` is the trait that all framing impls implement.
- `crates/riftgate-parser::sse::SseFramer` is the `v0.1` impl and the only one shipped by default.
- W3C SSE conformance tests live in `crates/riftgate-parser/tests/sse_conformance.rs` and cover the edge cases enumerated in [Options 008 §3.1](../05-options/008-stream-framing.md).
- A property-based test (`proptest`) verifies that any valid SSE byte stream, split at arbitrary boundaries, produces the same event sequence.
- WAL serialization for SSE events lives in `crates/riftgate-replay::sse_record.rs` and is round-trip tested.
- Adding a new `StreamFramer` impl requires a new ADR superseding (or amending) this one and passing the conformance suite.

## Notes

- The decision is heavily constrained by OpenAI's de-facto standard. If LLM serving had not standardized on SSE, NDJSON would have been a stronger candidate; the small framing-overhead win is real but irrelevant when the entire ecosystem speaks SSE.
- The `[DONE]` sentinel that OpenAI emits at the end of a streaming response is a convention layered on top of SSE, not part of the W3C spec. The framer recognizes it explicitly and emits an `SseDone` event so downstream code does not have to string-match.
- The framer's relationship to `Http1Parser` (per [ADR 0007](0007-handrolled-fsm-parser.md)) is that `Http1Parser` produces `BodyChunk` events from chunked-encoded HTTP/1.1 bodies; `SseFramer` consumes `BodyChunk` and produces `SseToken` events. The separation lets us reuse the body parser for non-SSE streaming responses (NDJSON in `v0.2+`).
- Future MCP framing ([Options 026](../05-options/026-mcp-orchestration.md)) likely reuses the SSE substrate (MCP-over-SSE is a real transport). The `StreamFramer` trait shape is intentionally generic enough to accommodate.
