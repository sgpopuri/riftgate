# riftgate-core

The kernel trait surface and shared types that every Riftgate subsystem plugs into. Defines the eleven load-bearing traits — `AsyncIO`, `StreamParser`, `Scheduler`, `Queue`, `Allocator`, `TimerSubsystem`, `Filter`, `Router`, `ObservabilitySink`, plus the deferred-impl traits `RateLimiter`, `WAL`, and `CapabilityBroker` — and the small in-core implementations that are universal across deployments (`SystemAllocator`, `BumpArena`, `BinaryHeapTimers`, `DeterministicTimers`, `IdentityFilter`, `LoggingFilter`).

Read order for new contributors:

1. The crate-level rustdoc in [`src/lib.rs`](src/lib.rs) — a single-page tour of the trait surface with an ASCII map.
2. The LLD for whichever subsystem you intend to touch ([`docs/04-design/`](../../docs/04-design/)).
3. The Options doc and ADR that govern the trait shape.

This crate carries `#![deny(unsafe_code)]`. The bump arena uses the `bumpalo` crate, which encapsulates its own unsafe; we do not write raw unsafe in `riftgate-core`.

Per-trait impl status (FR-X02 in [`docs/01-requirements/functional.md`](../../docs/01-requirements/functional.md) requires every trait to have at least two implementations or a documented reason for one):

| Trait | First impl | Second impl | Notes |
|-------|------------|-------------|-------|
| `AsyncIO` | `EpollIO` (in `riftgate-io-epoll`) | `KqueueIO` (in `riftgate-io-epoll`, `cfg(macos)`) | `UringIO` lands in `v0.2`. |
| `StreamParser` | `Http1Parser` (in `riftgate-parser`) | `SseFramer` (in `riftgate-parser`) | Different parsers; same trait. |
| `Scheduler` | `PerShardScheduler` (in `riftgate` binary) | `#[cfg(test)] SimpleScheduler` here | Trait + test impl ship in core; production impl lives in the binary. |
| `Queue<T>` | `CrossbeamMpmcQueue<T>` here | `#[cfg(test)] MutexQueue<T>` here | crossbeam-channel adapter is the v0.1 default per ADR 0005. |
| `Allocator` | `SystemAllocator` here | `BumpArena` here | ADR 0006. `BumpArena` is non-Sync and non-Send by design. |
| `TimerSubsystem` | `BinaryHeapTimers` here | `DeterministicTimers` here | ADR 0010. |
| `Filter` | `IdentityFilter` here | `LoggingFilter` here | Filter chain executor lands in `riftgate-filter` in `v0.3`. |
| `Router` | `RoundRobinRouter` (in `riftgate-router`) | `ConstantRouter` (in `riftgate-router`) | LLD-routing. |
| `ObservabilitySink` | `OtelSink` (in `riftgate-obs`) | `MultiSink` + `JsonStdoutSink` + `InMemorySink` | ADR 0011. `InMemorySink` lives here for unit tests. |
| `RateLimiter` | deferred to `v0.2` (ADR 0009) | n/a | Trait-only in `v0.1`; documented reason for one. |
| `WAL` | deferred to `v0.2` | n/a | Trait-only in `v0.1`; documented reason for one. |
| `CapabilityBroker` | deferred to `v0.5` (ADR 0015) | n/a | Trait-only in `v0.1`; documented reason for one. |
