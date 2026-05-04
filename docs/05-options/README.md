# Options Docs Index

Each Options doc explores a load-bearing design decision: candidates, tradeoffs, source-systems chapters consulted, and a recommendation. Decisions land in [`../06-adrs/`](../06-adrs/).

This is the moat. Read a few entries to get the project's flavor before contributing.

## How to read these

- Start with the `Recommendation` section if you only want the answer.
- Read the full doc if you want to learn the decision space (Persona P3 — Maya — is the target reader).
- Each Options doc cites the source-systems chapter(s) that informed it by plain-text title (e.g. `Ch3 (io_uring)`). The source-systems curriculum is not a public sibling repo; citations are by chapter title and number, not by hyperlink.

## How to add a new one

1. Copy [`_template.md`](_template.md) to `NNN-<slug>.md`. Use the next free number; numbering is permanent.
2. Fill in. Be exhaustive. Cite real systems and real chapters.
3. Open a PR with the new file and a stub ADR in `../06-adrs/` that the Options doc will eventually feed.
4. Once the ADR is accepted, mark the Options doc `accepted` in its frontmatter and link to the ADR.

## Index

| # | Title | Status | ADR | Source chapters |
|---|-------|--------|-----|-----------------|
| 001 | [IO model](001-io-model.md) | recommended | [0002](../06-adrs/0002-start-on-epoll.md) | Ch1, Ch3, Ch6 |
| 002 | [async runtime](002-async-runtime.md) | recommended | [0003](../06-adrs/0003-tokio-multithread-default.md) | Ch2, Ch3, Ch7 |
| 003 | [concurrency model](003-concurrency-model.md) | recommended | [0004](../06-adrs/0004-per-shard-default-stealing-opt-in.md) | Ch7, Ch12, Ch4 |
| 004 | [request queue](004-request-queue.md) | recommended | [0005](../06-adrs/0005-sharded-mpmc-queue.md) | Ch4, Ch12 |
| 005 | [allocator](005-allocator.md) | recommended | [0006](../06-adrs/0006-bump-arena-plus-system-malloc.md) | Ch14 |
| 006 | timer subsystem | TBD | TBD | Ch15 |
| 007 | [protocol parser](007-protocol-parser.md) | recommended | [0007](../06-adrs/0007-handrolled-fsm-parser.md) | Ch13 |
| 008 | [stream framing](008-stream-framing.md) | recommended | [0008](../06-adrs/0008-sse-default-grpc-future.md) | Ch5, Ch13 |
| 009 | request log | TBD | TBD | Ch9, Ch11 |
| 010 | routing strategy | TBD | TBD | Ch12 + research |
| 011 | circuit breaker | TBD | TBD | Ch12 |
| 012 | backpressure policy | TBD | TBD | Ch8, Ch12 |
| 013 | observability sink | TBD | TBD | Ch16 |
| 014 | eBPF integration | TBD | TBD | Ch16 |
| 015 | configuration model | TBD | TBD | Ch12 |
| 016 | extension mechanism | TBD | TBD | Ch12 |
| 017 | multitenancy | TBD | TBD | Ch12 |
| 018 | deployment | TBD | TBD | Ch12 |
| 019 | replay-eval | TBD | TBD | Ch10, Ch11 |
| 020 | language (Rust vs Zig) | TBD | [0001](../06-adrs/0001-rust-not-go-or-zig.md) | — |
| 021 | [rate-limiting](021-rate-limiting.md) | recommended | [0009](../06-adrs/README.md) (reserved) | Ch12; `hashing/ch07` (future distributed); `trees/ch04` (priority under pressure) |
| 022 | fairness-scheduling (optional; decide at `v0.2` retro) | TBD | TBD | Ch7; `trees/ch04_heaps_priority_queues.md` |
| 026 | [mcp-orchestration](026-mcp-orchestration.md) | recommended | [0015](../06-adrs/README.md) (reserved) | Ch12; `advanced/ch08_design_data_structures.md`; `graphs/ch03_topological_sort_dags.md` |
| 027 | upstream-protocols-http2-grpc (optional; deepens 008) | TBD | — | Ch13 |
| 028 | token-accounting (optional; folds into filter starter library) | TBD | — | — |
| 029 | async-telemetry-pipeline (optional; deepens 013) | TBD | — | Ch8, Ch10 |

This index is updated whenever a new Options doc lands or an existing one changes status. Stale entries are a documentation bug; please open a PR.

### Research-pass additions (2026-05)

Options `021` and `026` were added after a research pass against the 2026 LLM-gateway landscape; the rationale, competing approaches, and the items we deliberately declined (multi-provider adapters as a kernel feature, semantic cache reference impl, distributed state substrate) are recorded in [`docs/00-vision.md §4`](../00-vision.md) and [`docs/00-vision.md §8`](../00-vision.md). The optional Options (`022`, `027`, `028`, `029`) are gated on retrospective decisions at milestone close — if the kernel is already honest about the gap, we do not manufacture a doc just to have one.
