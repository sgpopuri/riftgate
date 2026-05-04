# crates/

The Rust workspace. **Empty during the `v0.0` public design phase.** Crates land as the milestones described in [`../docs/02-mvp-roadmap.md`](../docs/02-mvp-roadmap.md) reach implementation.

## Planned crate layout

| Crate | Purpose | Lands at |
|-------|---------|----------|
| `riftgate-core` | Trait surface (`AsyncIO`, `Scheduler`, `Queue`, `Allocator`, `TimerSubsystem`, `WAL`, `Filter`, `Router`, `ObservabilitySink`) and the small types they share | v0.1 |
| `riftgate-io-epoll` | epoll-based `AsyncIO` impl (also kqueue under `cfg(target_os = "macos")`) | v0.1 |
| `riftgate-io-uring` | io_uring-based `AsyncIO` impl behind a feature flag | v0.2 |
| `riftgate-parser` | FSM-based HTTP/1.1 + SSE parser | v0.1 |
| `riftgate-router` | Routing strategies (round-robin, weighted, KV-aware, hedged) | v0.1 → v0.3 |
| `riftgate-filter` | Filter chain executor + WASM runtime | v0.3 |
| `riftgate-obs` | OTel + Prometheus + Aya eBPF sinks | v0.1 → v0.4 |
| `riftgate-replay` | WAL writer + replay CLI | v0.2 → v1.0 |
| `riftgate-config` | Config parser, validator, hot-reload | v0.1 → v1.0 |
| `riftgate-operator` | Kubernetes operator + CRDs | v1.0 |
| `riftgate` | Main binary | v0.1 |

## Workspace conventions

- `Cargo.toml` at this directory's parent will declare the workspace; pin MSRV.
- Each crate has its own `README.md` with purpose, dependencies, and a one-paragraph design rationale that links to the relevant LLD and ADR.
- Public items are documented with rustdoc; `cargo doc --document-private-items` builds clean.
- Tests live alongside (`#[cfg(test)] mod tests`) for unit, in `tests/` for integration, in `benches/` for criterion.

## Reading order for new contributors

When the crates exist, read in this order:

1. `riftgate-core` — the trait surface tells you what is pluggable.
2. `riftgate-parser` — small, self-contained, demonstrates the FSM style.
3. `riftgate-io-epoll` — the simplest `AsyncIO` impl; shows the conformance test pattern.
4. `riftgate` — the main binary; shows how the pieces compose.
5. Then specific subsystem crates as needed.

Until then, read [`../docs/`](../docs/).
