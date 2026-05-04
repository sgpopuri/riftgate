# 04.c LLD — Parsing

> Incremental, FSM-based HTTP/1.1 + SSE parser. Streams in bytes, emits typed events without backtracking.
>
> Status: **outline-stage**. Filled out as `v0.1` lands.

## Purpose

Convert a stream of TCP bytes into a stream of typed events (`HeadersComplete`, `BodyChunk`, `BodyComplete`, `SseToken`, etc.) without buffering the whole request, without backtracking, and without ambiguity about partial input.

## Trait surface

```rust
// Sketch
pub enum ParseEvent<'a> {
    HeadersComplete(Headers),
    BodyChunk(&'a [u8]),
    BodyComplete,
    SseToken(&'a [u8]),
    SseDone,
    Error(ParseError),
}

pub trait StreamParser: Send {
    fn feed(&mut self, bytes: &[u8]) -> Vec<ParseEvent<'_>>;
    fn reset(&mut self);
}
```

## Implementations

| Impl | Status | Source crate |
|------|--------|--------------|
| `Http1Parser` | `v0.1` | `riftgate-parser` |
| `SseFramer` | `v0.1` | `riftgate-parser` |
| `Http2Parser` | future | TBD |

Decision rationale: [Options 007 (protocol parser)](../05-options/007-protocol-parser.md), [Options 008 (stream framing)](../05-options/008-stream-framing.md).

Source-systems chapter: `Ch13 (FSM and protocol parsing)`. The parser is intentionally a table-driven FSM rather than a hand-rolled `if`/`else` chain; the table is testable and the state space is enumerable.

## Component context

### Architecture and dependencies

The parser sits between the [`io-runtime`](lld-io-runtime.md) (which provides bytes) and the rest of the data plane (which consumes typed events). It owns no IO and no allocation outside the bounded scratch buffer it carries.

### Patterns and conventions

- **Incremental, no buffering of whole bodies.** The parser yields events as soon as they are unambiguous.
- **No backtracking.** A parsed-then-rolled-back state is a sign of an FSM design error; the FSM should always advance or wait for more input.
- **Borrowed slices wherever possible.** `BodyChunk` and `SseToken` reference into the input buffer rather than copying.
- **Errors are typed.** No string error messages; an `enum ParseError` drives downstream behavior.

### Pitfalls

- **Header continuation lines** (deprecated in HTTP/1.1 but still seen) — the parser must reject them cleanly.
- **Chunked encoding boundaries** that span buffer boundaries — the FSM must remember partial chunk-size lines.
- **SSE `data:` lines** that span buffer boundaries — easy to get wrong.
- **Trailing whitespace and CRLF variants** — strict mode by default; permissive opt-in.
- **Very large headers** — bound by the scratch buffer size; over-large headers return `ParseError::HeaderTooLarge`.

### Standards and review gates

- Property-based tests on the parser using `proptest` — feed valid HTTP/SSE byte sequences split at every possible boundary, verify event sequence is identical.
- Fuzz tests via `cargo-fuzz` — feed random byte sequences, verify the parser never panics.
- Conformance against a curated set of edge cases (chunked + trailers, SSE with comments, etc.).

## Testing strategy

- Property tests with arbitrary byte-split boundaries.
- Fuzz tests for crash safety.
- Conformance tests against [hyperfine](https://github.com/sharkdp/hyperfine) and [picohttpparser](https://github.com/h2o/picohttpparser) reference vectors.
- Regression suite from real captured traffic (anonymized).

## Open questions

- Should we generate the FSM table from a declarative spec (similar to `nom` or `pest`)? Recommend hand-curated table for `v0.1`; revisit if it becomes hard to maintain.
- Should we support pipelined HTTP/1.1 requests? Recommend yes (the FSM supports it natively); document the limit on pipeline depth.
- HTTP/2 lands when? Probably `v1.x`; not on the current roadmap.
