//! # riftgate-io-uring
//!
//! Second concrete impl of [`riftgate_core::io::AsyncIO`], backed by Linux
//! `io_uring` via the [`io-uring`](https://crates.io/crates/io-uring) crate.
//!
//! Per [ADR `0002`](../../../docs/06-adrs/0002-start-on-epoll.md), the
//! default `AsyncIO` impl is epoll (in `riftgate-io-epoll`). This crate is
//! the opt-in `io_uring` backend, gated by the `io-uring` Cargo feature and
//! `cfg(target_os = "linux")`. On every other target — and on Linux when
//! the feature is off — the crate compiles to an empty library so the
//! workspace builds without `io_uring` headers in scope.
//!
//! The design contract lives in
//! [`docs/04-design/lld-io-runtime.md`](../../../docs/04-design/lld-io-runtime.md).

#![doc(html_root_url = "https://docs.rs/riftgate-io-uring/0.1.0-dev")]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

#[cfg(all(target_os = "linux", feature = "io-uring"))]
mod uring;

#[cfg(all(target_os = "linux", feature = "io-uring"))]
pub use uring::UringIO;

/// Build-time descriptor — useful for runtime introspection and bench
/// harnesses that want to know whether the io_uring backend is compiled
/// in.
pub const BACKEND_ENABLED: bool = cfg!(all(target_os = "linux", feature = "io-uring"));
