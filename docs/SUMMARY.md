# Summary

[Riftgate](../README.md)

# Vision and requirements

- [Vision](00-vision.md)
- [Requirements](01-requirements/README.md)
  - [Functional](01-requirements/functional.md)
  - [Non-functional](01-requirements/non-functional.md)
  - [Personas](01-requirements/personas.md)

# Roadmap

- [MVP-to-v1.0 roadmap](02-mvp-roadmap.md)
- [v0.0 retrospective](02a-v0.0-retrospective.md)
- [v0.1 retrospective](02b-v0.1-retrospective.md)
- [v0.2 retrospective](02c-v0.2-retrospective.md)
- [v0.3 retrospective](02d-v0.3-retrospective.md)
- [v0.4 retrospective](02e-v0.4-retrospective.md)
- [v0.5 retrospective](02f-v0.5-retrospective.md)

# Architecture

- [High-level design](03-architecture/hld.md)
- [Data plane](03-architecture/data-plane.md)
- [Control plane](03-architecture/control-plane.md)
- [Extension plane](03-architecture/extension-plane.md)
- [Observability plane](03-architecture/observability-plane.md)

# Low-level design

- [LLD index](04-design/README.md)
  - [IO runtime](04-design/lld-io-runtime.md)
  - [Scheduling](04-design/lld-scheduling.md)
  - [Parsing](04-design/lld-parsing.md)
  - [Storage / WAL](04-design/lld-storage.md)
  - [Allocator](04-design/lld-allocator.md)
  - [Timers](04-design/lld-timers.md)
  - [Routing](04-design/lld-routing.md)
  - [Observability](04-design/lld-observability.md)
  - [Rate limiter](04-design/lld-rate-limiter.md)
  - [Filter chain](04-design/lld-filter-chain.md)
  - [MCP capability](04-design/lld-mcp-capability.md)

# Options docs

- [Options index](05-options/README.md)
  - [001 — IO model](05-options/001-io-model.md)
  - [002 — Async runtime](05-options/002-async-runtime.md)
  - [003 — Concurrency model](05-options/003-concurrency-model.md)
  - [004 — Request queue](05-options/004-request-queue.md)
  - [005 — Allocator](05-options/005-allocator.md)
  - [006 — Timer subsystem](05-options/006-timer-subsystem.md)
  - [007 — Protocol parser](05-options/007-protocol-parser.md)
  - [008 — Stream framing](05-options/008-stream-framing.md)
  - [009 — Request log](05-options/009-request-log.md)
  - [010 — Routing strategy](05-options/010-routing-strategy.md)
  - [011 — Circuit breaker](05-options/011-circuit-breaker.md)
  - [012 — Backpressure](05-options/012-backpressure.md)
  - [013 — Observability sink](05-options/013-observability-sink.md)
  - [014 — eBPF integration](05-options/014-ebpf-integration.md)
  - [015 — Configuration model](05-options/015-config-model.md)
  - [016 — Extension mechanism](05-options/016-extension-mechanism.md)
  - [019 — Replay-eval](05-options/019-replay-eval.md)
  - [021 — Rate limiting](05-options/021-rate-limiting.md)
  - [023 — Token-bucket parameters](05-options/023-token-bucket-parameters.md)
  - [024 — Stream cancellation](05-options/024-stream-cancellation.md)
  - [025 — v0.3 routing strategies](05-options/025-v03-routing-strategies.md)
  - [026 — MCP orchestration](05-options/026-mcp-orchestration.md)
  - [027 — Token-level metrics](05-options/027-token-level-metrics.md)
  - [028 — GPU-pressure correlation](05-options/028-gpu-pressure-correlation.md)
  - [017 — Multitenancy](05-options/017-multitenancy.md)
  - [018 — Deployment model](05-options/018-deployment.md)

# Architecture decisions

- [ADR index](06-adrs/README.md)
  - [0001 — Rust, not Go or Zig](06-adrs/0001-rust-not-go-or-zig.md)
  - [0002 — Start on epoll](06-adrs/0002-start-on-epoll.md)
  - [0003 — Tokio multi-thread default](06-adrs/0003-tokio-multithread-default.md)
  - [0004 — Per-shard scheduler default](06-adrs/0004-per-shard-default-stealing-opt-in.md)
  - [0005 — Sharded MPMC queue](06-adrs/0005-sharded-mpmc-queue.md)
  - [0006 — Bump arena + system malloc](06-adrs/0006-bump-arena-plus-system-malloc.md)
  - [0007 — Hand-rolled FSM parser](06-adrs/0007-handrolled-fsm-parser.md)
  - [0008 — SSE default, gRPC future](06-adrs/0008-sse-default-grpc-future.md)
  - [0009 — Rate limiter trait + in-proc only](06-adrs/0009-rate-limiter-trait-in-proc-only.md)
  - [0010 — Binary-heap timers in v0.1](06-adrs/0010-binary-heap-timers-v01-hierarchical-wheel-v02.md)
  - [0011 — OtelSink + MultiSink in v0.1](06-adrs/0011-otel-default-sink-multisink-fanout.md)
  - [0012 — Static TOML config in v0.1](06-adrs/0012-static-toml-env-override-v01.md)
  - [0013 — Append-only file WAL](06-adrs/0013-append-only-file-wal.md)
  - [0014 — Weighted-random router](06-adrs/0014-weighted-random-router.md)
  - [0015 — MCP as extension-plane broker](06-adrs/0015-mcp-extension-plane-broker.md)
  - [0016 — Three-state circuit breaker](06-adrs/0016-three-state-circuit-breaker.md)
  - [0017 — Drop-newest backpressure](06-adrs/0017-drop-newest-503-backpressure.md)
  - [0018 — Token-bucket parameters](06-adrs/0018-token-bucket-parameters.md)
  - [0019 — WASM extension mechanism](06-adrs/0019-wasm-extension-mechanism.md)
  - [0020 — Stream cancellation via CancellationToken](06-adrs/0020-stream-cancellation-cancellation-token.md)
  - [0021 — External replay CLI](06-adrs/0021-external-replay-cli.md)
  - [0022 — KV-aware routing via prefix trie](06-adrs/0022-kv-aware-routing-prefix-trie.md)
  - [0023 — Hedged requests p99-triggered](06-adrs/0023-hedged-requests-p99-triggered.md)
  - [0024 — eBPF via Aya](06-adrs/0024-ebpf-via-aya.md)
  - [0025 — Token-level metrics probabilistic](06-adrs/0025-token-level-metrics-probabilistic.md)
  - [0026 — GPU pressure via DCGM exporter](06-adrs/0026-gpu-pressure-via-dcgm-exporter.md)
  - [0027 — Per-shard bump-arena pool](06-adrs/0027-per-shard-bump-arena-pool.md)
  - [0028 — Timer cutover benchmark-gated](06-adrs/0028-timer-cutover-benchmark-gated.md)
  - [0029 — API-key tenant resolver](06-adrs/0029-api-key-tenant-resolver.md)
  - [0030 — Kubernetes operator with CRDs](06-adrs/0030-k8s-operator-crds.md)

# Reference

- [Glossary](08-glossary.md)

# Operations

- [Operator handbook](07-operator-handbook.md)
- [Troubleshooting](09-troubleshooting.md)
- [Upgrading](../UPGRADING.md)
