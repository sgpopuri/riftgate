# ADR 0007. Hand-rolled table-driven FSM in riftgate-parser; httparse for headers in v0.1; full FSM in v0.2

> **Date:** 2026-05-03
> **Status:** accepted
> **Options doc:** [007-protocol-parser](../05-options/007-protocol-parser.md)
> **Deciders:** Sriram Popuri

## Context

Riftgate's data plane needs an HTTP/1.1 + SSE parser. Full exploration of candidates (`hyper` end-to-end, `httparse` + custom FSM, combinators, hand-rolled table FSM, generated FSM) and the tradeoff matrix live in [Options 007](../05-options/007-protocol-parser.md).

The forces summarized: per-request `BumpArena` integration ([ADR 0006](0006-bump-arena-plus-system-malloc.md)) is incompatible with `hyper`'s `Bytes`-throughout allocation model; the documentation-first pillar argues for an FSM that lives in `riftgate-parser` rather than behind a third-party façade; future MCP-aware parsing ([Options 026](../05-options/026-mcp-orchestration.md)) is easier when we own the FSM. Header tokenization is the one area where a mature substrate (`httparse`) is a meaningful shortcut for `v0.1` without compromising the principle.

## Decision

**`v0.1` ships:**

- `crates/riftgate-parser` containing a hand-rolled, table-driven FSM that handles the HTTP/1.1 body framing path (chunked-encoding, content-length, end-on-close) and the SSE event framing path.
- The `StreamParser` trait per [`docs/04-design/lld-parsing.md`](../04-design/lld-parsing.md) as the public surface; `Http1Parser` and `SseFramer` are the `v0.1` impls.
- HTTP/1.1 header tokenization uses the `httparse` crate (a small, audited, zero-copy header parser), wrapped inside `Http1Parser`. Header borrows are returned as `&'arena str` references after copying into the per-request arena where lifetime requires.
- Parser scratch buffers are allocated from the per-request `BumpArena` ([ADR 0006](0006-bump-arena-plus-system-malloc.md)). Body chunks are emitted as `&'a [u8]` borrows into the input buffer; no per-event allocation on the hot path.
- Errors are typed (`enum ParseError`); no string error messages.

**`v0.2` adds:**

- A hand-rolled header tokenizer in `riftgate-parser`, replacing the `httparse` dependency. This is the systems-showpiece hardening goal — own the substrate end-to-end.
- A property-based test suite (`proptest`) that feeds valid HTTP/1.1 byte sequences split at every possible boundary and verifies the event sequence is identical.
- A `cargo-fuzz` corpus seeded from real captured traffic; CI runs fuzz for >1 hour per release per [FR-404](../05-options/README.md).

**`v0.x` does not:**

- Adopt `hyper` end-to-end as the parser substrate.
- Adopt parser combinators (`nom`, `combine`) on the hot path.
- Adopt a generated FSM (Ragel, re2c, `logos`).
- Implement HTTP/2 — that is a future milestone, captured in [`docs/04-design/lld-parsing.md`](../04-design/lld-parsing.md).

## Consequences

- **Positive:**
  - The parser is readable end-to-end; the FSM table is a teaching artifact aligned with [Vision §3.2](../00-vision.md).
  - Per-request arena integration is clean: scratch from the arena, body slices borrowed from the input.
  - No backtracking; the FSM always advances or waits, which matches the streaming-parser invariant from the table-driven FSM literature (dragon book ch. 3; `http_parser` / `picohttpparser` design).
  - Future MCP-aware extensions ([Options 026](../05-options/026-mcp-orchestration.md)) plug in as new FSM states or new transition tables, with no third-party shim.
  - Compile-time enumerable state space — missing transitions are visible at table construction time.
- **Negative / accepted tradeoffs:**
  - We give up the `Tower` / `Axum` middleware ecosystem that comes for free with `hyper`. Riftgate's filter chain and routing replace that ecosystem with our own pluggable surface.
  - HTTP/1.1 edge cases (header continuation, OWS, chunked extensions, trailers) require our own test coverage rather than inheriting `hyper`'s years of CVEs-and-fixes.
  - Header tokenization in `v0.1` carries the `httparse` dependency. Replacing it in `v0.2` is real engineering work; we accept the cost as part of the systems-showpiece milestone.
  - HTTP/2 is deferred. Operators who need it cannot use Riftgate as a `v0.x` HTTP/2 ingress.
- **Future work this enables:**
  - HTTP/2 parser as a separate `StreamParser` impl in a future milestone, possibly wrapping `h2` (the lower-level HTTP/2 crate, separate from `hyper`'s server side).
  - MCP-aware mode in `v0.5` per [Options 026](../05-options/026-mcp-orchestration.md).
  - Per-stream parser metrics: bytes-in, events-emitted, parse-errors-by-class.
  - Property-based and fuzz coverage as first-class CI ([FR-404](../05-options/README.md)).
- **Future work this forecloses (until superseded):**
  - We will not adopt `hyper` end-to-end without a new ADR.
  - We will not ship combinator-based parsing on the hot path.
  - We will not introduce a build-time FSM generator.

## Compliance

- `crates/riftgate-parser::StreamParser` is the single trait that all parser impls implement.
- `crates/riftgate-parser::http1::Http1Parser` and `crates/riftgate-parser::sse::SseFramer` are the `v0.1` impls.
- `crates/riftgate-parser/Cargo.toml` lists `httparse` as a dependency in `v0.1`; the dependency is removed in `v0.2`.
- Property-based tests (`proptest`) live in `crates/riftgate-parser/tests/property.rs` and run on every PR.
- Fuzz corpus and `cargo-fuzz` targets live in `crates/riftgate-parser/fuzz/`. CI runs the fuzz target for at least 5 minutes per PR; nightly CI runs for >1 hour ([FR-404](../05-options/README.md)).
- Conformance tests against curated edge cases (chunked + trailers, SSE with comments, header continuation rejection, very-large headers) live in `crates/riftgate-parser/tests/conformance.rs`.
- Body chunks emitted by `Http1Parser` carry `&'a [u8]` lifetimes; review enforces no per-event `Vec<u8>` materialization.

## Notes

- The `httparse` shortcut in `v0.1` is deliberate. The header tokenizer is the most CVE-prone part of HTTP/1.1; reusing a small audited library is the conservative choice. The hand-roll in `v0.2` happens when we have the engineering capacity to take on that surface area properly.
- The decision to defer HTTP/2 to a future milestone is captured in [`docs/04-design/lld-parsing.md`](../04-design/lld-parsing.md). The `StreamParser` trait is shaped to accommodate it; specifically, the `feed` method returns a `Vec<ParseEvent<'_>>` rather than a single event, which is friendly to HTTP/2's frame-at-a-time model.
- The naming `Http1Parser` (vs `Http1FsmParser` or similar) is the public-API choice; the internal FSM table can be renamed without breaking callers.
- The relationship to [Options 008 (stream framing)](../05-options/008-stream-framing.md) is layered: `Http1Parser` handles the request body framing (chunked, content-length); `SseFramer` handles the SSE event framing on the response side. Both are FSMs; both are in `riftgate-parser`; both feed `ParseEvent`s into the rest of the data plane.
