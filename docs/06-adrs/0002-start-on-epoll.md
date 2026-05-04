# ADR 0002. Start on epoll; add io_uring behind a feature flag in v0.2

> **Date:** 2026-05-02
> **Status:** accepted
> **Options doc:** [001-io-model](../05-options/001-io-model.md)
> **Deciders:** Sriram Popuri

## Context

Riftgate needs an IO-multiplexing mechanism for its data plane. The full exploration of candidates (epoll, kqueue, io_uring, DPDK, AF_XDP) and the tradeoff matrix live in [Options 001](../05-options/001-io-model.md). The decision is recorded here.

The forces summarized: we target Linux as Tier-1, want to ship something operationally honest, want to keep eBPF observability paths open ([Options 014](../05-options/014-ebpf-integration.md)), and explicitly accept that we will not compete with TensorZero on raw P99 throughput in `v0.1`.

## Decision

**`v0.1` ships with epoll as the only `AsyncIO` impl on Linux.** kqueue ships under `cfg(target_os = "macos")` for developer convenience.

**`v0.2` adds `io_uring` as an opt-in via a `cargo` feature flag (`--features io-uring`) and a runtime config setting (`io_model = "io_uring"`).** It is not the default. Users opting in accept the security and maturity tradeoffs documented in the Options doc.

DPDK and AF_XDP are explicitly out of scope.

## Consequences

- **Positive:**
  - Riftgate is operationally legible from day one — `strace`, `bpftrace`, `perf`, `ss`, and every other Linux network-debugging tool just works.
  - We avoid the io_uring security exposure (Project Zero data) by default. Users who need the perf can opt in deliberately.
  - The `AsyncIO` trait surface accommodates both readiness-based and completion-based backends without leaking the difference into upstream code.
  - Mature, well-understood backend means our scarce engineering attention can go to the differentiation pillars (pluggability, documentation, eBPF), not into reinventing IO.
- **Negative / accepted tradeoffs:**
  - We will be slower than an io_uring-default competitor at very high QPS. We are honest about this in the README and the Vision doc.
  - Maintaining two IO impls in `v0.2+` is real engineering cost. We accept it because both backends are core to the project's pluggability story.
  - Hot code paths must be careful not to bake epoll-isms into the trait. The conformance test suite ([Options 001 §3.1](../05-options/001-io-model.md)) is the guard.
- **Future work this enables:**
  - `tokio-uring`-style fast paths in `v0.2`.
  - Multishot-accept and registered-buffer optimization explorations.
  - Honest A/B benchmarks (epoll vs io_uring on the same workload, same machine, reproducible from `benchmarks/`).
- **Future work this forecloses (until superseded):**
  - We will not investigate DPDK or AF_XDP as Riftgate backends.
  - We will not auto-detect "best" backend at runtime; users opt in deliberately.

## Compliance

- `crates/riftgate-io-epoll/` is the default `AsyncIO` impl on Linux.
- `crates/riftgate-io-uring/` is a separate crate gated by a `cargo` feature; it does not affect default builds.
- The `AsyncIO` conformance test suite (`crates/riftgate-io-epoll/tests/conformance.rs`) runs against every impl in CI.
- Adding a new `AsyncIO` impl requires a new ADR superseding (or amending) this one, plus passing the conformance suite.

## Notes

- The decision to treat io_uring as opt-in rather than default may look conservative. It is. The Vision doc commits us to honest scoping; the security data on io_uring is real; users who need the perf are sophisticated enough to opt in.
- We will revisit if (a) the io_uring CVE rate drops materially in observable Google Project Zero data, or (b) Linux distros / container runtimes start enabling io_uring by default for unprivileged workloads. Either signal would change the calculus.
- Conformance against macOS kqueue is dev-only. We do not promise production support on macOS.
