# Options Docs Index

Each Options doc explores a load-bearing design decision: candidates, tradeoffs, foundational principles, and a recommendation. Decisions land in [`../06-adrs/`](../06-adrs/).

This is the moat. Read a few entries to get the project's flavor before contributing.

## How to read these

- Start with the `Recommendation` section if you only want the answer.
- Read the full doc if you want to learn the decision space (Persona P3 — Maya — is the target reader).
- Each Options doc names the technology, pattern, or paper that informed it directly in the prose. The full external bibliography (RFCs, kernel docs, papers, source repos, books) lives in the doc's `## References` section.

## How to add a new one

1. Copy [`_template.md`](_template.md) to `NNN-<slug>.md`. Use the next free number; numbering is permanent.
2. Fill in. Be exhaustive. Cite real systems, real papers, real RFCs.
3. Open a PR with the new file and a stub ADR in `../06-adrs/` that the Options doc will eventually feed.
4. Once the ADR is accepted, mark the Options doc `accepted` in its frontmatter and link to the ADR.

## Index

| # | Title | Status | ADR | Foundational topics |
|---|-------|--------|-----|---------------------|
| 001 | [IO model](001-io-model.md) | recommended | [0002](../06-adrs/0002-start-on-epoll.md) | Unix I/O multiplexing (`epoll`/`kqueue`), `io_uring`, DPDK / kernel-bypass |
| 002 | [async runtime](002-async-runtime.md) | recommended | [0003](../06-adrs/0003-tokio-multithread-default.md) | reactor pattern, `io_uring`, work-stealing schedulers |
| 003 | [concurrency model](003-concurrency-model.md) | recommended | [0004](../06-adrs/0004-per-shard-default-stealing-opt-in.md) | work-stealing, shared-nothing per-shard isolation, lock-free structures |
| 004 | [request queue](004-request-queue.md) | recommended | [0005](../06-adrs/0005-sharded-mpmc-queue.md) | lock-free structures, MPMC queues, sharded-queue patterns |
| 005 | [allocator](005-allocator.md) | recommended | [0006](../06-adrs/0006-bump-arena-plus-system-malloc.md) | memory allocators (jemalloc / mimalloc / arenas) |
| 006 | timer subsystem | TBD | TBD | hierarchical / hashed timer wheels |
| 007 | [protocol parser](007-protocol-parser.md) | recommended | [0007](../06-adrs/0007-handrolled-fsm-parser.md) | FSM-based protocol parsing |
| 008 | [stream framing](008-stream-framing.md) | recommended | [0008](../06-adrs/0008-sse-default-grpc-future.md) | ring buffers and zero-copy I/O, FSM-based parsing |
| 009 | request log | TBD | TBD | LSM trees, write-ahead logging |
| 010 | routing strategy | TBD | TBD | sidecar / ambassador patterns, KV-aware prefix routing |
| 011 | circuit breaker | TBD | TBD | resilience patterns (Nygard *Release It*) |
| 012 | backpressure policy | TBD | TBD | backpressure as policy, drop-on-full ring buffers |
| 013 | observability sink | TBD | TBD | OTel exporters, eBPF |
| 014 | eBPF integration | TBD | TBD | eBPF (Aya, libbpf, bpftrace) |
| 015 | configuration model | TBD | TBD | configuration patterns (static TOML, hot-reload, CRD) |
| 016 | extension mechanism | TBD | TBD | sandboxed extension surfaces (WASM via wasmtime) |
| 017 | multitenancy | TBD | TBD | tenant-isolation patterns |
| 018 | deployment | TBD | TBD | sidecar / ambassador deployment patterns |
| 019 | replay-eval | TBD | TBD | streaming sketches, write-ahead logging |
| 020 | language (Rust vs Zig) | TBD | [0001](../06-adrs/0001-rust-not-go-or-zig.md) | — |
| 021 | [rate-limiting](021-rate-limiting.md) | recommended | [0009](../06-adrs/0009-rate-limiter-trait-in-proc-only.md) (proposed) | rate-limiting algorithms (token bucket, GCRA), consistent hashing (future distributed), priority heaps (priority under pressure), lock-free structures |
| 022 | fairness-scheduling (optional; decide at `v0.2` retro) | TBD | TBD | work-stealing, priority heaps |
| 026 | [mcp-orchestration](026-mcp-orchestration.md) | recommended | [0015](../06-adrs/0015-mcp-extension-plane-broker.md) (proposed) | ambassador pattern, capability-based security, allowlist data structures (tries, bit-sets), topological sort over DAGs, write-ahead logging for audit |
| 027 | upstream-protocols-http2-grpc (optional; deepens 008) | TBD | — | FSM-based parsing |
| 028 | token-accounting (optional; folds into filter starter library) | TBD | — | — |
| 029 | async-telemetry-pipeline (optional; deepens 013) | TBD | — | pub/sub messaging, streaming sketches |

This index is updated whenever a new Options doc lands or an existing one changes status. Stale entries are a documentation bug; please open a PR.

### Research-pass additions (2026-05)

Options `021` and `026` were added after a research pass against the 2026 LLM-gateway landscape; the rationale, competing approaches, and the items we deliberately declined (multi-provider adapters as a kernel feature, semantic cache reference impl, distributed state substrate) are recorded in [`docs/00-vision.md §4`](../00-vision.md) and [`docs/00-vision.md §8`](../00-vision.md). The optional Options (`022`, `027`, `028`, `029`) are gated on retrospective decisions at milestone close — if the kernel is already honest about the gap, we do not manufacture a doc just to have one.
