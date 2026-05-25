//! # riftgate-replay
//!
//! v0.2 append-only request log. Implements
//! [`riftgate_core::wal::WAL`](../../riftgate-core/src/wal.rs).
//!
//! Per [Options `009`](../../../docs/05-options/009-request-log.md) and
//! [ADR `0013`](../../../docs/06-adrs/0013-append-only-file-wal.md):
//!
//! - **Per-shard segment files** under a configured root dir, named
//!   `seg-{shard:04}-{seqno:020}.wal`.
//! - **Length-prefixed framing** per entry:
//!   `[u32 length][u8 durability_tag][u8 entry_kind][u64 timestamp_nanos]
//!   [u64 entry_id][payload bytes]`.
//! - **Group-commit `fdatasync`** flusher per shard. Cadence: every
//!   `flush_interval` or every `flush_buffer_bytes`, whichever is first.
//! - **Mixed durability per entry.** `Async` entries return immediately;
//!   `FdataSync` and `Fsync` park the caller until the next flush completes
//!   and reports success.
//!
//! This v0.2 implementation prioritizes correctness and clarity over peak
//! throughput. Operators who care about microbenchmark numbers run the
//! `benchmarks/v0.2-headline/` harness; everyone else gets a WAL whose
//! crash semantics match the documentation.

#![doc(html_root_url = "https://docs.rs/riftgate-replay/0.1.0-dev")]
#![deny(unsafe_code)]
#![warn(missing_docs)]
#![warn(rustdoc::broken_intra_doc_links)]

mod file_wal;

pub use file_wal::{ENTRY_HEADER_BYTES, FileWal, FileWalConfig};
