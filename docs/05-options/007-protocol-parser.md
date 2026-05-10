# 007. Protocol Parser

> **Status:** `recommended` — table-driven hand-rolled FSM in `riftgate-parser`, leveraging `httparse` as the v0.1 zero-copy header tokenizer; full hand-rolled FSM (header tokenizer included) is a v0.2 hardening goal. See [ADR 0007](../06-adrs/0007-handrolled-fsm-parser.md).
> **Foundational topics:** finite-state-machine protocol parsing, table-driven lexers and grammars, streaming (non-backtracking) parser discipline
> **Related options:** [001](001-io-model.md) (IO model), [002](002-async-runtime.md) (async runtime), [005](005-allocator.md) (allocator), [008](008-stream-framing.md) (stream framing)
> **Related ADR:** [ADR 0007](../06-adrs/0007-handrolled-fsm-parser.md)

## 1. The decision in one sentence

> Does Riftgate's HTTP/1.1 + SSE parser sit on top of `hyper` (or another existing crate), use parser combinators, hand-roll a table-driven finite-state machine, or generate the FSM from a declarative spec?

## 2. Context — what forces this decision

Every byte that enters Riftgate goes through the parser. The parser sits between the [`io-runtime`](../04-design/lld-io-runtime.md) (which provides bytes) and the rest of the data plane (which consumes typed events). Its design choice cascades through:

- The hot-path allocation model. A parser that does its own buffering bypasses the per-request `BumpArena` from [ADR 0006](../06-adrs/0006-bump-arena-plus-system-malloc.md); a parser that emits borrowed slices into the arena participates correctly.
- The shape of the streaming pipeline. SSE response framing requires a parser that yields tokens incrementally without buffering the whole body; HTTP/2 (in a future milestone) requires a frame-aware parser.
- The fuzz-and-test surface. The parser is the most-attacked surface in the system; it must be fuzz-clean by [NFR-R05](../01-requirements/non-functional.md) and [FR-404](../01-requirements/functional.md).
- The contributor onramp. A parser that is readable end-to-end is part of Riftgate's documentation-first pillar; a parser hidden behind a third-party crate's macros is not.

Forces driving this decision:

- **Per-request arena integration** ([ADR 0006](../06-adrs/0006-bump-arena-plus-system-malloc.md)). The parser must allocate scratch buffers from the request's `BumpArena`, not from the global allocator. Existing crates like `hyper` allocate through their own buffer types; integrating cleanly is non-trivial.
- **Borrow-of-input tokens.** [`docs/04-design/lld-parsing.md`](../04-design/lld-parsing.md) commits the parser to emitting `&'a [u8]` slices that reference into the input buffer. No copying, no per-event allocations.
- **Bounded state space.** HTTP/1.1's grammar is finite and well-known; the FSM has a small number of states, all enumerable. This is the textbook setting where an FSM beats both ad-hoc parsing and combinators.
- **Operability.** The parser must produce typed errors, not free-form strings. [`docs/04-design/lld-parsing.md`](../04-design/lld-parsing.md) Pitfalls calls out specific edge cases (header continuation, chunked-encoding boundary, SSE `data:` line spans) that must be reachable in tests.
- **The documentation-first pillar.** Per [`docs/00-vision.md §3.2`](../00-vision.md), every load-bearing subsystem should be readable end-to-end as a teaching artifact. A `hyper`-backed parser hides the FSM behind a stable public API; the FSM itself is not the readable surface.
- **Future MCP-aware framing.** [Options 026 (MCP orchestration)](026-mcp-orchestration.md) recommends parsing MCP requests in the gateway. Owning the parser substrate makes adding an MCP-aware mode straightforward; bolting it onto `hyper` is harder.

## 3. Candidates

### 3.1. Use `hyper` end-to-end

**What it is.** `hyper` is the canonical Rust HTTP/1.1 + HTTP/2 crate. It owns the parser, the body framing, the connection lifecycle, the request/response abstractions, and the integration with Tokio. A `hyper`-based Riftgate would expose `tower::Service` boundaries and let `hyper` do all parsing.

**Why it's interesting.**
- **Battle-tested.** `hyper` is the parser substrate for `reqwest`, `axum`, `warp`, and most of the production Rust HTTP ecosystem. Years of fuzz coverage, years of CVEs caught and fixed.
- **HTTP/2 already in the box.** The `hyper-h2` codebase is one of the few production-quality Rust HTTP/2 implementations.
- **Idiomatic Tokio integration.** `hyper::Server::builder(...).http1_keepalive(true).serve(...)` is a one-liner; everything composes.
- **Zero engineering cost on day one.** A working HTTP server in `v0.1` could be a few hundred lines of glue.

