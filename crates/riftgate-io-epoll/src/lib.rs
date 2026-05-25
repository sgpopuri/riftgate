//! # riftgate-io-epoll
//!
//! `AsyncIO` impl backed by [`mio`].
//!
//! On Linux this resolves to `epoll(7)`; on macOS and BSD it resolves to
//! `kqueue(2)`. The single [`MioIO`] struct wraps both via mio's
//! cross-platform abstraction; the platform-conditional aliases
//! `EpollIO` and `KqueueIO` let callers express the platform
//! commitment explicitly when desired.
//!
//! Per [ADR 0002](../../../docs/06-adrs/0002-start-on-epoll.md) this is the
//! v0.1 IO substrate; `riftgate-io-uring` will land in v0.2 as a peer
//! impl behind the same `AsyncIO` trait.
//!
//! ```text
//!   shard worker thread
//!         |
//!         v
//!   MioIO { poll: mio::Poll, events_buf: mio::Events, registered: HashMap }
//!         |
//!         v
//!   register(fd, token, interest)  -->  Registry::register(SourceFd(&fd), Token, mio::Interest)
//!   poll(timeout)                  -->  Poll::poll(&mut events, timeout)  -->  Vec<Event>
//!   deregister(fd)                 -->  Registry::deregister(SourceFd(&fd))
//! ```

#![doc(html_root_url = "https://docs.rs/riftgate-io-epoll/0.1.0-dev")]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

mod mio_io;

pub use mio_io::MioIO;

/// Linux alias for [`MioIO`]. The underlying `mio::Poll` uses `epoll(7)` on
/// Linux.
#[cfg(target_os = "linux")]
pub use mio_io::MioIO as EpollIO;

/// macOS / BSD alias for [`MioIO`]. The underlying `mio::Poll` uses
/// `kqueue(2)` on these targets.
#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd",
    target_os = "dragonfly",
))]
pub use mio_io::MioIO as KqueueIO;
