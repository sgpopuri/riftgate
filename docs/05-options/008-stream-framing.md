# 008. Stream Framing

> **Status:** `recommended` — SSE (`text/event-stream`) as the only client-facing streaming framing in `v0.1`; NDJSON considered as a `v0.2`+ opt-in; gRPC bidirectional streaming and WebSockets deferred to `v1.0+` per [Vision §8](../00-vision.md). See [ADR 0008](../06-adrs/0008-sse-default-grpc-future.md).
> **Foundational topics:** ring buffers and zero-copy I/O on the response path, FSM-based streaming framers (line-delimited and length-prefixed)
> **Related options:** [001](001-io-model.md) (IO model), [007](007-protocol-parser.md) (protocol parser), [027](README.md) (upstream protocols, optional)
> **Related ADR:** [ADR 0008](../06-adrs/0008-sse-default-grpc-future.md)

## 1. The decision in one sentence

> What wire format does Riftgate use to stream LLM tokens to clients (and to receive them from upstream OpenAI-compatible backends), and what alternative formats are kept available behind a feature flag?

## 2. Context — what forces this decision

LLM serving is dominated by streaming responses: clients want tokens as they're generated, not after the entire completion is done. The choice of wire format affects every byte that crosses Riftgate's data plane, the parser shape ([Options 007](007-protocol-parser.md)), the timer wheel ([`docs/04-design/lld-timers.md`](../04-design/lld-timers.md)) for stream deadlines, and the WAL semantics ([`docs/04-design/lld-storage.md`](../04-design/lld-storage.md)) for replayable streams.

Forces driving this decision:

- **OpenAI defines the de-facto standard.** The `/v1/chat/completions` endpoint with `stream: true` returns Server-Sent Events. Every OpenAI-compatible backend (vLLM, SGLang, Ollama, etc.) implements SSE for streaming. [FR-004](../01-requirements/functional.md) commits us to SSE in `v0.1`.
- **TTFT and inter-token latency are user-visible.** [NFR-P05](../01-requirements/non-functional.md) targets <5 ms TTFT overhead; [NFR-P06](../01-requirements/non-functional.md) targets <500 µs inter-token. The framing's per-event overhead matters at these budgets.
- **Streaming framing must be FSM-friendly.** The same hand-rolled, table-driven parser shape from [ADR 0007](../06-adrs/0007-handrolled-fsm-parser.md) applies here; the framer is the FSM that turns a chunked HTTP body into typed `SseToken` events.
- **Zero-copy on the response path.** The framer should pass the upstream's bytes through to the client with the minimum possible touching: change the chunked-encoding boundary, optionally inject a header or trailer, but do not re-format the JSON inside.
- **Backpressure must propagate.** A slow client must propagate backpressure upstream so we don't OOM buffering tokens for a stalled connection. The framing should not buffer more than a bounded number of events ahead.
- **Replayability.** The framing should be replayable from the WAL ([NFR-OBS06](../01-requirements/non-functional.md)) — the WAL stores the event boundaries, not just the bytes, so `riftgate-replay` can re-emit the original event sequence.
- **Future MCP support.** [Options 026](026-mcp-orchestration.md) recommends gateway-level MCP parsing; MCP uses JSON-RPC over various transports. The framing decision should keep the door open for a JSON-RPC-shaped streaming mode in `v0.5+`.

## 3. Candidates

### 3.1. Server-Sent Events (SSE, `text/event-stream`)

**What it is.** A simple text-based framing for server-to-client streaming over HTTP. Each event is a sequence of UTF-8 lines, separated by `\n\n` from the next event. Lines are typed by prefix: `data:` for payload, `event:` for event type, `id:` for event ID, `retry:` for reconnect timing. The connection is a long-lived HTTP response with `Content-Type: text/event-stream`. The W3C standardized SSE in 2009; OpenAI adopted it for streaming completions in 2023.

