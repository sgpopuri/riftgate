# Riftgate

> A programmable AI data plane: a small Rust kernel + WASM extensions, with eBPF-native observability, where every internal decision is documented in public as a teaching artifact for modern systems engineering.

Riftgate is a repo-first exploration of the systems decisions behind a robust LLM gateway. The goal is not to start with a grand product announcement. The goal is to make the specs, options, decisions, architecture notes, and eventually code public as the project takes shape.

**Status: `v0.2` shipped the systems showpiece, `v0.3` shipped programmability, and `v0.4` shipped gateway-internal observability (eBPF runtime wiring, token-level metrics, GPU pressure correlation) and is closed out. The project is now in `v0.5` planning for the MCP capability-broker milestone.** The `v0.0` design surface — vision, requirements, four-plane architecture (data, control, extension, observability), low-level design notes, Options docs, and foundational ADRs — landed first. `v0.1` shipped the walking-skeleton crates (`riftgate-core`, `riftgate-io-epoll`, `riftgate-parser`, `riftgate-config`, `riftgate-router`, `riftgate-obs`, and the `riftgate` binary) and a self-contained [`examples/01-basic-openai-proxy`](examples/01-basic-openai-proxy/) dev loop. `v0.2` then added the `PerCoreScheduler`, `TokenBucketLimiter`, `WeightedRandomRouter`, `CircuitBreakerArbiter`, `HighWaterPolicy`, the new [`crates/riftgate-replay`](crates/riftgate-replay/) `FileWal`, and the [`crates/riftgate-io-uring`](crates/riftgate-io-uring/) scaffold. `v0.3` closed the programmability milestone with the native `FilterChain` executor, production `WasmFilter` runtime, `KvAwareRouter`, `HedgedRouter`, stream cancellation primitives, and the `riftgate-replay` CLI. `v0.4` closed the observability milestone with `TokenLevelAggregator`, `DcgmScrapeSource`, feature-gated `NvmlSource`, `BpfSink` runtime wiring, staged Aya object workflows, and GPU-aware routing signal integration. Retrospectives: [`v0.0`](docs/02a-v0.0-retrospective.md), [`v0.1`](docs/02b-v0.1-retrospective.md), [`v0.2`](docs/02c-v0.2-retrospective.md), [`v0.3`](docs/02d-v0.3-retrospective.md), [`v0.4`](docs/02e-v0.4-retrospective.md).

## Why Riftgate exists

LLM gateways are useful because they force old systems questions into a current problem:

- How should one process handle many concurrent, long-lived streaming requests?
- Where should work queue, and where should it be rejected?
- How should routing, rate limiting, backpressure, deadlines, replay, and observability compose?
- Which parts should be pluggable, and which parts should stay deliberately boring?

Riftgate uses that gateway-shaped problem to study the options behind robust, scalable, performance-sensitive infrastructure.

The design bet is a small Rust core where major subsystems are swappable behind traits, an extension surface for policy and filters, and observability that can eventually see below the HTTP layer. The documentation bet is just as important: decisions are written down before they disappear into code.

## What Riftgate explicitly is NOT

- Not a TensorZero killer. We will not promise to beat it on raw P99.
- Not an Envoy AI Gateway replacement. We will not match its ecosystem maturity.
- Not a vLLM-class KV-cache router. We integrate with `vllm-router` and `llm-d-kv-cache` where users want that fidelity.
- Not yet production-ready. The `v0.1` walking skeleton proxies OpenAI-shaped traffic, streams SSE, and emits OTel — but it is not hardened for production. Read [docs/02-mvp-roadmap.md](docs/02-mvp-roadmap.md).

## Repo and writing

The repo is the source material: specs, options, decisions, architecture notes, and eventually code. It is quiet right now; it will fill in as the project moves.

## How to read this repository

Read in this order if you are new:

1. **[`docs/00-vision.md`](docs/00-vision.md)** — north star, non-goals, differentiation pillars.
2. **[`docs/01-requirements/`](docs/01-requirements/)** — functional, non-functional, personas.
3. **[`docs/02-mvp-roadmap.md`](docs/02-mvp-roadmap.md)** — what ships when, milestone by milestone.
4. **[`docs/03-architecture/hld.md`](docs/03-architecture/hld.md)** — high-level design across data, control, extension, and observability planes.
5. **[`docs/05-options/`](docs/05-options/)** — every major decision is a numbered Options doc here. Start with [`001-io-model.md`](docs/05-options/001-io-model.md) for the flavor.
6. **[`docs/06-adrs/`](docs/06-adrs/)** — the corresponding decisions, in Michael-Nygard ADR format.

