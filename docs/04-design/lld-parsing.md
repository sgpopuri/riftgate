# 04.c LLD — Parsing

> Incremental, FSM-based HTTP/1.1 + SSE parser. Streams in bytes, emits typed events without backtracking.
>
> Status: **shipped (v0.1)**. The `Http1Parser` (httparse-backed for headers, hand-rolled FSM for body) and `SseFramer` are the v0.1 default; chunked transfer encoding lands in v0.2 per [ADR 0007](../06-adrs/0007-handrolled-fsm-parser.md).

## Purpose

Convert a stream of TCP bytes into a stream of typed events (`HeadersComplete`, `BodyChunk`, `BodyComplete`, `SseToken`, etc.) without buffering the whole request, without backtracking, and without ambiguity about partial input.

## Trait surface

The shipped trait — see [`crates/riftgate-core/src/parser.rs`](../../crates/riftgate-core/src/parser.rs):

```rust
pub enum ParseEvent<'a> {
    HeadersComplete(Headers),
    BodyChunk(&'a [u8]),
    BodyComplete,
    SseToken(&'a [u8]),
    SseDone,
    Error(ParseError),
}

pub enum ParseError {
    HeaderTooLarge(usize),
    InvalidChunkedEncoding { reason: &'static str },
    MalformedRequestLine { reason: &'static str },
    MalformedHeader { offset: usize },
    MalformedSse { reason: &'static str },
    UnsupportedHttpVersion(String),
}

pub trait StreamParser: Send {
    fn feed<'a>(&'a mut self, bytes: &'a [u8]) -> Vec<ParseEvent<'a>>;
    fn reset(&mut self);
}
```

Two design adjustments from the v0.0 sketch:

- The `feed` lifetime is explicit (`fn feed<'a>(&'a mut self, bytes: &'a [u8]) -> Vec<ParseEvent<'a>>`) so events can borrow from either the input bytes or the parser's scratch buffer (whichever holds the data after the current call). This is the key constraint that lets the parser avoid copying `BodyChunk` and `SseToken` payloads.
- `ParseError` is a typed enum with stable variants — downstream code pattern-matches on the variant; the `reason: &'static str` fields are diagnostics only, never part of the public match contract.

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `Http1Parser` | shipped (v0.1) | `riftgate-parser` | Headers via `httparse`, body via a hand-rolled `Content-Length` FSM. Chunked encoding deliberately deferred to v0.2 — the FSM is wired for it but the dispatch table currently only handles `Content-Length` and identity bodies. |
| `SseFramer` | shipped (v0.1) | `riftgate-parser` | Hand-rolled FSM over the SSE wire format. Recognizes `data:`, `event:`, `id:`, `retry:` lines and emits `SseToken` for each `data:` payload, `SseDone` for the OpenAI `data: [DONE]\n\n` sentinel. |
| `Http1Parser` (chunked) | v0.2 | `riftgate-parser` | Adds chunked transfer encoding to the existing `Http1Parser`; the trait surface does not change. |
| `Http2Parser` | future (v1.x) | TBD | Not on the v0.1 / v0.2 roadmap. |

Decision rationale: [Options 007 (protocol parser)](../05-options/007-protocol-parser.md), [Options 008 (stream framing)](../05-options/008-stream-framing.md).

