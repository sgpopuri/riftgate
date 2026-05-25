# riftgate-parser

Two `StreamParser` impls for the v0.1 walking skeleton:

- `Http1Parser` — parses HTTP/1.1 requests. Headers via `httparse` (well-tested, fast); body via a hand-rolled FSM. v0.1 supports `Content-Length`-framed bodies; chunked-encoding lands in v0.2 per [ADR 0007](../../docs/06-adrs/0007-handrolled-fsm-parser.md).
- `SseFramer` — parses Server-Sent Events streams. Recognizes `data:` lines, blank-line event terminators, and the OpenAI-style `data: [DONE]\n\n` sentinel.

Both impls satisfy the `riftgate-core::parser::StreamParser` trait. They are structured around a small explicit state-machine type and never rely on hidden implicit state.

## Tests

- `tests/http1_corpus.rs` — fixture-driven request-parse tests including header / body byte boundaries inside the same feed and across feed calls.
- `tests/sse_boundary.rs` — `proptest`-driven boundary tests that feed an SSE byte stream in random splits and verify the parsed events match the bytes-at-once result.
- `fuzz/fuzz_targets/http1_request.rs` — `cargo-fuzz` harness for arbitrary byte sequences against `Http1Parser`. Run with `cargo +nightly fuzz run http1_request` once `cargo-fuzz` is installed.