**Where it falls short.**
- **Allocation discipline lives in `hyper`, not in our arena.** `hyper` allocates buffers, headers, body chunks through its own `Bytes`/`BytesMut` machinery. Wrapping that to allocate from a per-request `BumpArena` is awkward at best; the buffer types are exposed in public APIs.
- **Bytes-vs-borrowed-slices is not the parser's choice in `hyper`.** `hyper` returns `Bytes` (reference-counted), not `&'a [u8]` slices. We can convert, but we pay the cost.
- **The FSM is hidden.** New contributors who want to learn HTTP/1.1 parsing in Riftgate's codebase find `hyper` types and a stable façade. The teaching-artifact goal is harder to deliver.
- **Future MCP-aware parsing requires owning the buffer.** An MCP-aware mode that wants to short-circuit parsing on a `tools/call` envelope cannot easily insert hooks into `hyper`'s internal state machine.
- **Locks Riftgate into the `tower::Service` shape.** Exiting that shape later (if we ever want a non-`tower` `Service` trait, or per-shard execution outside the standard Tokio pattern) is meaningful work.
- **Couples upgrades to `hyper`'s release cadence.** Major `hyper` releases (e.g. 0.14 → 1.0 in 2023-2024) have meaningful API churn; we would carry that risk.

**Real-world systems that use it.** `axum`, `warp`, `actix-web` (partially), `reqwest` client side, most Rust HTTP servers above the toy tier.

### 3.2. `httparse` for headers + hand-rolled FSM for body framing

**What it is.** `httparse` is a tiny crate (~500 lines) that does only one thing: zero-copy parsing of HTTP/1.1 request and response *headers*. It does not handle body framing, chunked encoding, or SSE. The application layer composes `httparse` for the head and writes its own state machine for the body. This is what `hyper` itself does internally; `hyper` is `httparse` plus a lot of glue.

**Why it's interesting.**
- **Header parsing is the most fiddly part of HTTP/1.1.** Header continuation lines (deprecated but still seen), trailing whitespace, CRLF variants, very large headers, very many headers — all of this is in `httparse` and tested.
- **Zero-copy by construction.** `httparse::Request::parse(&buf)` returns `&str` slices into the input buffer. Friendly to a per-request arena.
- **Tiny dependency surface.** ~500 lines of carefully audited code, no `unsafe` outside the SIMD fast path, BSD-licensed, written by the same people who built `hyper`.
- **Lets us hand-roll the interesting parts.** Body framing (chunked, content-length, end-on-close), SSE event framing, future MCP envelope detection — these are the parts where Riftgate adds value, and they live entirely in `riftgate-parser`.

**Where it falls short.**
- **Header parsing is not in our codebase.** A reader of Riftgate's parser sees the body FSM but not the header tokenization; the teaching-artifact story is partly outsourced.
- **`httparse` semantics lock us in.** Some HTTP/1.1 edge cases (e.g. how it handles missing CR before LF) follow `httparse`'s opinion, not ours.
- **Two codebases to track.** Upstream `httparse` is stable but not zero-maintenance; we follow its release cadence.
- **Some duplication of FSM logic.** The header tokenizer's state machine and the body framing's state machine are both real FSMs; combining them later (if we ever want a single unified table) requires re-implementing the header tokenizer.

**Real-world systems that use it.** `hyper` (internally), several smaller Rust HTTP servers, parts of `reqwest`, embedded Rust HTTP impls.

### 3.3. Parser combinators (`nom` or `combine`)

**What it is.** A combinator library lets you express the grammar in code: `tag(b"GET ")` matches a literal, `take_until(" ")` extracts a method, and so on. The combinator engine handles the underlying state. `nom` is the canonical Rust combinator library; `combine` is a smaller alternative.

**Why it's interesting.**
- **Grammar reads like the spec.** A reviewer can map `nom` parser code to the HTTP/1.1 RFC line-by-line.
- **Composition is cheap.** Combining parsers (e.g. an HTTP/1.1 parser composed with a chunked-encoding parser composed with a content-length parser) is what combinators are designed for.
- **Backtracking is cheap to express.** Optional fields, alternative grammars — combinators handle these naturally.
- **Type-safe error propagation** through `IResult<I, O, E>`.

**Where it falls short.**
- **Not zero-copy by default.** `nom` returns `&[u8]` slices fine for the matched portion, but composing parsers often involves intermediate types that allocate. Strict zero-copy demands work to enforce.
- **Backtracking is the wrong cost model for streaming.** A combinator that backtracks after consuming half the input has read bytes it cannot un-read on the wire. Streaming parsers should never backtrack; they should always advance or wait. `nom` supports streaming mode but the discipline is on the developer.
- **Compile-time and code-size cost.** `nom` relies heavily on Rust's monomorphization; the resulting binary can be substantially larger than a hand-rolled FSM, and compile times grow.
- **Performance is good but not great.** `nom` is fast for offline parsing; on-the-hot-path streaming, hand-rolled FSMs are typically 2-3× faster because they avoid combinator-internal trampolines.
- **Less natural for streaming-ifying** (incrementally feeding bytes and asking "any events yet?") than a state-machine-with-an-explicit-state model.

**Real-world systems that use it.** `nom` is used by many tools (parsers for various binary formats, configuration languages, etc.); few high-throughput network servers use combinators on the hot path.

### 3.4. Hand-rolled table-driven FSM (the LLD's stated direction)

**What it is.** A finite state machine encoded as a state-transition table: rows indexed by current state, columns by input character class, cells naming the next state and an action. The parser feeds bytes through the table; transitions emit events. The `state` is a small enum, the `transition` table is a `[[...]]` constant, the action is a function pointer or a small `match`.

**Why it's interesting.**
- **The state space is enumerable.** Every (state, input class) pair has exactly one entry; missing transitions are compile-time visible.
- **Performance.** A table-driven FSM is essentially `loop { state = TABLE[state][class[*p]]; p++; }`. Sub-100 ns per byte on modern x86, mostly bound by memory bandwidth.
- **Zero allocation on the hot path.** The FSM owns a small scratch buffer (allocated from the per-request `BumpArena`); events are emitted as borrowed slices into the input.
- **No backtracking.** The FSM always advances or waits; "rolled-back state" is a design error caught at table-construction time.
- **Fuzz-friendly.** Property-based tests can feed every (state, byte) pair and verify reachability; the FSM never panics on adversarial input.
- **Teaching artifact.** A reader who understands "table-driven FSM" can read the table and know exactly what the parser does.
- **Composes naturally with the SSE framer** ([Options 008](008-stream-framing.md)). The HTTP body FSM hands off to the SSE FSM at the right state transition.
- **Owns the substrate.** Future MCP-aware mode, future per-stream metrics, future arena-aware allocations — all are local to the FSM.

**Where it falls short.**
- **Real engineering work.** The HTTP/1.1 grammar has subtle corners (header continuation, OWS handling, chunked-encoding boundaries spanning buffers, trailing-headers pseudo-state). A from-scratch FSM must enumerate them and a fuzz suite must cover them.
- **Header tokenization is the most painful part.** `httparse` solves this problem in 500 lines that took years to harden. Re-doing this work for `v0.1` is a meaningful slowdown, and the result will not be better than `httparse` for a long time.
- **Mid-pipeline maintenance burden.** A new HTTP/1.1 corner case requires updating the table; the table is small but the discipline is on us.
- **No ecosystem integration.** Existing `tower`, `axum`, `warp` middlewares assume a `hyper`-based shape; a hand-rolled parser does not slot into those.

**Real-world systems that use it.** nginx (table-driven HTTP/1.1 parser written in C), HAProxy, Envoy (parts of the HTTP/1.1 codec, partly hand-rolled), Cloudflare's Pingora (custom Rust parser), Apache Traffic Server.

### 3.5. Generated FSM (`ragel`-style)

**What it is.** A grammar written in a declarative DSL (Ragel, re2c, or a Rust equivalent like `logos`'s state-machine generator) is compiled to optimized FSM code. The grammar is the source of truth; the generated FSM is build-output.

**Why it's interesting.**
- **Grammar is short and editable.** A Ragel `.rl` file for HTTP/1.1 is a few hundred lines; the generated FSM is tens of thousands of optimized lines.
- **Performance is excellent.** Generators emit code that is typically faster than what humans write by hand.
- **Used in serious projects.** Mongrel (the Ruby web server, when it was a thing) and parts of HAProxy use Ragel-generated parsers.
- **Bug surface is in the grammar, not in the FSM.** A grammar error is one place to fix; an FSM error is many.

**Where it falls short.**
- **Generator becomes a build dependency.** Ragel is a C++ tool; integrating it into `cargo build` is real work. Cross-platform builds become more brittle.
- **Generated code is unreadable.** A reader cannot easily map a generated FSM back to the grammar; the teaching-artifact goal is undermined by the indirection.
- **Iterative development is slower.** Every grammar change re-runs the generator; debugging is between the grammar and the generated state machine, neither of which is what you stepped through with the debugger.
- **Rust ecosystem support is weak.** Ragel does not target Rust natively; community wrappers exist but lag the upstream. `logos` is more Rust-native but is targeted at lexers, not full HTTP/1.1 parsers.
- **Integration with the per-request arena is hard.** Generated FSMs typically allocate via standard library types; redirecting to the arena is generator-specific work.

**Real-world systems that use it.** Mongrel (Ruby), HAProxy (parts), Apache HTTP Server (configuration parser), some compilers' tokenizers, RSpamd. Not common in modern Rust HTTP servers.

## 4. Tradeoff matrix

| Property | `hyper` | `httparse` + custom FSM | Combinators (`nom`) | Hand-rolled table FSM | Generated FSM (Ragel) | Why it matters |
|----------|---------|-------------------------|---------------------|-----------------------|------------------------|----------------|
| Per-request arena integration | poor (`Bytes` types throughout) | good (we own the buffers; `httparse` is borrow-only) | medium | very good | poor (generator-specific) | [ADR 0006](../06-adrs/0006-bump-arena-plus-system-malloc.md). |
| Zero-copy borrowed slices | medium (`Bytes` clones cheap but not zero-copy) | good | medium | natural | depends | Hot-path allocation. |
| Hot-path performance | good | very good | good | very good | very good | [NFR-P03](../01-requirements/non-functional.md). |
| Streaming-friendly (incremental, no backtracking) | yes | natural | possible (developer discipline) | natural | yes | SSE response framing requires this. |
| HTTP/1.1 correctness coverage day-one | very high (years of CVEs) | very high (header tokenizer is `hyper`'s) | depends on us | depends on us (we write all tests) | depends on grammar | We do not want to discover header parsing edge cases in production. |
| HTTP/2 readiness | already there | not yet (would need a separate impl) | not yet | not yet | possibly (regenerate from grammar) | Future milestone, not blocking `v0.1`. |
| Engineering cost in `v0.1` | very low | medium | medium | high (header tokenizer is the hard part) | medium-high (build integration) | One maintainer in `v0.x`. |
| Engineering cost in `v0.2`+ (extending) | medium (work around `hyper`'s API) | low (we own the FSM) | low | low | low (regenerate) | Future MCP-aware mode. |
| Teaching-artifact value | low (FSM is hidden) | medium (FSM partly ours) | medium | very high (FSM is ours) | low (grammar is ours but generated code is not) | [Vision §3.2](../00-vision.md). |
| Compatibility with future `riftgate-mcp` parser | poor | good (we extend the FSM) | medium | very good | good | [Options 026](026-mcp-orchestration.md). |
| Compatibility with `Tower`/`Axum` middleware ecosystem | natural | adapters required | adapters required | adapters required | adapters required | We've already chosen pluggability over ecosystem integration. |
| Fuzz surface | covered (years) | partly covered (`httparse` upstream) | depends on us | depends on us | depends on grammar | [FR-404](../01-requirements/functional.md). |

## 5. Foundational principles

The protocol-parsing literature — the dragon book (Aho/Sethi/Ullman), Ragel's manual, and the design notes for `http_parser` (Node.js) and `picohttpparser` — is direct: for protocols whose grammar is finite and well-known, a hand-written or generated FSM is the right substrate; combinators trade away too much of the cost model for too little gain.

Three takeaways:

1. **The FSM is the smallest abstraction that captures protocol semantics.** Combinators add an interpreter layer (one indirection per combinator); generators add a code-generation layer (one indirection per regeneration). A direct FSM is the substrate; everything else is an abstraction over it.
2. **Streaming parsers must never backtrack.** A parser that can roll back has bytes on the wire it cannot un-read. Combinators that support backtracking are dangerous; FSMs that always advance are safe. SSE response framing (where every byte that arrived is already in the client's buffer) makes this absolute.
3. **Table-driven beats hand-rolled if-else for HTTP/1.1.** Benchmarks of HTTP/1.1 method-and-path parsing consistently show table-driven implementations at roughly 2× the throughput of equivalent if-else trees: the if-else version branches unpredictably, the table-driven version is one indirect load per byte. This is the basic case made by Aho/Sethi/Ullman for lexer construction and reproduced by every serious protocol-parsing project since.

A pragmatic caveat from the same literature: header parsing in HTTP/1.1 is fiddly, and mature substrates (`http_parser` in C, `httparse` in Rust) capture years of edge-case fixes. The pragmatic stance is "build the parts that matter for your project; reuse the parts that are commodity." For Riftgate, the body framing and the SSE event framing are where we add value; the header tokenizer is commodity.

## 6. Recommendation

**`v0.1` ships a hand-rolled, table-driven FSM in `riftgate-parser` for the HTTP/1.1 body framing path (chunked-encoding, content-length, end-on-close) and the SSE event framing path. Header tokenization in `v0.1` uses the `httparse` crate as a battle-tested zero-copy substrate. The full FSM (header tokenizer included) is a `v0.2` hardening goal, gated on the engineering capacity to take on the surface area `httparse` currently covers.**

The reasoning, restated:

- The body framing and the SSE framing are where Riftgate's design needs control: per-request arena integration, MCP-aware extensions, per-stream metrics. Hand-rolling these is the right answer.
- Header tokenization is a commodity. `httparse` is small, audited, zero-copy, and free of `unsafe` outside the SIMD fast path. Using it in `v0.1` is the pragmatic shortcut; replacing it in `v0.2` is the principled hardening.
- The combination keeps the documentation-first pillar honest: the readable, FSM-shaped substrate is in `riftgate-parser`; the part we outsource is the part that has been "settled" for a decade.
- We explicitly do not adopt `hyper` end-to-end. `hyper` is excellent at what it does, but its allocation discipline and its `Bytes`-everywhere shape are wrong for Riftgate's per-request arena model.

### Conditions under which we'd revisit

- The `v0.2` hand-roll of the header tokenizer turns out to be more work than expected (or the maintained quality is not better than `httparse`'s). We would keep `httparse` indefinitely and document the choice.
- An HTTP/2 deliverable lands and a re-evaluation of `hyper`-as-substrate becomes warranted (HTTP/2's frame layer is uniform enough that hand-rolling is more cost than win).
- A new mature Rust HTTP/1.1 parser (with arena-friendly allocation) emerges and changes the calculus.

### What stays available behind feature flags

- `riftgate-parser` exposes the `StreamParser` trait. A future `HyperStreamParser` impl could exist behind `--features hyper-parser` if there is operator demand for the `Tower`/`Axum` middleware ecosystem; not on the roadmap.
- A `RagelStreamParser` is not on the roadmap; no opt-in planned.

## 7. What we explicitly reject

- **`hyper` end-to-end as the parser substrate.** Allocation model is wrong for the per-request arena; teaching-artifact value is low; future extensibility (MCP-aware mode) is hard. Reconsider only if the Riftgate maintainer set grows enough that owning the parser becomes the wrong allocation of attention.
- **Combinators (`nom`, `combine`) on the hot path.** Backtracking is the wrong cost model; performance is good but not best-in-class; teaching-artifact value is medium. Reconsider for offline-only parsing tools (e.g. WAL replay) where streaming constraints do not apply.
- **Generated FSM (Ragel, re2c).** Build-tool dependency, generated code is not the teaching surface, integration with the per-request arena is hard. Reconsider if the parser's grammar surface ever grows enough that maintaining a hand-rolled table becomes burdensome.
- **A `hyper`-on-top-of-our-FSM hybrid.** Half-and-half adds complexity without clear win; pick a side, document it.

## 8. References

1. `httparse` crate — https://docs.rs/httparse
2. `hyper` project — https://hyper.rs/
3. `nom` crate — https://docs.rs/nom
4. `http_parser` (Node.js heritage) — https://github.com/nodejs/http-parser
5. `picohttpparser` (h2o.examp1e.net) — https://github.com/h2o/picohttpparser
6. nginx HTTP/1.1 parser source (table-driven C) — https://github.com/nginx/nginx/tree/master/src/http
7. Cloudflare, *Pingora's custom HTTP parser story* (selected blog posts) — https://blog.cloudflare.com/
8. Ragel state machine generator — https://www.colm.net/open-source/ragel/
9. The `logos` lexer-generator crate — https://docs.rs/logos
10. Alfred V. Aho, Ravi Sethi, Jeffrey D. Ullman, *Compilers: Principles, Techniques, and Tools* (the "dragon book", 2nd ed.) — chapters 3 (lexical analysis) and 4 (syntax analysis).
