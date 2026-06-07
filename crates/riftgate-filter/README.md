# riftgate-filter

v0.3 filter chain executor + WASM runtime for the frozen
`riftgate:filter/v1` WebAssembly Component Model ABI.

Per [ADR 0019](../../docs/06-adrs/0019-wasm-extension-mechanism.md) and
[Options 016](../../docs/05-options/016-extension-mechanism.md).

## Implementation status

- `FilterChain` — production. In-order on the request side, reverse-order
  on the response side. Implements `riftgate_core::Filter`, so a chain
  composes recursively under any other `Filter`-bearing call site.
- `WasmFilter` — production runtime behind the `wasm` feature. It loads a
  component at `WasmFilterConfig.component_path`, wires host functions,
  and maps component actions to `FilterAction`. `WasmFilter::scaffold()`
  remains for explicit identity behavior.

The runtime validates AOT precompile eligibility at load and configures
pooling allocation. Further optimizations (instance reuse strategy,
wallclock interruption, richer action variants) can iterate without
changing the public `WasmFilter` surface.
