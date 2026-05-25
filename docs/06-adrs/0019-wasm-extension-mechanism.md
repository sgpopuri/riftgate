# ADR 0019. WASM extension mechanism via wasmtime with frozen `riftgate:filter/v1` component-model ABI

> **Date:** 2026-05-25
> **Status:** accepted
> **Options doc:** [016-extension-mechanism](../05-options/016-extension-mechanism.md)
> **Deciders:** Sriram Popuri

## Context

`v0.1` shipped the `Filter` trait in `riftgate-core` with two in-tree impls (`IdentityFilter`, `LoggingFilter`); the in-code comment names a filter chain executor "lands in `riftgate-filter` in v0.3." `v0.3`'s Programmability pillar requires a real extension surface for PII redaction, prompt-template substitution, output-schema validation, cost guards, and token-budget guards — without recompiling the gateway. Sandboxing is non-negotiable (`NFR-S03`); five candidates (none, Lua, JavaScript, native `.so`, WASM) were evaluated in [Options `016`](../05-options/016-extension-mechanism.md). Envoy's documented Lua→WASM migration and the maturation of WASI Preview 2 / the WebAssembly Component Model in 2024 make WASM the de-facto industry answer for this exact problem.

## Decision

**`v0.3` ships a WASM filter chain in a new crate `crates/riftgate-filter` backed by `wasmtime` with the component-model ABI frozen at `riftgate:filter/v1`; native in-tree `Filter` impls remain a supported first-class path; Lua, JavaScript, and native dynamic plugins are rejected.**

- Filter authors compile to WebAssembly components via `cargo component build` (Rust first-class; Go, JS-via-`jco`, Python-via-`componentize-py` supported but documented as second-tier).
- Host functions exposed are exactly: `log`, `now-millis`, `emit-counter`. No filesystem, no network, no environment, no random, no process. Future scope requires a new ABI version and a new ADR.
- Per-filter resource limits: `fuel` (default 5M instructions), `memory` (default 16 MiB linear), `wallclock` (default 50ms, enforced via the existing binary-heap timer subsystem per [ADR `0010`](0010-binary-heap-timers-v01-hierarchical-wheel-v02.md)).
- `wasmtime::PoolingAllocationConfig` is used to pre-allocate one instance pool per filter; hot path is `Instance::call_typed`, never `Instance::new`.
- AOT precompile via `Engine::precompile_component` at config-load; reload events swap the live chain atomically when the new component is ready.
- Starter filter library ships under `examples/02-starter-filters/`, *not* as workspace crates (PII redactor, prompt template, schema validator, cost guard, token-budget guard).

## Consequences

- **Positive:**
  - Mechanical (not conventional) sandboxing: `NFR-S03` met by construction; a filter cannot reach the filesystem, network, or syscalls.
  - Multi-language filter authoring (Rust, Go, JS, Python, C) via one frozen WIT ABI.
  - Declarative per-filter resource limits (fuel, memory, wallclock) — bounding misbehaving filters without gateway-author defensive code.
  - Single signed `.wasm` artifact composes with operator-side supply-chain tooling (cosign, sigstore, SLSA).
  - The `Filter` trait in `riftgate-core` is unchanged; `IdentityFilter` and `LoggingFilter` continue to work; `WasmFilter` implements the same trait by delegating to a `wasmtime::Component`.
- **Negative / accepted tradeoffs:**
  - Cold-instantiation cost (millisecond range) requires instance pooling on the hot path. Pooling adds runtime memory; documented and bounded by config.
  - Linear-memory boundary means host-component data crosses with a copy; we accept the per-filter copy cost and measure it in benches.
  - Component-model toolchains for some non-Rust languages are still rough; we document the supported languages and tell others to wait.
  - Operators must learn one new artifact format (`.wasm` / `.cwasm`); we ship recipes and `examples/02-starter-filters/` to flatten the learning curve.
- **Future work this enables:**
  - WASM-graded eval tasks (per [Options `019`](../05-options/019-replay-eval.md) §6) reuse the same WIT shape with different host functions.
  - Future capability grants (per-filter network, per-filter `kv-store`) become explicit ABI extensions at `riftgate:filter/v2`.
  - WASM-pluggable routing remains *closed* in v0.3 (routing stays as in-tree `Router` impls per [Options `025`](../05-options/025-v03-routing-strategies.md)); revisit only if a credible benchmark shows acceptable dispatch cost.
- **Future work this forecloses (until superseded):**
  - Riftgate will not ship Lua or JavaScript filter mechanisms.
  - Riftgate will not ship native `.so` / `.dylib` plugin loading.
  - Riftgate will not embed V8 or any C++ runtime in the data plane.
  - Riftgate will not ship an `ext-proc`-style sidecar as the default filter path in v0.3.

## Compliance

- The `riftgate-filter` crate's `wit/riftgate-filter.wit` is the contract. Any change to the WIT requires a new package version and a paired ADR.
- `cargo build --workspace --all-features` succeeds on macOS and Linux (wasmtime is cross-platform; no Linux-only target gating).
- Integration test `crates/riftgate-filter/tests/sandbox_isolation.rs` asserts that a hostile filter cannot (a) loop without fuel exhaustion, (b) allocate without memory exhaustion, (c) escape to the host filesystem.
- A criterion bench at `crates/riftgate-filter/benches/dispatch.rs` measures per-`on_request` dispatch cost; CI fails if the starter filter library exceeds 50µs/filter at p99 on the reference hardware (per `NFR-P09`).
- Adding a new starter filter (under `examples/02-starter-filters/`) does **not** require a new ADR. Adding a new host function or breaking the WIT package version does.

## Notes

- WASM dispatch is genuinely fast in 2026 — Envoy proxy-wasm production deployments and wasmtime benchmark publications place AOT-compiled dispatch in the low microseconds. Riftgate inherits this; the budget concern is real only if we forget to pool instances.
- The decision to ship starter filters as `examples/` rather than as published crates is deliberate: it discourages "the gateway team owns every PII rule forever" and encourages forking, which is what we want.
- The MCP capability broker (v0.5, per [ADR `0015`](0015-mcp-extension-plane-broker.md)) sits in the same extension plane and *consumes* the same filter chain; the WIT ABI for filter and the trait for `CapabilityBroker` are intentionally separate (filter is byte-level; broker is capability-level).
- We deliberately avoid `proxy-wasm`'s ABI: that spec was designed for Envoy's stream-of-buffers HTTP filter model and predates the component model. Riftgate's ABI is component-model-native from the start.
