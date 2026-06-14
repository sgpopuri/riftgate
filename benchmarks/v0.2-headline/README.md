# v0.2 headline benchmarks

This directory holds the reproducible harness behind every load-bearing
number Riftgate publishes for the **v0.2** milestone. Per AGENTS.md §5 we
ship numbers only when an operator can reproduce them from this repo.

## What ships in v0.2

The four subsystems that received concrete implementations in v0.2:

| Subsystem | Bench target | Cargo command |
|-----------|--------------|---------------|
| Scheduler (`PerShardScheduler` over `ShardedMpmcQueue`) | `crates/riftgate/benches/scheduler.rs` (landed in `v0.3`) | `cargo bench -p riftgate scheduler` |
| Rate limiter (`TokenBucketLimiter`, 64 dashmap shards) | `crates/riftgate-core/benches/rate_limit.rs` (landed in `v0.3`) | `cargo bench -p riftgate-core rate_limit` |
| Router (`WeightedRandomRouter` + `CircuitBreakerArbiter`) | `crates/riftgate-router/benches/routing.rs` (landed in `v0.3`) | `cargo bench -p riftgate-router routing` |
| WAL (`FileWal` append + group-commit fdatasync) | `crates/riftgate-replay/benches/wal.rs` (landed in `v0.3`) | `cargo bench -p riftgate-replay wal` |

The v0.2 milestone publishes **no headline P99 numbers**. The four
benches above landed in v0.3 as part of the perf-stabilization sweep; this
directory anchors the harness contract and the benchmark provenance for the
v0.2 systems-showpiece surfaces.

## Why no numbers yet

Per AGENTS.md §5 ("Honest numbers only"), a benchmark must:

1. Be reproducible from the repo with `cargo bench`.
2. Compare against a real baseline (LiteLLM, an existing Rust gateway, or
   a vendor-published claim with citation).
3. Carry no vendor-style number-fishing.

v0.2 shipped the implementations the bench targets depend on; v0.3 then
shipped the targets themselves and the comparison runs.

## See also

- [`docs/02-mvp-roadmap.md`](../../docs/02-mvp-roadmap.md) §"Currently shipping" — current milestone state.
- [`docs/02c-v0.2-retrospective.md`](../../docs/02c-v0.2-retrospective.md) — what shipped and what slipped.
