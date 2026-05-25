# riftgate-io-epoll

`AsyncIO` impl backed by [`mio`](https://docs.rs/mio). On Linux this resolves to the kernel's `epoll(7)` interface; on macOS / *BSD it resolves to `kqueue(2)`. Both surfaces are wrapped in a single `MioIO` struct, with platform-conditional type aliases (`EpollIO`, `KqueueIO`) for callers that want to express the platform commitment explicitly.

This crate is the v0.1 IO substrate per [ADR 0002](../../docs/06-adrs/0002-start-on-epoll.md). The v0.2 `riftgate-io-uring` crate will live alongside it as a peer impl behind the same trait.

## Trait shape

`MioIO: AsyncIO` from `riftgate-core::io`. Per-shard ownership ([ADR 0004](../../docs/06-adrs/0004-per-shard-default-stealing-opt-in.md)) means each shard owns its own `MioIO`; cross-shard registration goes through the shard's work queue, not through a shared IO handle.

## Tests

- `tests/conformance.rs` — unit tests for register / deregister / poll round-trip behavior, including idempotent deregister and edge-triggered drain semantics.
- `benches/accept_echo.rs` — criterion benchmark for the accept + echo round-trip on a localhost TCP socket.