**Why it's interesting.**
- **The standard.** OpenAI's `/v1/chat/completions` with `stream: true` returns SSE. Every OpenAI-compatible backend speaks SSE. [FR-004](../01-requirements/functional.md) is non-negotiable.
- **Trivial framing.** A `\n\n` separator is the entire framer. State machine has ~5 states (in-event, in-data-line, in-event-line, awaiting-empty-line, done).
- **Reconnect built-in.** SSE's `id:` and `retry:` lines support client-side reconnect with last-seen-event resumption. Useful for long-lived streams.
- **Browser-native.** `EventSource` is a standard JavaScript API; web clients work without library glue. (Less relevant for Riftgate's typical workload but a free win.)
- **Plays well with chunked encoding.** SSE is just a body content type; HTTP/1.1 chunked transfer-encoding handles the actual byte framing on the wire.
- **Existing `SseFramer` slot in `riftgate-parser`** ([`docs/04-design/lld-parsing.md`](../04-design/lld-parsing.md)) — the LLD already commits to SSE in `v0.1`.

**Where it falls short.**
- **Text-based.** Every payload is UTF-8 JSON wrapped in `data:` prefixes; binary data must be base64-encoded. For LLM tokens this is fine (text-shaped), but it precludes efficient transport of audio / image streaming if Riftgate ever supports those.
- **Headers per event.** `data:` prefix on every payload line is ~6 bytes of framing overhead per event. At thousands of events per second, this is small but non-zero.
- **No bidirectional streaming.** SSE is server-to-client only. Client-to-server uses standard HTTP request bodies. For interactive workloads (live tool-use turns, multi-step agent conversations), this is a real limitation; for completion-style streaming, it does not matter.
- **CORS quirks** for browser clients (preflight, credentials handling). Not a Riftgate problem in `v0.1` but a future browser-deployment consideration.
- **Event-boundary parsing is the framer's job.** Edge cases (CRLF vs LF line endings, comments starting with `:`, multi-line `data:` events that should be re-joined with `\n`) all need explicit handling. The W3C spec is precise; the implementations vary.

**Real-world systems that use it.** OpenAI, Anthropic (since 2024 SSE-mode), Google Vertex AI streaming, vLLM, SGLang, Ollama, ChatGPT's UI internally, every Server-Sent-Events Wikipedia article since 2009.

**Wire example.**
```
data: {"id":"chatcmpl-1","choices":[{"delta":{"content":"Hello"}}]}

data: {"id":"chatcmpl-1","choices":[{"delta":{"content":", world"}}]}

data: [DONE]

```

### 3.2. NDJSON / JSONL (`application/x-ndjson`)

**What it is.** Newline-delimited JSON. Each event is a complete JSON object on a single line, terminated by `\n`. No `data:` prefix, no event types, no event IDs. Just JSON lines.

**Why it's interesting.**
- **Simpler than SSE.** No framing prefix, no event metadata, no `\n\n` separator. Just `<json>\n<json>\n<json>\n`.
- **Less framing overhead.** Saves the ~6 bytes of `data:` prefix per event.
- **Easier to replay from the WAL.** Each line is a self-contained record; no context needed for parsing.
- **Familiar to data-engineering audiences.** NDJSON is the standard for log streams, JSONL training data, and many ETL pipelines.
- **Some backend support.** Anthropic offered NDJSON streaming as an alternative to SSE in early API versions; some self-hosted backends offer it.

**Where it falls short.**
- **Not the OpenAI standard.** A client that expects OpenAI-shape SSE will not parse NDJSON without explicit support.
- **No reconnect semantics.** No event IDs, no replay-from-last-seen support.
- **Same "text-based" limitation as SSE.** Binary data is base64-encoded.
- **Browsers do not have a native NDJSON streaming API.** Application code must read the chunked body and split on `\n` manually. (Same complexity as SSE for non-`EventSource` consumers, but no built-in `EventSource`.)
- **Less ecosystem inertia.** Tooling, documentation, examples — all lean SSE for LLM streaming.

**Real-world systems that use it.** Anthropic's older API (now SSE), some self-hosted backends, HTTP-streaming variants of Elasticsearch's `_bulk` endpoint, log-shipping services.

### 3.3. gRPC bidirectional streaming (HTTP/2 frames)

**What it is.** gRPC over HTTP/2's bidirectional streaming. Each "message" is a length-prefixed protobuf frame inside an HTTP/2 stream. Clients and servers both can send messages on the same stream concurrently. The HTTP/2 layer handles flow control, framing, and connection multiplexing.

**Why it's interesting.**
- **Bidirectional.** A client can stream tokens as it generates a multi-turn dialogue; the server can stream responses back; both share the same connection. Powerful for agentic workloads.
- **Binary framing.** Length-prefixed frames are cheaper to parse than text-line-delimited streams. No CRLF handling, no UTF-8 decoding on the framer's hot path.
- **HTTP/2 multiplexing.** Many concurrent streams over one connection — important if Riftgate is ever a sidecar in a service mesh where connection setup matters.
- **Mature Rust support.** `tonic` is the canonical gRPC crate; battle-tested.

**Where it falls short.**
- **Not the OpenAI standard.** Adopting gRPC bidi as the primary client-facing format would require every client to switch off the OpenAI-compatible HTTP/SSE shape. This is a non-starter.
- **Requires HTTP/2 throughout.** [Options 007](007-protocol-parser.md) defers HTTP/2 to a future milestone; gRPC bidi cannot ship before that.
- **Operational complexity.** HTTP/2 brings flow control windows, settings frames, ping/pong, GOAWAY semantics — all of which are real engineering work to handle correctly.
- **Less observable on the wire.** A `tcpdump` of an HTTP/2 stream is largely opaque; SSE is human-readable.
- **Schema discipline.** gRPC requires a `.proto` file; semantics changes are versioned through that file, with all the protobuf-style discipline. For Riftgate this is a feature in some senses but a load-bearing commitment.

**Real-world systems that use it.** Most internal-service streaming inside Google, Cloudflare's some-Pingora-internal traffic, dgraph, and many vendor-specific gRPC streaming APIs. Most LLM gateways do not.

### 3.4. WebSockets

**What it is.** Full-duplex bidirectional binary/text framing over a single TCP connection, upgraded from HTTP/1.1. Defined by RFC 6455. Connection starts as HTTP/1.1, upgrades to WebSocket via the `Upgrade: websocket` header dance, then exchanges length-prefixed frames.

**Why it's interesting.**
- **Bidirectional.** Same advantage as gRPC bidi but on HTTP/1.1, so no HTTP/2 prerequisite.
- **Browser-native.** `WebSocket` is a standard JavaScript API; some web LLM UIs use it.
- **Per-frame control.** Text vs binary frames distinguished at the protocol layer, no base64 needed for binary payloads.

**Where it falls short.**
- **Not the OpenAI standard.** Same fatal objection as gRPC and NDJSON.
- **Connection-state-heavy.** WebSocket connections are long-lived; per-connection state is more than HTTP/SSE's request-scoped state. Per-connection memory budget must accommodate ([NFR-P04](../01-requirements/non-functional.md)).
- **Proxy and firewall semantics are messier.** Many HTTP-aware proxies do not pass WebSocket Upgrade headers cleanly; deployment is finicky.
- **No replay semantics out of the box.** A WebSocket connection does not have HTTP request/response framing for the WAL ([NFR-OBS06](../01-requirements/non-functional.md)).
- **The Upgrade dance complicates the parser.** `Http1Parser` would need an Upgrade-aware mode that passes the connection to a separate WebSocket framer.

**Real-world systems that use it.** Some web LLM UIs, real-time collaboration tools (Figma, Notion), gaming, financial trading dashboards. Almost no LLM gateway uses WebSockets as the primary client-facing format for streaming completions.

### 3.5. Raw HTTP/1.1 chunked transfer-encoding (no event framing)

**What it is.** Just stream the raw response body as HTTP/1.1 chunked transfer-encoding, with no event framing on top. The client receives chunks as they arrive, one chunk per token (or several tokens batched).

**Why it's interesting.**
- **Simplest possible framing.** Chunked transfer-encoding is already in HTTP/1.1. No event prefixes, no `\n\n` separators.
- **Lowest framing overhead.** Just the chunk-length lines and the body bytes.
- **Useful for non-event payloads.** If we ever want to stream raw text (no JSON envelope), this is the format.

**Where it falls short.**
- **No event boundaries on the wire.** Clients have no protocol-level way to know "this chunk is one token, this chunk is two." Every consumer has to know the application-level framing convention out-of-band.
- **Not the OpenAI standard.** Same fatal objection.
- **Hard to correlate with the WAL.** Without explicit event boundaries, the WAL cannot store discrete events.
- **No bidirectional support.**

**Real-world systems that use it.** Pre-SSE streaming endpoints (early HTTP/1.1 streaming dashboards), some specialized text-streaming endpoints. Almost no production LLM streaming uses raw chunked.

## 4. Tradeoff matrix

| Property | SSE | NDJSON | gRPC bidi | WebSockets | Raw chunked | Why it matters |
|----------|-----|--------|-----------|------------|-------------|----------------|
| OpenAI-compatible | yes | no | no | no | no | [FR-004](../01-requirements/functional.md) is non-negotiable in `v0.1`. |
| HTTP/1.1 only (`v0.1` constraint) | yes | yes | no (HTTP/2) | yes (Upgrade) | yes | HTTP/2 is a future milestone. |
| Per-event framing overhead | ~6 bytes (`data:`) | ~0 bytes (just `\n`) | ~5 bytes (length prefix) | ~2-14 bytes (frame header) | 0 (just chunk length) | At thousands of events per second, small. |
| FSM-friendly | very (line-delimited, simple states) | very (line-delimited) | medium (binary frames) | medium (binary frames) | trivial (no event layer) | [ADR 0007](../06-adrs/0007-handrolled-fsm-parser.md). |
| Zero-copy passthrough from upstream | yes | yes | yes (binary) | yes (binary) | yes | Hot-path memory bandwidth. |
| Bidirectional streaming | no | no | yes | yes | no | Future agentic workloads benefit. |
| Replay-from-WAL semantics | yes (event boundaries) | yes (line boundaries) | yes (length-prefixed) | possible (frame boundaries) | poor (no event layer) | [NFR-OBS06](../01-requirements/non-functional.md). |
| Reconnect / resume support | yes (`id:`, `retry:`) | no | yes (HTTP/2 stream resumption is application-level) | no (must build it) | no | Useful for long-lived streams. |
| Browser-native client API | yes (`EventSource`) | no | no (gRPC-Web is a thing but separate) | yes (`WebSocket`) | no | Not Riftgate's primary deployment. |
| Wire observability (`tcpdump`, `tcpflow`) | very good (text) | very good (text) | poor (binary) | medium (binary) | medium (text but no event layer) | At 3am on a pager. |
| Engineering cost in `v0.1` | low (FSM is simple) | low | very high (HTTP/2 stack) | medium (Upgrade dance) | trivial | Capacity-bounded. |
| Compatibility with `SseFramer` slot in [`lld-parsing`](../04-design/lld-parsing.md) | natural | possible (NDJSON framer) | requires new impl | requires new impl | trivial | Trait shape is set. |
| Compatibility with future MCP framing ([Options 026](026-mcp-orchestration.md)) | medium (MCP is JSON-RPC over various transports) | medium | natural (gRPC bidi for MCP) | natural | poor | `v0.5` consideration. |

## 5. Foundational principles

**Ring buffers and zero-copy I/O on the response path.** The streaming-pipeline literature (LMAX Disruptor design notes, the kernel-level `splice(2)` / `sendfile(2)` documentation, MSG_ZEROCOPY) argues that streaming pipelines should pass byte buffers through with minimum copies: the upstream's chunk arrives in our I/O buffer, we framing-detect on the boundary, we hand the original buffer to the response writer with the framing massaged at the edges only. The relevant copies-per-event accounting puts SSE-with-zero-copy at one (the `data:` prefix is the only thing we have to add) and NDJSON-with-zero-copy at zero (the upstream's bytes are usable verbatim if the upstream emits NDJSON). For the OpenAI use case, the upstream emits SSE, so the copies-per-event for SSE is also zero.

