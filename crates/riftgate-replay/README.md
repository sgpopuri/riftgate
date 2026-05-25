# riftgate-replay

Append-only request log for Riftgate. v0.2 ships `FileWal`: per-shard segment files, length-prefixed framed entries, group-commit `fdatasync` flusher, mixed `Async` / `FdataSync` / `Fsync` durability per entry.

Implements `riftgate_core::wal::WAL`. See [`docs/04-design/lld-storage.md`](../../docs/04-design/lld-storage.md), [Options `009`](../../docs/05-options/009-request-log.md), and [ADR `0013`](../../docs/06-adrs/0013-append-only-file-wal.md).
