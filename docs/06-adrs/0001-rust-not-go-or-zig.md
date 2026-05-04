# ADR 0001. Rust, not Go or Zig, for the Riftgate kernel

> **Date:** 2026-05-02
> **Status:** accepted
> **Options doc:** n/a (foundational; no Options doc, recorded directly)
> **Deciders:** Sriram Popuri

## Context

Riftgate is a high-concurrency, low-latency network data plane that needs to coexist as a sidecar in service-mesh deployments and as a single binary in self-hosted deployments. The implementation language is the most foundational decision the project will ever make. Alternatives considered:

- **Rust** — memory-safe, no GC, mature async ecosystem (Tokio, axum, hyper, wasmtime), excellent FFI for eBPF (Aya) and WASM, growing OSS infra mindshare.
- **Go** — excellent ergonomics for network services, GC pauses are real but typically sub-millisecond, the ecosystem of LLM gateways and proxies is heavily Go-coded today (`llm-d-kv-cache`, parts of Envoy AI Gateway control plane). Faster to v1.0 if the team is Go-fluent.
- **Zig** — manual memory management with stronger ergonomics than C; very small standard library; immature async story; small community.
- **C++** — Envoy precedent; mature; but the safety story is exactly what Rust replaces, and the build/distribution story is harder.

## Decision

**Rust** is the implementation language for the entire Riftgate kernel and all maintained subsystems. C is permitted only for narrow eBPF programs loaded via Aya. Python and shell are permitted for tests, benchmarks, and tooling — never in the data path.

## Consequences

- **Positive:**
  - Memory safety eliminates a class of bugs that would haunt a C/C++ implementation. Production deployments do not have to budget for use-after-free debugging.
  - No GC pauses. Tail latency is predictable in a way Go cannot match for high-percentile workloads.
  - The Tokio ecosystem (and the alternative `glommio`/`monoio` ecosystems) gives us multiple async runtimes to choose between under a single language.
  - First-class WASM (wasmtime) and eBPF (Aya) integration let us cleanly build the extension and observability planes without polyglot complexity.
  - Aligns with the project's Distinguished-Engineer-grade brand: "I take systems seriously" reads differently in Rust than in Go.
- **Negative / accepted tradeoffs:**
  - Slower velocity than Go for a solo maintainer in the first six months. We accept this; pluggability + documentation are the moats, not feature velocity.
  - Smaller pool of contributors who are immediately productive on the codebase. Mitigated by clear trait surface, docs, and AGENTS.md.
  - Some Rust async ecosystem decisions (which executor, which I/O backend) are themselves Options-doc-worthy. We do that work explicitly in [Options 002](../05-options/002-async-runtime.md).
- **Future work this enables:**
  - Aya for the eBPF observability plane in `v0.4`.
  - Native interop with `tokio-uring` for the io_uring `AsyncIO` impl in `v0.2`.
  - WASM filter chain in `v0.3` via `wasmtime`.
- **Future work this forecloses (until superseded):**
  - We will not have a Go-native plugin model. WASM is our extension story.
  - We will not adopt `cgo`-based C-library integrations as a default; if we link C, it is via FFI in a small dedicated crate.

## Compliance

- `Cargo.toml` at the workspace root pins MSRV (Minimum Supported Rust Version).
- CI runs `cargo build`, `cargo test`, `cargo clippy --deny warnings`, `cargo fmt --check` on every PR.
- Any non-Rust file (other than eBPF C, test fixtures, and the listed exceptions above) requires reviewer signoff with a one-sentence justification.

## Notes

- Some serious Rust LLM-gateway competitors already exist: TensorZero, Helicone AI Gateway, LangDB, Traceloop Hub. We are not the first; we are not trying to be the fastest. See [`docs/00-vision.md`](../00-vision.md) for the differentiation argument.
- Zig was a serious consideration. Rejected primarily on async ecosystem maturity; revisit at the next inflection point if Zig 1.0 ships with a usable async story.
- The Go alternative — note that the team-of-one factor matters here. A larger team optimizing for time-to-market would have a stronger Go case. We are a one-person OSS project optimizing for technical depth and long-term defensibility.