**FSM-based streaming framers.** A streaming framer is a finite state machine, not an event aggregator. It should yield events as soon as boundaries are unambiguous, and never buffer more than the smallest possible amount. SSE's two-character separator (`\n\n`) makes this trivial; NDJSON's single-character separator (`\n`) makes it trivial-er. WebSocket length-prefixed frames make it bookkeeping-heavy but still streaming; gRPC bidi's HTTP/2 framing makes it framing-on-framing.

**Edge cases on the wire.** The W3C SSE specification, the OpenAI streaming docs, and the experience embedded in `EventSource` implementations all enumerate the half-dozen SSE edge cases that must be in the test suite: CRLF vs LF line endings, comments (`:` lines), multi-line `data:` events that should be re-joined with `\n`, the `[DONE]` sentinel OpenAI emits, the difference between an empty event and a heartbeat, very-large events that exceed the chunk boundary.

## 6. Recommendation

**`v0.1` ships SSE (`text/event-stream`) as the only client-facing streaming framing. The `SseFramer` impl in `riftgate-parser` ([`docs/04-design/lld-parsing.md`](../04-design/lld-parsing.md)) is the table-driven FSM that produces `SseToken` events from chunked HTTP body bytes. NDJSON, gRPC bidi, and WebSockets are not in `v0.1`.**

