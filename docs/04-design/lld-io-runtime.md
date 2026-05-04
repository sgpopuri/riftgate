# 04.a LLD — IO Runtime

> The async IO subsystem: epoll, io_uring, and the trait that abstracts them. The performance floor of the entire data plane.
>
> Status: **outline-stage**. Filled out as `v0.1` (epoll) and `v0.2` (io_uring) land.

## Purpose

Provide an `AsyncIO` trait that abstracts over readiness-based (epoll, kqueue) and completion-based (io_uring, IOCP) interfaces, with concrete implementations for the platforms Riftgate supports.

## Trait surface

```rust
// Sketch — actual signatures in riftgate-core
pub trait AsyncIO: Send + Sync {
    type Handle: Copy + Send;

    fn register(&self, fd: RawFd, interest: Interest) -> Result<Self::Handle>;
    fn deregister(&self, h: Self::Handle) -> Result<()>;
    fn poll(&self, timeout: Option<Duration>) -> Result<Vec<Event>>;
    // io_uring-only fast paths exposed via a separate ProvidesCompletion trait
}
```

## Implementations

| Impl | Platform | Status | Source crate |
|------|----------|--------|--------------|
| `EpollIO` | Linux | `v0.1` | `riftgate-io-epoll` |
| `UringIO` | Linux 5.10+ | `v0.2` (feature flag) | `riftgate-io-uring` |
| `KqueueIO` | macOS, BSD | `v0.1` (dev convenience) | `riftgate-io-epoll` (cfg) |

Decision rationale and rejected alternatives: see [Options 001 (IO model)](../05-options/001-io-model.md).

## Component context

### Architecture and dependencies

The IO subsystem sits between the kernel network stack and the rest of the data plane. It owns no business logic. Its only consumers are the `accept` loop and the worker shards, both of which are part of the [`scheduling`](lld-scheduling.md) subsystem. The IO subsystem itself depends on the [`allocator`](lld-allocator.md) for buffer management when zero-copy paths are involved.

### Patterns and conventions

- All IO is non-blocking. The trait is intentionally non-blocking-only.
- Event-driven, not callback-based. Workers explicitly `poll()` for events.
- Edge-triggered semantics where the underlying interface supports them (epoll ET, io_uring multishot). Workers must drain to `EAGAIN`.
- Zero-copy where possible (sendfile, splice, MSG_ZEROCOPY, io_uring SEND_ZC). See `Ch5 (ring buffers and zero-copy)`.

### Pitfalls (will grow as we hit them)

- **ET draining**: workers must read until `EAGAIN`. This is the most common bug class with ET epoll. See chapter 1 source material.
- **io_uring SQPOLL**: the kernel polling thread is hot CPU; only enable on dedicated boxes. Default is non-SQPOLL.
- **Buffer ownership in MSG_ZEROCOPY**: the buffer is owned by the kernel until the completion arrives via the error queue. Re-using it earlier corrupts in flight.
- **kqueue semantics differ from epoll** in subtle ways (one-shot vs persistent registration). Tests must cover both.

### Standards and review gates

- Every implementation must pass the same set of integration tests in `crates/riftgate-io-epoll/tests/conformance.rs`.
- Microbenchmarks against `epoll` baseline live in `benchmarks/io/`.
- The trait surface change requires a new ADR superseding [ADR 0002](../06-adrs/0002-start-on-epoll.md).

## Testing strategy

- Conformance suite — every `AsyncIO` impl runs the same suite.
- Microbenchmarks — req/s, syscalls/req, p99 latency for accept→read→echo→close.
- Fault injection — random EAGAIN, EINTR, partial reads.
- Long-running soak — 24h continuous load to surface fd leaks, slow timer drift.

## Open questions

- Should we expose a unified completion API even on epoll (synthesizing completions in userland)? Recommend no; let users opt into io_uring explicitly.
- How do we surface per-event metadata (e.g. number of bytes available)? Return in the `Event` struct; epoll has limited metadata, io_uring has more.
