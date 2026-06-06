# riftgate-filter

v0.3 filter chain executor + scaffold for the frozen `riftgate:filter/v1`
WebAssembly Component Model ABI.

Per [ADR 0019](../../docs/06-adrs/0019-wasm-extension-mechanism.md) and
[Options 016](../../docs/05-options/016-extension-mechanism.md).

## Implementation status (pass 1: scaffold)

- `FilterChain` — production. In-order on the request side, reverse-order
  on the response side. Implements `riftgate_core::Filter`, so a chain
  composes recursively under any other `Filter`-bearing call site.
- `WasmFilter` — scaffold. Public type surface lands today;
  `WasmFilter::scaffold()` returns an instance that behaves as the identity
  filter. `WasmFilter::try_load()` returns
  `WasmFilterError::BackendNotWired` so the "not yet implemented" path is
  observable.

The production wasmtime backend (AOT precompile, instance pooling, host
functions `log` / `now-millis` / `emit-counter`, fuel / memory / wallclock
limits) lands in a follow-on implementation PR within the combined `v0.3 +
v0.4` implementation phase. The substitution is transparent to callers —
the public type surface does not change.