**`v0.2`+ optionally adds NDJSON behind a `--features ndjson-framing` flag for users with NDJSON-emitting backends or NDJSON-consuming clients. This is a small, contained addition.**

**`v1.0+` revisits gRPC bidirectional streaming when HTTP/2 lands and when an external request emerges (likely from agentic workloads needing duplex). Captured in [Vision §8](../00-vision.md) as a known extension point.**

The reasoning, restated:

- SSE is the OpenAI standard; [FR-004](../01-requirements/functional.md) is non-negotiable. Every alternative loses the OpenAI-compatibility benefit.
- The `SseFramer` is a small, contained FSM that fits naturally into [ADR 0007](../06-adrs/0007-handrolled-fsm-parser.md)'s parser model. The marginal cost over "no framer at all" is small.
- NDJSON is similar enough to SSE in framing and so dramatically less costly than gRPC or WebSockets that we can hold it open for `v0.2+` as a near-trivial addition. We do not commit to it; we keep the door open.
- gRPC bidi is the right answer for *some* future agentic workload, but it requires HTTP/2, which is a future milestone. Lock-in risk: low (the trait surface accommodates it).
- WebSockets and raw chunked are out: WebSockets' connection-state cost and Upgrade-dance complexity buy us nothing for OpenAI-compatible streaming; raw chunked has no event boundaries for the WAL.