Foundational principle: table-driven FSM-based protocol parsing (Aho/Sethi/Ullman dragon book, ch. 3; the same shape used by `http_parser`, `picohttpparser`, and nginx's HTTP/1.1 codec). The parser is intentionally a table-driven FSM rather than a hand-rolled `if`/`else` chain; the table is testable and the state space is enumerable.

## Component context

### Architecture and dependencies

The parser sits between the [`io-runtime`](lld-io-runtime.md) (which provides bytes) and the rest of the data plane (which consumes typed events). It owns no IO; it owns one bounded `scratch: Vec<u8>` per parser instance for buffering bytes across `feed` calls. Both `Http1Parser` and `SseFramer` follow the same shape: append to scratch, drive the FSM, emit events that may borrow from the input or from scratch.

External dependencies are tightly scoped:

- `httparse` for the header-and-request-line parse step in `Http1Parser`. Chosen because it is `#![no_std]`, has a stable single-shot API (`Request::parse`), and does not allocate. The body FSM is hand-rolled per [ADR 0007](../06-adrs/0007-handrolled-fsm-parser.md).
- No external crate for `SseFramer` — pure FSM over `\n` and `:` boundaries.

### Patterns and conventions

- **Incremental, no buffering of whole bodies.** The parser yields `BodyChunk` events as the FSM consumes bytes; callers stream them onward without ever holding the full body.
- **No backtracking.** A parsed-then-rolled-back state is a sign of an FSM design error; the FSM should always advance or wait for more input. The header step uses `httparse::Status::Partial` to signal "wait for more input" without consuming anything.
- **Borrowed slices wherever possible.** `BodyChunk` and `SseToken` reference into the scratch buffer rather than copying. Callers that need the bytes past the next `feed` call must copy.
- **Errors are typed.** Pattern-match on the variant; never on the `reason: &'static str` fields.
- **`reset` reuses the scratch buffer.** Per-connection parser reuse avoids a `Vec::with_capacity` per request on the hot path.
- **Strict mode only.** No permissive parsing for malformed input. Header continuation lines, bare `\n` line terminators, and similar pre-RFC-7230 quirks return `ParseError::MalformedHeader`.

### Pitfalls

- **Header continuation lines** (RFC 7230 deprecates them; some old clients still send them) — `httparse` rejects them, which is the correct behavior; do not "fix" this.
- **Chunked encoding boundaries** that span `feed` calls — the FSM must remember partial chunk-size lines; this is wired but not exercised in v0.1.
- **SSE `data:` lines** that span `feed` calls — the framer's FSM holds the partial line in scratch until the terminating `\n\n` arrives.
- **Trailing whitespace and CRLF variants** — `\r\n` only; bare `\n` is rejected. Strict by default.
- **Very large headers** — bound by the scratch buffer cap (default 16 KB); over-large headers return `ParseError::HeaderTooLarge` with the cap as the payload.
- **Lifetime on `feed`** — the returned events borrow from `&mut self`, so the next `feed` call invalidates them. Callers that need to retain `BodyChunk` data must clone first.

### Standards and review gates

- Property-based tests via `proptest` — feed valid HTTP/SSE byte sequences split at every possible boundary, verify the event sequence is identical to the unsplit case.
- Fuzz tests via `cargo-fuzz` — feed random byte sequences, verify the parser never panics, never reads past the input, and either emits a `ParseError` or produces a structurally valid event sequence.
- Microbenchmarks: [`crates/riftgate-parser/benches/http1.rs`](../../crates/riftgate-parser/benches/http1.rs) and [`crates/riftgate-parser/benches/sse.rs`](../../crates/riftgate-parser/benches/sse.rs) gate parse throughput.
- The trait surface is part of the v0.1 frozen surface; changes require a new ADR superseding [ADR 0007](../06-adrs/0007-handrolled-fsm-parser.md).

## Testing strategy

- Unit tests in `crates/riftgate-parser/src/http1.rs` and `src/sse.rs` cover the happy path, the boundary-split path, and every `ParseError` variant.
- Property tests with arbitrary byte-split boundaries (proptest harness in the same files).
- Fuzz tests for crash safety (cargo-fuzz harness wired in `crates/riftgate-parser/fuzz/`).
- Microbenchmarks in [`crates/riftgate-parser/benches/http1.rs`](../../crates/riftgate-parser/benches/http1.rs) and [`crates/riftgate-parser/benches/sse.rs`](../../crates/riftgate-parser/benches/sse.rs).
- The end-to-end test in [`crates/riftgate/tests/e2e.rs`](../../crates/riftgate/tests/e2e.rs) exercises the parsers in the full proxy flow.

## Open questions

- Should we generate the FSM table from a declarative spec (similar to `nom` or `pest`)? Recommend hand-curated table for v0.1 and v0.2; revisit if maintenance pain becomes real.
- Should we support pipelined HTTP/1.1 requests? Recommend yes (the FSM supports it natively); document the per-connection pipeline-depth limit when we ship it.
- HTTP/2 lands when? Probably v1.x; not on the v0.1 / v0.2 roadmap.
