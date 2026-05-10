# ADR 0003. Tokio multi-threaded runtime as the only v0.1 runtime; per-core runtimes revisited at v0.2 retro

> **Date:** 2026-05-03
> **Status:** accepted
> **Options doc:** [002-async-runtime](../05-options/002-async-runtime.md)
> **Deciders:** Sriram Popuri

## Context

Riftgate needs a Rust async runtime to drive its `AsyncIO` and `Scheduler` traits. Full exploration of candidates (Tokio multi-thread, Tokio current-thread sharded, glommio, monoio, custom reactor) and the tradeoff matrix live in [Options 002](../05-options/002-async-runtime.md). The decision is recorded here.

The forces summarized: `v0.1` ships epoll only ([ADR 0002](0002-start-on-epoll.md)) which immediately rules out io_uring-only runtimes; the Rust async ecosystem (`hyper`, `tonic`, `tower`, `tracing`, `wasmtime`'s async API, OpenTelemetry) overwhelmingly assumes a Tokio reactor; and the `Scheduler` decision in [Options 003](../05-options/003-concurrency-model.md) is what governs per-core vs work-stealing, not the runtime decision.

## Decision

**`v0.1` ships with the Tokio multi-threaded runtime as the only embedded runtime in the `riftgate` binary.**

The discipline:

- `riftgate-core`'s public trait surface (`AsyncIO`, `Scheduler`, `Queue`, `Allocator`, `TimerSubsystem`, `WAL`, `Filter`, `Router`) **must not** expose Tokio types. Tokio types may appear inside impl crates and inside non-public modules of `riftgate-core`.
- `tokio` is a `pub use` only inside the binary crate (`riftgate`), not re-exported from any library crate.
- A future thread-per-core runtime (monoio is the current front-runner) becomes a candidate at the `v0.2` retro, behind a `--features per-core-runtime` cargo feature. It is not on the `v0.1` or `v0.2` deliverable list.

## Consequences

- **Positive:**
  - Riftgate inherits the entire Tokio ecosystem (`hyper`, `tonic`, `tower`, `tracing`, `metrics`, OpenTelemetry's Rust SDK, `wasmtime`'s async API) without writing adapter layers.
  - `tokio-console` and `tokio-metrics` give us best-in-class runtime introspection from day one.
  - Tokio's 1.x semver discipline means the runtime is not a moving target underneath us.
  - The work-stealing scheduler is a sane default for our heterogeneous-cost workload (parser → router → upstream call → response framing); see the work-stealing literature (Blumofe–Leiserson Cilk-5; Tokio scheduler design notes) cited in [Options `002`](../05-options/002-async-runtime.md).
  - macOS dev convenience ([NFR-PT03](../01-requirements/non-functional.md)) works out of the box via mio's kqueue backend.
- **Negative / accepted tradeoffs:**
  - We accept the small but non-zero overhead of Tokio's multi-threaded scheduler (global injector queue, cross-worker steal coordination) in exchange for the ecosystem and tooling wins.
  - We accept the discipline cost of keeping Tokio types out of `riftgate-core`'s public API. This is a code-review responsibility, not a compiler-enforced one.
  - When io_uring lands in `v0.2`, we will use `tokio-uring` rather than swapping the entire runtime; this leaves some peak-throughput potential of native io_uring runtimes (glommio, monoio) on the table.
- **Future work this enables:**
  - `tokio-uring` integration in `v0.2` is straightforward.
  - Adding a per-core `Scheduler` impl that wraps multiple Tokio current-thread runtimes is possible without changing the runtime decision.
  - All Tokio-native crates we adopt later (`tower-http`, `hyper-util`, etc.) integrate with no additional runtime work.
- **Future work this forecloses (until superseded):**
  - We will not author or vendor a custom reactor in `v0.x`.
  - We will not adopt `async-std`, `smol`, or any other niche runtime as the primary executor.
  - We will not ship glommio or monoio as the default runtime in `v0.2` without a new ADR superseding this one.

## Compliance

- The `riftgate` binary uses `#[tokio::main(flavor = "multi_thread")]`. Worker thread count is configurable via `RIFTGATE_WORKER_THREADS`; default is the CPU count.
- A clippy lint (`disallowed-types`) prohibits direct re-export of Tokio types from any `riftgate-core` public module. The lint configuration lives in `clippy.toml`.
- A code-review checklist item: "no Tokio types in `riftgate-core` `pub` API." PRs that violate this are rejected.
- The `AsyncIO` conformance test suite in `crates/riftgate-io-epoll/tests/conformance.rs` (per [ADR 0002](0002-start-on-epoll.md) compliance) does not assume Tokio; impls are tested via the trait, not the runtime.
- Adding a non-Tokio runtime requires a new ADR superseding this one and a demonstration that the trait-surface discipline holds across both runtimes.

## Notes

- The decision to keep the runtime as a single, conventional choice in `v0.1` is in the spirit of [Vision §4](../00-vision.md): we are honest about not competing on raw P99 throughput in `v0.x`, so we do not need to manufacture differentiation through runtime choice.
- The `v0.2` retro is the right place to revisit. By then we have io_uring in tree, we have measured Tokio's tail behavior on our own workload, and we know whether thread-per-core's predictable-tail story is something we want to invest in.
- Pingora (Cloudflare) is the cautionary tale on the other side: they did move to a custom runtime and the engineering cost was substantial, justified only by hyperscaler-edge volumes that Riftgate is explicitly not chasing.
