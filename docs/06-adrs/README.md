# Architecture Decision Records (ADRs)

Each ADR captures a decision: context, decision, consequences. Format is Michael Nygard's. Decisions are numbered and immutable — supersede with a new ADR rather than editing in place.

## Conventions

- **Numbered sequentially.** `0001`, `0002`, … Numbering is permanent.
- **Status reflects current.** `proposed` → `accepted`. A superseded ADR keeps `superseded by ADR-NNNN` forever.
- **Decision is a sentence.** If you need a paragraph, you are writing an Options doc, not an ADR.
- **Compliance is explicit.** Say how the decision is enforced — CI, lint, review.

## How to add a new ADR

1. Identify the corresponding Options doc in [`../05-options/`](../05-options/). If none exists, write one first.
2. Copy [`_template.md`](_template.md) to `NNNN-<slug>.md` with the next free number.
3. Fill in. Be brief and decisive.
4. Update both this index and the Options doc's frontmatter to link the ADR.
5. Open a PR. ADR PRs require explicit reviewer signoff (no auto-merge) because the cost of a bad decision compounds.

## Index

| # | Title | Status | Date | Options doc |
|---|-------|--------|------|-------------|
| 0001 | [Rust, not Go or Zig, for the kernel](0001-rust-not-go-or-zig.md) | accepted | 2026-05-02 | n/a (foundational) |
| 0002 | [Start on epoll, add io_uring as feature flag](0002-start-on-epoll.md) | accepted | 2026-05-02 | [001-io-model](../05-options/001-io-model.md) |
| 0003 | [Tokio multi-threaded runtime as the only v0.1 runtime; per-core runtimes revisited at v0.2 retro](0003-tokio-multithread-default.md) | accepted | 2026-05-03 | [002-async-runtime](../05-options/002-async-runtime.md) |
| 0004 | [Shared-nothing per-shard scheduler in v0.1; work-stealing as v0.2 opt-in](0004-per-shard-default-stealing-opt-in.md) | accepted | 2026-05-03 | [003-concurrency-model](../05-options/003-concurrency-model.md) |
| 0005 | [Sharded MPMC queue strategy; crossbeam-channel in v0.1, hand-rolled Vyukov in v0.2](0005-sharded-mpmc-queue.md) | accepted | 2026-05-03 | [004-request-queue](../05-options/004-request-queue.md) |
| 0006 | [Per-request bump arena on the hot path; system malloc globally in v0.1; mimalloc opt-in in v0.2](0006-bump-arena-plus-system-malloc.md) | accepted | 2026-05-03 | [005-allocator](../05-options/005-allocator.md) |
| 0007 | [Hand-rolled table-driven FSM in riftgate-parser; httparse for headers in v0.1; full FSM in v0.2](0007-handrolled-fsm-parser.md) | accepted | 2026-05-03 | [007-protocol-parser](../05-options/007-protocol-parser.md) |
| 0008 | [SSE as the only v0.1 streaming framing; NDJSON optional in v0.2+; gRPC bidi deferred to v1.0+](0008-sse-default-grpc-future.md) | accepted | 2026-05-03 | [008-stream-framing](../05-options/008-stream-framing.md) |
| 0009 | [Rate limiter trait + in-proc token-bucket only in v0.2; distributed impls deferred](0009-rate-limiter-trait-in-proc-only.md) | accepted | 2026-05-25 | [021-rate-limiting](../05-options/021-rate-limiting.md), [023-token-bucket-parameters](../05-options/023-token-bucket-parameters.md) |
| 0010 | [Binary-heap timer subsystem in v0.1; hierarchical wheel in v0.2 behind the same trait](0010-binary-heap-timers-v01-hierarchical-wheel-v02.md) | accepted | 2026-05-10 | [006-timer-subsystem](../05-options/006-timer-subsystem.md) |
| 0011 | [ObservabilitySink trait + bounded-MPSC bus + OtelSink + MultiSink in v0.1](0011-otel-default-sink-multisink-fanout.md) | accepted | 2026-05-10 | [013-observability-sink](../05-options/013-observability-sink.md) |
| 0012 | [Static TOML configuration with env-var overrides; safe-subset hot reload deferred to v0.2 / v0.3; CRDs in v1.0](0012-static-toml-env-override-v01.md) | accepted | 2026-05-10 | [015-config-model](../05-options/015-config-model.md) |
| 0013 | [Per-shard append-only file WAL with group-commit fdatasync; RocksDB and SQLite rejected](0013-append-only-file-wal.md) | accepted | 2026-05-25 | [009-request-log](../05-options/009-request-log.md) |
| 0014 | [Weighted-random router (Walker alias method) added in v0.2; KV-aware and hedged deferred to v0.3](0014-weighted-random-router.md) | accepted | 2026-05-25 | [010-routing-strategy](../05-options/010-routing-strategy.md) |
| 0015 | [MCP as a first-class citizen of the extension plane (gateway-as-broker)](0015-mcp-extension-plane-broker.md) | proposed | TBD (target: open of `v0.5`) | [026-mcp-orchestration](../05-options/026-mcp-orchestration.md) |
| 0016 | [Three-state circuit breaker per backend; sliding-window and adaptive deferred](0016-three-state-circuit-breaker.md) | accepted | 2026-05-25 | [011-circuit-breaker](../05-options/011-circuit-breaker.md) |
| 0017 | [Drop-newest 503 backpressure with high/low water marks; adaptive concurrency deferred](0017-drop-newest-503-backpressure.md) | accepted | 2026-05-25 | [012-backpressure](../05-options/012-backpressure.md) |
| 0018 | [TokenBucketLimiter parameter set: packed AtomicU64 with SCALE = 65536, 64 DashMap shards](0018-token-bucket-parameters.md) | accepted | 2026-05-25 | [023-token-bucket-parameters](../05-options/023-token-bucket-parameters.md) |
| 0019 | [WASM extension mechanism via wasmtime with frozen `riftgate:filter/v1` component-model ABI](0019-wasm-extension-mechanism.md) | accepted | 2026-05-25 | [016-extension-mechanism](../05-options/016-extension-mechanism.md) |
| 0020 | [Stream cancellation via `tokio_util::sync::CancellationToken` wrapped in a typed `Cancellation` newtype](0020-stream-cancellation-cancellation-token.md) | accepted | 2026-05-25 | [024-stream-cancellation](../05-options/024-stream-cancellation.md) |
| 0021 | [External `riftgate-replay` CLI binary with `dump`, `replay`, `eval` subcommands](0021-external-replay-cli.md) | accepted | 2026-05-25 | [019-replay-eval](../05-options/019-replay-eval.md) |
| 0022 | [KV-cache-aware routing via an in-tree prefix trie with xxHash3-64 byte-hashing](0022-kv-aware-routing-prefix-trie.md) | accepted | 2026-05-25 | [025-v03-routing-strategies](../05-options/025-v03-routing-strategies.md) |
| 0023 | [Hedged requests via Dean–Barroso threshold-triggered shape, degree=2, rate-limit-budget-aware](0023-hedged-requests-p99-triggered.md) | accepted | 2026-05-25 | [025-v03-routing-strategies](../05-options/025-v03-routing-strategies.md) |
| 0024 | [eBPF integration via Aya (pure-Rust BPF), Linux 5.15+, feature-gated and opt-in](0024-ebpf-via-aya.md) | accepted | 2026-05-25 | [014-ebpf-integration](../05-options/014-ebpf-integration.md) |
| 0025 | [Token-level metrics via reservoir-sampled OTel spans + HDR-histogram aggregates + per-token WAL records](0025-token-level-metrics-probabilistic.md) | accepted | 2026-05-25 | [027-token-level-metrics](../05-options/027-token-level-metrics.md) |
| 0026 | [GPU pressure correlation via DCGM exporter scrape (primary) and NVML in-process FFI (escape hatch)](0026-gpu-pressure-via-dcgm-exporter.md) | accepted | 2026-05-25 | [028-gpu-pressure-correlation](../05-options/028-gpu-pressure-correlation.md) |

ADR `0015` is listed above as `proposed` because its Options doc is already authored and its decision is already framed; it will move to `accepted` at the open of `v0.5`.

## Status legend

- **proposed** — under discussion. The decision is not yet binding.
- **accepted** — current. Code and docs follow this.
- **superseded by ADR-NNNN** — historical. The new ADR is current.
- **deprecated** — was accepted, no longer applies, and was not superseded by another decision (e.g. the area of concern no longer exists).
