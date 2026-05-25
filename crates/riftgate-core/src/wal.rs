//! `WAL` trait — defined in `v0.1`, default impl deferred to `v0.2`.
//!
//! The append-only request log lands in `crates/riftgate-replay` in `v0.2`
//! per the roadmap. The trait shape is locked in here so callers (the
//! future audit-event publisher, the v0.5 MCP capability broker) compile
//! against it now.
//!
//! See [`docs/04-design/lld-storage.md`](../../../docs/04-design/lld-storage.md).
//!
//! **Why no v0.1 impl?** [`FR-105`](../../../docs/01-requirements/functional.md)
//! targets the WAL at `v0.2`; [`NFR-OBS06`](../../../docs/01-requirements/non-functional.md)
//! gates the replayable log on v0.2 as well. Shipping a `NoopWal` in v0.1
//! would mask the absence; the trait + `Option<Arc<dyn WAL>>` shape is the
//! right `v0.1` posture.

use std::io;
use std::time::Duration;

/// Identifier for an entry in the WAL.
///
/// Returned by [`WAL::append`]; passed to recovery/replay code.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Ord, PartialOrd)]
pub struct WalEntryId(pub u64);

/// Durability level for an append.
///
/// Per the LLD: callers choose the durability they need. Most request-log
/// entries use `Async`; audit entries use `Fsync`.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Durability {
    /// Append in memory; flush to disk in the background. Lowest latency.
    Async,
    /// Append and `fsync` before returning. Highest durability.
    Fsync,
    /// Append and `fdatasync` before returning. Trade-off between Async
    /// and Fsync; metadata may not be persisted.
    FdataSync,
}

/// Append-only request log.
///
/// **Trait shape only in `v0.1`.** The `v0.2` `FileWal` impl lives in
/// `crates/riftgate-replay` along with the `riftgate-replay` CLI.
///
/// **`Send + Sync`** — one WAL instance per process, shared by all shards
/// via `Arc`.
///
/// Trait object safety: yes.
pub trait WAL: Send + Sync {
    /// Append `bytes` to the log with the requested `durability`.
    ///
    /// Returns the new entry id on success.
    ///
    /// # Errors
    /// Returns the underlying IO error if the write or sync fails.
    fn append(&self, bytes: &[u8], durability: Durability) -> io::Result<WalEntryId>;

    /// Flush any buffered writes. Useful for graceful shutdown.
    ///
    /// # Errors
    /// Returns the underlying IO error if the flush fails.
    fn flush(&self, timeout: Duration) -> io::Result<()>;

    /// Most recently durably-written entry id, or `None` if the WAL is
    /// empty.
    fn last_durable(&self) -> Option<WalEntryId>;
}
