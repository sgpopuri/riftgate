# Riftgate

> A programmable AI data plane: a small Rust kernel + WASM extensions, with eBPF-native observability, where every internal decision is documented in public as a teaching artifact for modern systems engineering.

Riftgate is a repo-first exploration of the systems decisions behind a robust LLM gateway. The goal is not to start with a grand product announcement. The goal is to make the specs, options, decisions, architecture notes, and eventually code public as the project takes shape.

**Status: `v0.0` complete (2026-05-03); `v0.1` (walking skeleton — first Rust binary) is the next milestone.** No Rust code yet. The `v0.0` design surface — vision, requirements, architecture, low-level design notes, eight foundational Options docs, and eight ADRs — is in the repo. `v0.1` adds the first crate scaffolding plus three remaining prerequisite Options docs (`006-timer-subsystem`, `013-observability-sink`, `015-config-model`).

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
- Not yet production-ready. Not even `v0.1`. Read [docs/02-mvp-roadmap.md](docs/02-mvp-roadmap.md).

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

`v0.0` has accepted the foundational design decisions for the kernel:

| Subsystem | Options doc | ADR |
|-----------|-------------|-----|
| IO model | [`001-io-model`](docs/05-options/001-io-model.md) | [`0002`](docs/06-adrs/0002-start-on-epoll.md) |
| Async runtime | [`002-async-runtime`](docs/05-options/002-async-runtime.md) | [`0003`](docs/06-adrs/0003-tokio-multithread-default.md) |
| Concurrency model | [`003-concurrency-model`](docs/05-options/003-concurrency-model.md) | [`0004`](docs/06-adrs/0004-per-shard-default-stealing-opt-in.md) |
| Request queue | [`004-request-queue`](docs/05-options/004-request-queue.md) | [`0005`](docs/06-adrs/0005-sharded-mpmc-queue.md) |
| Allocator | [`005-allocator`](docs/05-options/005-allocator.md) | [`0006`](docs/06-adrs/0006-bump-arena-plus-system-malloc.md) |
| Protocol parser | [`007-protocol-parser`](docs/05-options/007-protocol-parser.md) | [`0007`](docs/06-adrs/0007-handrolled-fsm-parser.md) |
| Stream framing | [`008-stream-framing`](docs/05-options/008-stream-framing.md) | [`0008`](docs/06-adrs/0008-sse-default-grpc-future.md) |
| Language choice | n/a (foundational) | [`0001`](docs/06-adrs/0001-rust-not-go-or-zig.md) |

`v0.1` (walking skeleton) is next. See the [`v0.1` section of the MVP roadmap](docs/02-mvp-roadmap.md#v01--walking-skeleton) for the deliverable list, or the [Options index](docs/05-options/README.md) and [ADR index](docs/06-adrs/README.md) for the full decision history.

## How to contribute

Right now: read, comment, open issues, and critique the design. The project especially welcomes engineers with a critic's eye: people who can punch holes in proposals, point out missed failure scenarios, question hidden assumptions, and improve the options before the code hardens around them.

See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## Working with AI agents on this project

If you are an agent, or a human running an agent, working in this repo, read [`AGENTS.md`](AGENTS.md) before editing.

## License

Apache-2.0. See [`LICENSE`](LICENSE).