## Current focus

`v0.0` accepted the foundational design decisions for the kernel; `v0.1` shipped the walking-skeleton implementation against them. The shipped subsystems and the Options doc + ADR pair that govern each:

| Subsystem | Options doc | ADR | Shipped in |
|-----------|-------------|-----|------------|
| IO model | [`001-io-model`](docs/05-options/001-io-model.md) | [`0002`](docs/06-adrs/0002-start-on-epoll.md) | `crates/riftgate-io-epoll` (mio: epoll on Linux, kqueue on macOS) |
| Async runtime | [`002-async-runtime`](docs/05-options/002-async-runtime.md) | [`0003`](docs/06-adrs/0003-tokio-multithread-default.md) | `crates/riftgate` (tokio multi-thread runtime) |
| Concurrency model | [`003-concurrency-model`](docs/05-options/003-concurrency-model.md) | [`0004`](docs/06-adrs/0004-per-shard-default-stealing-opt-in.md) | trait surface in `riftgate-core` (per-shard default; production scheduler in v0.2) |
| Request queue | [`004-request-queue`](docs/05-options/004-request-queue.md) | [`0005`](docs/06-adrs/0005-sharded-mpmc-queue.md) | trait surface in `riftgate-core` (sharded MPMC impl in v0.2) |
| Allocator | [`005-allocator`](docs/05-options/005-allocator.md) | [`0006`](docs/06-adrs/0006-bump-arena-plus-system-malloc.md) | `BumpArena` + `SystemAllocator` in `riftgate-core` |
| Timer subsystem | [`006-timer-subsystem`](docs/05-options/006-timer-subsystem.md) | [`0010`](docs/06-adrs/0010-binary-heap-timers-v01-hierarchical-wheel-v02.md) | `BinaryHeapTimers` in `riftgate-core` |
| Protocol parser | [`007-protocol-parser`](docs/05-options/007-protocol-parser.md) | [`0007`](docs/06-adrs/0007-handrolled-fsm-parser.md) | `Http1Parser` in `riftgate-parser` |
| Stream framing | [`008-stream-framing`](docs/05-options/008-stream-framing.md) | [`0008`](docs/06-adrs/0008-sse-default-grpc-future.md) | `SseFramer` in `riftgate-parser` |
| Observability sink | [`013-observability-sink`](docs/05-options/013-observability-sink.md) | [`0011`](docs/06-adrs/0011-mpsc-bus-with-otel-sink.md) | `OtelSink` + `JsonStdoutSink` + `MultiSink` in `riftgate-obs` |
| Config model | [`015-config-model`](docs/05-options/015-config-model.md) | [`0012`](docs/06-adrs/0012-toml-plus-env-fail-loudly.md) | `riftgate-config` |
| Language choice | n/a (foundational) | [`0001`](docs/06-adrs/0001-rust-not-go-or-zig.md) | Rust, stable toolchain |

`v0.2` added five more decision pairs on top of the walking skeleton:

