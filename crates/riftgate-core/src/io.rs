//! Async IO trait and primitives.
//!
//! `AsyncIO` is the abstraction layer between Riftgate's data plane and the
//! underlying OS event-multiplexing primitive (`epoll` on Linux, `kqueue` on
//! macOS, `io_uring` in `v0.2`).
//!
//! ```text
//!   socket fd  --register(EPOLLIN|EPOLLET)-->  AsyncIO::register
//!                                                     |
//!                                                     v
//!                                          (kernel watches the fd)
//!                                                     |
//!                                                     v
//!   AsyncIO::poll(timeout) -----> Vec<Event>          (one event per ready fd)
//!                                       |
//!                                       v
//!                              worker reads/writes the fd to EAGAIN
//! ```
//!
//! See [`docs/04-design/lld-io-runtime.md`](../../../docs/04-design/lld-io-runtime.md)
//! for the full design rationale and pitfalls.

use std::io;
use std::os::fd::RawFd;
use std::time::Duration;

/// Set of IO interests for a registered file descriptor.
///
/// An `Interest` is the union of read- and write-readiness flags that a
/// caller wants to be notified about. Mirrors `EPOLLIN` / `EPOLLOUT` on Linux
/// and `EVFILT_READ` / `EVFILT_WRITE` on BSD/macOS.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct Interest(u8);

impl Interest {
    const READABLE_BIT: u8 = 0b0000_0001;
    const WRITABLE_BIT: u8 = 0b0000_0010;

    /// Notify when the fd becomes readable.
    pub const READABLE: Self = Self(Self::READABLE_BIT);
    /// Notify when the fd becomes writable.
    pub const WRITABLE: Self = Self(Self::WRITABLE_BIT);
    /// Notify on either readable or writable.
    pub const READABLE_AND_WRITABLE: Self = Self(Self::READABLE_BIT | Self::WRITABLE_BIT);

    /// `true` if the read interest is set.
    #[inline]
    pub fn is_readable(self) -> bool {
        self.0 & Self::READABLE_BIT != 0
    }

    /// `true` if the write interest is set.
    #[inline]
    pub fn is_writable(self) -> bool {
        self.0 & Self::WRITABLE_BIT != 0
    }
}

/// Single IO event surfaced by [`AsyncIO::poll`].
///
/// The `token` field is opaque to the trait — concrete implementations
/// assign it at registration time so callers can correlate events back to
/// their own per-fd state.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub struct Event {
    /// Caller-supplied opaque token from registration time.
    pub token: u64,
    /// `true` if the fd is readable.
    pub readable: bool,
    /// `true` if the fd is writable.
    pub writable: bool,
}

/// Async IO trait.
///
/// Concrete implementations live outside `riftgate-core`:
///
/// - `EpollIO` (Linux, alias of `MioIO`) — see `crates/riftgate-io-epoll`.
/// - `KqueueIO` (macOS / BSD, alias of `MioIO`) — see `crates/riftgate-io-epoll`.
/// - `UringIO` (Linux 5.10+, `--features io-uring`, `v0.2`+) — see `crates/riftgate-io-uring`.
///
/// The trait is intentionally non-blocking only. All registered fds must be
/// in non-blocking mode; the trait does not provide synchronous IO.
///
/// **Per-shard ownership; not `Send + Sync`.** Per [ADR
/// 0004](../../../docs/06-adrs/0004-per-shard-default-stealing-opt-in.md)
/// each shard owns its own IO instance. Cross-shard registration goes
/// through the per-shard work queue, not through a shared IO handle. This
/// matches `mio::Poll`'s native API (`poll(&mut self, ...)`) without
/// requiring an interior mutex on the hot path.
///
/// **Edge-triggered semantics where the underlying interface supports them.**
/// On Linux with `EPOLLET`, callers must drain to `EAGAIN` after every wakeup
/// or risk missing events. This is the most common bug class with
/// edge-triggered epoll; the conformance test suite in
/// `crates/riftgate-io-epoll/tests/conformance.rs` covers it explicitly.
///
/// **Trait object safety.** The trait is dyn-safe (no generics, no associated
/// types, methods take `&mut self`). Wrapping in `Box<dyn AsyncIO>` lets the
/// data plane choose between epoll, kqueue, and io_uring at startup.
pub trait AsyncIO {
    /// Register a file descriptor for notification on the given interest.
    ///
    /// `token` is an opaque identifier the implementation will return in
    /// every [`Event`] for this fd. Re-registering an already-registered fd
    /// updates the interest and token in-place.
    ///
    /// Returns the underlying OS error if registration fails (e.g. fd is
    /// invalid, max-watches reached).
    fn register(&mut self, fd: RawFd, token: u64, interest: Interest) -> io::Result<()>;

    /// Deregister a file descriptor.
    ///
    /// Idempotent: deregistering an unknown fd is a no-op.
    fn deregister(&mut self, fd: RawFd) -> io::Result<()>;

    /// Wait for events with an optional timeout.
    ///
    /// Returns the set of events that fired since the last `poll` call.
    /// Returns an empty `Vec` if the timeout elapsed before any event
    /// arrived. Returns an error only if the underlying syscall fails for a
    /// reason other than EINTR (which is retried internally).
    ///
    /// `timeout = None` blocks until at least one event arrives.
    fn poll(&mut self, timeout: Option<Duration>) -> io::Result<Vec<Event>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interest_flags() {
        assert!(Interest::READABLE.is_readable());
        assert!(!Interest::READABLE.is_writable());
        assert!(Interest::WRITABLE.is_writable());
        assert!(!Interest::WRITABLE.is_readable());
        assert!(Interest::READABLE_AND_WRITABLE.is_readable());
        assert!(Interest::READABLE_AND_WRITABLE.is_writable());
    }
}