### Conditions under which we'd revisit

- An external user emerges with a strong NDJSON requirement (an Anthropic-NDJSON-mode backend that we want to proxy). The opt-in NDJSON path becomes work for `v0.2+`.
- HTTP/2 lands ([Options 027](README.md), gated). gRPC bidi becomes a candidate for an agentic-workload deliverable in `v1.0+`.
- A web-UI deployment requires WebSockets specifically (browser SSE limits, some corporate-proxy quirks). We would consider a WebSocket-as-Upgrade impl as an opt-in.

### What stays available behind feature flags

- `--features ndjson-framing` in `v0.2+` (opt-in NDJSON `StreamFramer` impl).
- `--features grpc-bidi` in `v1.0+` (gated on HTTP/2 + tonic integration).
- `--features websocket-framing` not on the roadmap; possible if external pull emerges.

## 7. What we explicitly reject

- **NDJSON as the `v0.1` default.** Loses OpenAI compatibility; small win not worth the deviation.
- **gRPC bidi as the `v0.1` or `v0.2` default.** Requires HTTP/2; not on the parser roadmap until later milestones.
- **WebSockets as the primary or default streaming framing.** Connection-state cost, Upgrade complexity, deployment-environment quirks, no replay semantics out of the box, no OpenAI client expectation.
- **Raw chunked with no event layer.** Loses WAL replay semantics; no client expects this; no win.
- **Multiple framings as `v0.1` opt-ins simultaneously.** Engineering cost without payoff; one format, well-tested, in `v0.1`.
- **Reinventing SSE with a Riftgate-specific extension.** The whole point of compatibility is that we look like every other OpenAI-compatible upstream.

## 8. References

1. W3C, *Server-Sent Events* (current spec) — https://html.spec.whatwg.org/multipage/server-sent-events.html
2. OpenAI streaming completions documentation — https://platform.openai.com/docs/api-reference/chat/streaming
3. NDJSON specification — https://github.com/ndjson/ndjson-spec
4. RFC 7540, *Hypertext Transfer Protocol Version 2 (HTTP/2)* — https://www.rfc-editor.org/rfc/rfc7540
5. RFC 6455, *The WebSocket Protocol* — https://www.rfc-editor.org/rfc/rfc6455
6. gRPC over HTTP/2 protocol — https://github.com/grpc/grpc/blob/master/doc/PROTOCOL-HTTP2.md
7. Anthropic streaming completions documentation (current SSE; older NDJSON) — https://docs.anthropic.com/en/api/messages-streaming
8. The `tonic` Rust gRPC crate — https://docs.rs/tonic
9. LMAX Disruptor design (lock-free ring buffer for streaming pipelines) — https://lmax-exchange.github.io/disruptor/
10. Linux `splice(2)` and `sendfile(2)` man pages — https://man7.org/linux/man-pages/man2/splice.2.html and https://man7.org/linux/man-pages/man2/sendfile.2.html
