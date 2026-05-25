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
| 006 | [timer subsystem](006-timer-subsystem.md) | recommended | [0010](../06-adrs/0010-binary-heap-timers-v01-hierarchical-wheel-v02.md) | binary heaps, hashed / hierarchical timer wheels (Varghese & Lauck), `timerfd` / `kevent` `EVFILT_TIMER` |
| 007 | [protocol parser](007-protocol-parser.md) | recommended | [0007](../06-adrs/0007-handrolled-fsm-parser.md) | FSM-based protocol parsing |
| 008 | [stream framing](008-stream-framing.md) | recommended | [0008](../06-adrs/0008-sse-default-grpc-future.md) | ring buffers and zero-copy I/O, FSM-based parsing |
| 009 | [request log](009-request-log.md) | recommended | [0013](../06-adrs/0013-append-only-file-wal.md) | LSM trees, write-ahead logging (ARIES), group-commit fsync, append-only file design (Kafka log segments) |
| 010 | [routing strategy](010-routing-strategy.md) | recommended | [0014](../06-adrs/0014-weighted-random-router.md) | weighted-random sampling (Walker alias method, Vose 1991), KV-aware prefix routing (vLLM), hedged requests (Dean & Barroso) |
| 011 | [circuit breaker](011-circuit-breaker.md) | recommended | [0016](../06-adrs/0016-three-state-circuit-breaker.md) | resilience patterns (Nygard *Release It*), sliding-window failure-rate (Hystrix), FSM-based protection primitives |
| 012 | [backpressure policy](012-backpressure.md) | recommended | [0017](../06-adrs/0017-drop-newest-503-backpressure.md) | backpressure as policy, drop-on-full ring buffers (LMAX Disruptor), Little's law, AIMD admission control |
| 013 | [observability sink](013-observability-sink.md) | recommended | [0011](../06-adrs/0011-otel-default-sink-multisink-fanout.md) | OpenTelemetry / OTLP, Prometheus exposition format, bounded ring-buffer drop-on-full (LMAX Disruptor lineage), eBPF (later) |
| 014 | [eBPF integration](014-ebpf-integration.md) | recommended | [0024](../06-adrs/0024-ebpf-via-aya.md) | eBPF (Aya, libbpf, bpftrace), CO-RE, kprobes / tracepoints, continuous profiling |
| 015 | [configuration model](015-config-model.md) | recommended | [0012](../06-adrs/0012-static-toml-env-override-v01.md) | layered configuration (defaults → file → env), twelve-factor configuration, `serde` + `toml`, file-watch hot reload (`inotify` / `kevent` `EVFILT_VNODE`), Kubernetes CRDs |
| 016 | [extension mechanism](016-extension-mechanism.md) | recommended | [0019](../06-adrs/0019-wasm-extension-mechanism.md) | sandboxed extension surfaces (WASM via wasmtime), Component Model, capability-based security |
| 017 | multitenancy | TBD | TBD | tenant-isolation patterns |
| 018 | deployment | TBD | TBD | sidecar / ambassador deployment patterns |
| 019 | [replay-eval](019-replay-eval.md) | recommended | [0021](../06-adrs/0021-external-replay-cli.md) | write-ahead logging as eval corpus, declarative graders, telemetry tagging (`riftgate.run.kind`) |
| 020 | language (Rust vs Zig) | TBD | [0001](../06-adrs/0001-rust-not-go-or-zig.md) | — |
| 021 | [rate-limiting](021-rate-limiting.md) | accepted | [0009](../06-adrs/0009-rate-limiter-trait-in-proc-only.md) (accepted), [0018](../06-adrs/0018-token-bucket-parameters.md) | rate-limiting algorithms (token bucket, GCRA), consistent hashing (future distributed), priority heaps (priority under pressure), lock-free structures |
| 022 | fairness-scheduling (optional; decide at `v0.2` retro) | TBD | TBD | work-stealing, priority heaps |
| 023 | [token-bucket-parameters](023-token-bucket-parameters.md) | recommended | [0018](../06-adrs/0018-token-bucket-parameters.md) | packed atomic state (Vyukov-style CAS), sharded hash maps, fixed-point arithmetic |
| 024 | [stream cancellation](024-stream-cancellation.md) | recommended | [0020](../06-adrs/0020-stream-cancellation-cancellation-token.md) | cooperative cancellation, `tokio_util::sync::CancellationToken`, typed `CancelCause`, parent/child cascade |
| 025 | [v0.3 routing strategies](025-v03-routing-strategies.md) | recommended | [0022](../06-adrs/0022-kv-aware-routing-prefix-trie.md), [0023](../06-adrs/0023-hedged-requests-p99-triggered.md) | prefix trie (Knuth TAOCP §6.3), xxHash3-64, Dean–Barroso threshold-triggered hedge, P² quantile estimator |
| 026 | [mcp-orchestration](026-mcp-orchestration.md) | recommended | [0015](../06-adrs/0015-mcp-extension-plane-broker.md) (proposed) | ambassador pattern, capability-based security, allowlist data structures (tries, bit-sets), topological sort over DAGs, write-ahead logging for audit |
| 027 | [token-level metrics](027-token-level-metrics.md) | recommended | [0025](../06-adrs/0025-token-level-metrics-probabilistic.md) | streaming sketches (Cormode–Muthukrishnan CMS, Flajolet HLL, Vitter reservoir), HDR histograms, head-vs-tail sampling, WAL-vs-OTel split |
| 028 | [gpu-pressure correlation](028-gpu-pressure-correlation.md) | recommended | [0026](../06-adrs/0026-gpu-pressure-via-dcgm-exporter.md) | DCGM exporter Prometheus scrape, NVML in-process FFI, ambassador / sidecar deployment patterns, multi-vendor GPU telemetry |
| 029 | async-telemetry-pipeline (optional; deepens 013) | TBD | — | pub/sub messaging, streaming sketches |

This index is updated whenever a new Options doc lands or an existing one changes status. Stale entries are a documentation bug; please open a PR.

### Research-pass additions (2026-05)

Options `021` and `026` were added after a research pass against the 2026 LLM-gateway landscape; the rationale, competing approaches, and the items we deliberately declined (multi-provider adapters as a kernel feature, semantic cache reference impl, distributed state substrate) are recorded in [`docs/00-vision.md §4`](../00-vision.md) and [`docs/00-vision.md §8`](../00-vision.md). The optional Options (`022`, `027`, `028`, `029`) are gated on retrospective decisions at milestone close — if the kernel is already honest about the gap, we do not manufacture a doc just to have one.