| Subsystem | Options doc | ADR | Shipped in |
|-----------|-------------|-----|------------|
| Per-core scheduler | [`003-concurrency-model`](docs/05-options/003-concurrency-model.md) | [`0004`](docs/06-adrs/0004-per-shard-default-stealing-opt-in.md), [`0005`](docs/06-adrs/0005-sharded-mpmc-queue.md) | `PerCoreScheduler` + `ShardedMpmcQueue` in `crates/riftgate` |
| Rate limiting | [`021-rate-limiting`](docs/05-options/021-rate-limiting.md), [`023-token-bucket-parameters`](docs/05-options/023-token-bucket-parameters.md) | [`0009`](docs/06-adrs/0009-rate-limiter-trait-in-proc-only.md), [`0018`](docs/06-adrs/0018-token-bucket-parameters.md) | `TokenBucketLimiter` in `crates/riftgate-core` |
| Weighted + circuit-breaker routing | [`010-routing-strategy`](docs/05-options/010-routing-strategy.md), [`011-circuit-breaker`](docs/05-options/011-circuit-breaker.md) | [`0014`](docs/06-adrs/0014-weighted-random-router.md), [`0016`](docs/06-adrs/0016-three-state-circuit-breaker.md) | `WeightedRandomRouter` + `CircuitBreakerArbiter` in `crates/riftgate-router` |
| Backpressure | [`012-backpressure`](docs/05-options/012-backpressure.md) | [`0017`](docs/06-adrs/0017-drop-newest-503-backpressure.md) | `HighWaterPolicy` in `crates/riftgate-core` |
| Request log / WAL | [`009-request-log`](docs/05-options/009-request-log.md) | [`0013`](docs/06-adrs/0013-append-only-file-wal.md) | `FileWal` in `crates/riftgate-replay` |
| Second IO impl | [`001-io-model`](docs/05-options/001-io-model.md) | [`0002`](docs/06-adrs/0002-start-on-epoll.md) (extension) | `crates/riftgate-io-uring` scaffold |

`v0.3` and `v0.4` are both landed and closed out. The active project focus is `v0.5` planning and sequencing for the MCP capability-broker milestone: first-class MCP request parsing, per-tenant tool and resource allowlists, WAL-backed capability audit events, and downstream attestation headers. The governing design surface is already in place via [Options `026`](docs/05-options/026-mcp-orchestration.md) and [ADR `0015`](docs/06-adrs/0015-mcp-extension-plane-broker.md), and the authoritative live status remains the **Currently shipping** block at the top of the [MVP roadmap](docs/02-mvp-roadmap.md). See the [Options index](docs/05-options/README.md) and the [ADR index](docs/06-adrs/README.md) for the full decision history.

## v0.5 scope summary

`v0.5` is the agentic capability-plane milestone. The project stops being only an OpenAI-compatible request gateway and becomes a capability broker for Model Context Protocol traffic inside the existing extension plane.

- **MCP request parsing and brokering.** A dedicated `riftgate-mcp` surface implements the `CapabilityBroker` trait and understands MCP tool/resource invocations as first-class gateway events.
- **Per-tenant capability policy.** Each tenant gets an explicit allowlist for tools and resources, so authorization is enforced at the gateway rather than pushed into downstream services by convention.
- **Durable audit trail.** Every `tools/call` decision is written to the WAL and surfaced to observability as structured audit data, turning the gateway into the capability ledger for agent workflows.
- **Downstream attestation.** The gateway emits attestation headers such as caller, tool, and decision so downstream systems can apply policy based on the brokered decision rather than reconstructing intent from raw payloads.
- **Deliberate non-goals for this phase.** `v0.5` is not the Kubernetes/operator hardening milestone, and it does not advance crates.io distribution; those remain `v1.0` decisions.

The design basis is already documented in [Options `026`](docs/05-options/026-mcp-orchestration.md) and [ADR `0015`](docs/06-adrs/0015-mcp-extension-plane-broker.md). Implementation sequencing is next; the live status source remains the [MVP roadmap](docs/02-mvp-roadmap.md).

**Distribution:** through v0.4, install is **build from source** only (no [crates.io](https://crates.io) publish). Whether we add `cargo install` is a **v1.0** decision — see [Distribution](docs/02-mvp-roadmap.md#distribution-cratesio) in the roadmap.

For day-to-day build, test, run, and bench commands, see the [`RUNBOOK.md`](RUNBOOK.md).

```bash
git clone https://github.com/sgpopuri/riftgate.git && cd riftgate
cargo build --release -p riftgate
./target/release/riftgate --config examples/01-basic-openai-proxy/riftgate.toml
```

To run the walking skeleton against a mock OpenAI backend, see [`examples/01-basic-openai-proxy`](examples/01-basic-openai-proxy/).

## How to contribute

Right now: read, comment, open issues, and critique the design. The project especially welcomes engineers with a critic's eye: people who can punch holes in proposals, point out missed failure scenarios, question hidden assumptions, and improve the options before the code hardens around them.

See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Working with AI agents on this project

If you are an agent, or a human running an agent, working in this repo, read [`AGENTS.md`](AGENTS.md) before editing.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
