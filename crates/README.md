# crates/

The Rust workspace. The `v0.1` walking skeleton is in. Additional crates land as the milestones in [`../docs/02-mvp-roadmap.md`](../docs/02-mvp-roadmap.md) reach implementation.

## Crate layout

| Crate | Purpose | Status |
|-------|---------|--------|
| `riftgate-core` | Trait surface (`AsyncIO`, `Scheduler`, `Queue`, `Allocator`, `TimerSubsystem`, `WAL`, `Filter`, `Router`, `ObservabilitySink`, `RateLimiter`, `CapabilityBroker`) plus shared types and the in-core impls (`SystemAllocator`, `BumpArena`, `BinaryHeapTimers`, `DeterministicTimers`, `IdentityFilter`, `LoggingFilter`, `InMemorySink`) | shipped (v0.1) |
| `riftgate-io-epoll` | mio-backed `AsyncIO` impl (epoll on Linux, kqueue under `cfg(target_os = "macos")`) | shipped (v0.1) |
| `riftgate-parser` | `Http1Parser` (httparse + hand-rolled body FSM) and `SseFramer` | shipped (v0.1) |
| `riftgate-config` | TOML schema + env-override loader + fail-loudly validator + `Secret<T>` redaction | shipped (v0.1) |
| `riftgate-router` | `RoundRobinRouter` (atomic cursor) + `ConstantRouter` test impl | shipped (v0.1) |
| `riftgate-obs` | Bounded MPSC bus with drop-on-full + `OtelSink` + `JsonStdoutSink` + `MultiSink` + canonical span-name registry | shipped (v0.1) |
| `riftgate` | Main binary: tokio multi-thread runtime, accept loop, hyper-rustls upstream client, SSE forwarding, `/health` + `/ready`, SIGTERM drain | shipped (v0.1) |
| `riftgate-io-uring` | io_uring-based `AsyncIO` impl behind a feature flag | v0.2 |
| `riftgate-replay` | WAL writer + replay CLI | v0.2 → v1.0 |
| `riftgate-filter` | Filter chain executor + WASM runtime | v0.3 |
| `riftgate-mcp` | MCP capability broker impl | v0.5 |
| `riftgate-operator` | Kubernetes operator + CRDs | v1.0 |

## Workspace conventions

- `Cargo.toml` at this directory's parent declares the workspace; MSRV is pinned in `rust-toolchain.toml`.
- Each crate has its own `README.md` with purpose, dependencies, and a one-paragraph design rationale that links to the relevant LLD and ADR.
- Public items are documented with rustdoc; `cargo doc --document-private-items` builds clean.
- Tests live alongside (`#[cfg(test)] mod tests`) for unit, in `tests/` for integration, in `benches/` for criterion.
- `cargo clippy --deny warnings` and `cargo fmt --check` must pass.

## Reading order for new contributors

1. `riftgate-core` — the trait surface tells you what is pluggable.
2. `riftgate-parser` — small, self-contained, demonstrates the FSM style.
3. `riftgate-io-epoll` — the simplest `AsyncIO` impl; shows the conformance test pattern.
4. `riftgate` — the main binary; shows how the pieces compose.
5. Then specific subsystem crates as needed.

To run the walking skeleton end-to-end against a mock OpenAI backend, see [`../examples/01-basic-openai-proxy`](../examples/01-basic-openai-proxy/).
