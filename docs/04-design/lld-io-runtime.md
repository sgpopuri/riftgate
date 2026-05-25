# 04.a LLD — IO Runtime

> The async IO subsystem: epoll, io_uring, and the trait that abstracts them. The performance floor of the entire data plane.
>
> Status: **shipped (v0.1, epoll + kqueue via mio)**. `UringIO` lands in v0.2 behind a Cargo feature flag per [ADR 0002](../06-adrs/0002-start-on-epoll.md).

## Purpose

Provide an `AsyncIO` trait that abstracts over readiness-based (epoll, kqueue) and completion-based (io_uring, IOCP) interfaces, with concrete implementations for the platforms Riftgate supports.

## Trait surface

The shipped trait — see [`crates/riftgate-core/src/io.rs`](../../crates/riftgate-core/src/io.rs):

```rust
pub struct Interest(u8);  // READABLE | WRITABLE | READABLE_AND_WRITABLE

pub struct Event {
    pub token: u64,
    pub readable: bool,
    pub writable: bool,
}

pub trait AsyncIO {
    fn register(&mut self, fd: RawFd, token: u64, interest: Interest) -> io::Result<()>;
    fn deregister(&mut self, fd: RawFd) -> io::Result<()>;
    fn poll(&mut self, timeout: Option<Duration>) -> io::Result<Vec<Event>>;
}
```

Three design adjustments from the v0.0 sketch:

- **Methods take `&mut self`, no `Send + Sync` bound.** Per [ADR 0004](../06-adrs/0004-per-shard-default-stealing-opt-in.md) each shard owns its own IO instance; cross-shard work goes through the per-shard work queue, never through a shared IO handle. This matches `mio::Poll`'s native API (`poll(&mut self, ...)`) without requiring an interior mutex on the hot path.
- **No associated `Handle` type.** Tokens are plain `u64` values supplied at registration time. This keeps the trait dyn-safe (`Box<dyn AsyncIO>` works) so the binary can pick the impl at startup.
- **Completion-based fast paths are deferred.** v0.1 ships only the readiness-based surface; `UringIO` will live behind a separate `ProvidesCompletion` trait when v0.2 lands, keeping the readiness path unchanged.

## Implementations

| Impl | Platform | Status | Source crate | Notes |
|------|----------|--------|--------------|-------|
| `MioIO` (alias `EpollIO`) | Linux | shipped (v0.1) | `riftgate-io-epoll` | mio-backed; epoll under the hood. Edge-triggered by default. |
| `MioIO` (alias `KqueueIO`) | macOS, BSD | shipped (v0.1, dev convenience) | `riftgate-io-epoll` (cfg) | Same crate, same impl; mio selects the kqueue backend automatically. |
| `UringIO` | Linux 5.10+ | v0.2 (feature flag) | `riftgate-io-uring` | Behind `--features io-uring`. Adds a `ProvidesCompletion` trait alongside `AsyncIO`. |

Decision rationale and rejected alternatives: see [Options 001 (IO model)](../05-options/001-io-model.md).

## Component context

### Architecture and dependencies

The IO subsystem sits between the kernel network stack and the rest of the data plane. It owns no business logic. Its only consumers in v0.1 are the per-shard worker loops in `crates/riftgate/src/server.rs` and the conformance harness in `crates/riftgate-io-epoll/tests/conformance.rs`.

External dependencies are tightly scoped:

- `mio` for the readiness-poll abstraction. mio internally selects `epoll` on Linux and `kqueue` on macOS/BSD, so a single Rust impl in `crates/riftgate-io-epoll` covers both targets.
- `std::os::fd::RawFd` for fd handling — no platform-specific Rust types leak into the trait.

### Patterns and conventions

- **Per-shard ownership.** Each shard's `AsyncIO` instance is private to the shard; `&mut self` enforces this at the type level. Cross-shard work goes through the per-shard work queue, never through a shared IO handle.
- **All IO is non-blocking.** The trait is intentionally non-blocking-only; all registered fds must be in non-blocking mode before registration.
- **Event-driven, not callback-based.** Workers explicitly `poll()` for events on each loop iteration.
- **Edge-triggered by default** on epoll. Workers must drain each ready fd to `EAGAIN` before returning to `poll`.
- **Token-correlated events.** The `token: u64` supplied at registration is returned in every `Event` — callers correlate back to their per-fd state without searching a map.
- **Zero-copy paths are out of scope for v0.1.** `sendfile`, `splice`, `MSG_ZEROCOPY`, and `io_uring SEND_ZC` are deferred to v0.2 along with `UringIO`. See [Options 008](../05-options/008-stream-framing.md) §5; the canonical references are the `splice(2)` and `sendfile(2)` man pages and the LMAX Disruptor design notes.

### Pitfalls

- **ET draining.** Workers must read until `EAGAIN` after every wakeup. This is the most common bug class with edge-triggered epoll; the conformance test suite explicitly exercises it.
- **kqueue semantics differ from epoll** in subtle ways (one-shot vs persistent registration). mio papers over the difference but the conformance suite covers both targets to catch any leak.
- **Re-registering the same fd** updates interest and token in-place; do not deregister-then-register on the hot path (it costs an extra syscall).
- **`EINTR` is retried internally.** Callers should never see it; if they do, that is a bug in the impl.
- **`UringIO` SQPOLL** (v0.2) — the kernel polling thread is hot CPU; only enable on dedicated boxes. Default will be non-SQPOLL.
- **Buffer ownership in `MSG_ZEROCOPY`** (v0.2) — the buffer is owned by the kernel until the completion arrives via the error queue. Reusing it earlier corrupts in flight.

### Standards and review gates

- Every `AsyncIO` impl must pass the conformance suite in [`crates/riftgate-io-epoll/tests/conformance.rs`](../../crates/riftgate-io-epoll/tests/conformance.rs).
- Microbenchmarks live in [`crates/riftgate-io-epoll/benches/accept_echo.rs`](../../crates/riftgate-io-epoll/benches/accept_echo.rs) and gate accept→read→echo throughput.
- The trait surface is part of the v0.1 frozen surface — changes require a new ADR superseding [ADR 0002](../06-adrs/0002-start-on-epoll.md).

## Testing strategy

- Conformance suite — every `AsyncIO` impl runs the same suite (Linux epoll and macOS kqueue both go through `MioIO`).
- Microbenchmarks — req/s, syscalls/req, p99 latency for accept→read→echo→close.
- Fault injection — random `EAGAIN`, `EINTR`, partial reads.
- Long-running soak (v0.2) — 24 h continuous load to surface fd leaks and slow timer drift.

## Open questions

- Should we expose a unified completion API even on epoll (synthesizing completions in userland)? Recommend no; let users opt into io_uring explicitly when v0.2 lands.
- How do we surface per-event metadata (e.g. number of bytes available)? Return in the `Event` struct; epoll has limited metadata, io_uring has more. Decide when `UringIO` lands.
