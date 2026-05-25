# 04.k LLD — Filter chain

> Request- and response-side hooks: PII redaction, prompt template substitution, output schema validation, cost guards, token-budget guards. Native Rust filters and sandboxed WASM filters behind the same trait.
>
> Status: **shipped (v0.1, trait + `IdentityFilter` + `LoggingFilter`); v0.3 adds the chain executor, the `WasmFilter` impl, and the starter filter library.** WASM extension mechanism decision in [Options `016`](../05-options/016-extension-mechanism.md) and [ADR `0019`](../06-adrs/0019-wasm-extension-mechanism.md).

## Purpose

Apply user-provided policy to each request before it reaches the router and to each response before it returns to the client. The filter chain is the v0.3 expression of Riftgate's first differentiation pillar: programmable Rust core + WASM extensions.

A filter can: read request or response bytes, mutate headers or body, terminate the request with a structured status, or emit telemetry. A filter cannot: open sockets, touch the filesystem, call syscalls, escape its linear memory, or run beyond its declared fuel/wallclock budget.

## Trait surface

The shipped trait — see [`crates/riftgate-core/src/filter.rs`](../../crates/riftgate-core/src/filter.rs) — is unchanged from v0.1:

```rust
pub trait Filter: Send + Sync {
    fn on_request(&self, req: &mut Request) -> FilterAction;
    fn on_response(&self, _resp: &mut Response) -> FilterAction {
        FilterAction::Continue
    }
}

pub enum FilterAction {
    Continue,
    Terminate(StatusCode),
}
```

v0.3 adds nothing to the trait. The new artifacts are:

- A **chain executor** (`FilterChain`) in `crates/riftgate-filter` that drives an ordered list of `Box<dyn Filter>` in forward order on the request side and reverse order on the response side. Termination short-circuits the chain.
- A **`WasmFilter`** impl in `crates/riftgate-filter` that implements `Filter` by delegating to a `wasmtime::Component` instantiated against the `riftgate:filter/v1` WIT ABI.

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `IdentityFilter` | shipped (v0.1) | `riftgate-core` | Zero-cost pass-through; the v0.1 default for routes with no policy. |
| `LoggingFilter` | shipped (v0.1) | `riftgate-core` | Tracing-level filter; emits a `Filter::Continue` span for each request/response. |
| `FilterChain` | **v0.3** | `riftgate-filter` | Ordered executor; request-side forward, response-side reverse; short-circuits on `Terminate`. Per [ADR `0019`](../06-adrs/0019-wasm-extension-mechanism.md). |
| `WasmFilter` | **v0.3** | `riftgate-filter` | Hosts a `wasmtime::Component` against the `riftgate:filter/v1` WIT ABI. Pooled instances; AOT-precompiled at config-load. |

Starter filters land under `examples/02-starter-filters/`, not as workspace crates:

| Starter filter | What it does |
|----------------|--------------|
| `pii-redactor` | Regex-driven PII removal on the request body (and optionally response body). |
| `prompt-template` | Substitutes named variables in a prompt template, defined per-route in config. |
| `schema-validator` | Validates the response body against a JSON schema; terminates on mismatch. |
| `cost-guard` | Estimates per-request token cost (input + projected output) and terminates if the route's per-tenant budget is exhausted. |
| `token-budget-guard` | Per-tenant rolling token-budget accounting against [`crates/riftgate-core::TokenBucketLimiter`](../../crates/riftgate-core/src/limiter.rs); request-side decrement, response-side adjustment based on actual token count. |

Decision rationale: [Options `016`](../05-options/016-extension-mechanism.md), [ADR `0019`](../06-adrs/0019-wasm-extension-mechanism.md). Cancellation contract for filter-side `Terminate`: [Options `024`](../05-options/024-stream-cancellation.md), [ADR `0020`](../06-adrs/0020-stream-cancellation-cancellation-token.md).

## Component context

### Architecture and dependencies

The filter chain is invoked twice per request: once on the request side before the router and once on the response side after the upstream stream completes (or per-token on streaming responses if the filter declares `on = ["response-stream"]` in a future ABI). The chain is an ordered `Vec<Box<dyn Filter>>` configured per route via TOML. Native filters and WASM filters coexist freely in the same chain.

Dependencies:
- `riftgate-core` for the `Filter` trait, `Request`, `Response`, `Cancellation`.
- `wasmtime` (with `component-model` and `pooling-allocator` features) for `WasmFilter`.
- `riftgate-obs::Publisher` for filter-action telemetry.
- The binary-heap timer subsystem (`crates/riftgate-core::BinaryHeapTimers`, per [ADR `0010`](../06-adrs/0010-binary-heap-timers-v01-hierarchical-wheel-v02.md)) for per-filter wallclock enforcement.

The chain executor does *not* hold locks during dispatch; each filter is `Send + Sync` by trait, and `WasmFilter` borrows a pooled `wasmtime::Instance` for the duration of one `on_request` or `on_response` call.

### Patterns and conventions

- **Filters are pure(-ish) functions of their inputs.** Side effects allowed: emit a counter, log, mutate the request/response.
- **Request-side forward, response-side reverse.** A filter that adds a header on request removes it on response; the order ensures balanced state.
- **Termination short-circuits.** A filter that returns `Terminate(status)` skips all later filters in the chain and triggers the cancellation primitive ([Options `024`](../05-options/024-stream-cancellation.md)) on any in-flight upstream attempt.
- **Resource limits per filter, not per chain.** Each WASM filter has its own fuel and memory pool. A misbehaving filter exhausts its own budget; the chain continues with the next filter or terminates the request based on policy.
- **The WIT ABI is frozen.** `riftgate:filter/v1` is the contract; changes require a new ABI version (`v2`) and a new ADR. Filter authors target a specific ABI version in their `Cargo.toml`.

### Pitfalls

- **Cooperative cancellation in WASM.** A filter that spins inside its body never observes a `Cancellation` flip. Mitigated by wasmtime's fuel and wallclock limits — a spinning filter is killed by the runtime, not by the gateway author writing defensive code.
- **Instance-pool exhaustion.** If every filter is busy and a new request arrives, the chain waits for a free instance. Pool size defaults to `num_shards * 2`; operators raise it for high-fanout deployments. The pool size is observable as a gauge.
- **Filter ordering matters and is operator-controlled.** Putting a `cost-guard` after a `prompt-template` means the template substitution has already run before the cost check; in most cases this is what operators want (cost is computed on the substituted template), but it is operator-visible. Document.
- **WASM cold-instantiation cost.** `Instance::new` is in the millisecond range; never call it on the hot path. Pooling is mandatory.
- **AOT cache invalidation on filter reload.** Reloading a filter with a new hash triggers fresh AOT precompile. The live chain swaps atomically when the new component is ready; the old chain drains in-flight requests. Live-traffic interruption is zero.
- **Host-function expansion is a trust expansion.** Adding a new host function to the WIT ABI is a capability grant to every filter that targets the new ABI version. Treat as a security-review event.

### Standards and review gates

- New native `Filter` impls require unit tests against synthetic request/response fixtures.
- New `WasmFilter` integrations require: (a) sandbox-isolation tests (a hostile filter cannot escape), (b) resource-limit tests (fuel/memory/wallclock are enforced), (c) a dispatch microbenchmark inside the `NFR-P09` 50µs/filter budget.
- Adding a new host function to the WIT ABI requires a new ABI version, a new ADR superseding `0019`, and CODEOWNERS approval on `crates/riftgate-filter/wit/`.
- Starter filters under `examples/02-starter-filters/` must build with `cargo component build --release` in CI; broken examples are a regression.

## Testing strategy

- Unit tests on each native filter: fixture-driven request/response, assert `FilterAction`.
- Sandbox-isolation tests on `WasmFilter`: a hostile WAT/WIT module that tries to (a) loop forever, (b) allocate without bound, (c) import a host function that is not granted — all three are denied by the runtime, not the gateway.
- Chain-order tests: a chain `[A, B, C]` must invoke request-side in `A → B → C` and response-side in `C → B → A`; termination at B must skip C and stop A's response side from running.
- Hot-path microbenchmarks (criterion): `cargo bench -p riftgate-filter` measures per-filter dispatch at the 50µs/filter budget.
- Reload tests: live traffic continues during a filter-chain reload; no request observes a partial chain.
- End-to-end tests with the starter filter library: a request matched against `pii-redactor` arrives at the upstream with PII removed; a response matched against `schema-validator` is rejected on schema mismatch.

## Open questions

- **Streaming response filters.** v0.3's WIT ABI is request/response only (full body in, full body out). Streaming responses require an `on_response_chunk` host-callable; this is a `v2` ABI extension. Open: do we ship `v2` in v0.4 or later?
- **Per-tenant filter chains.** v0.3's chain is per-route, not per-tenant. Per-tenant chains intersect with the [multitenancy Options `017`](../05-options/README.md); deferred.
- **Filter-emitted custom metrics.** `emit-counter` is the only metric host function in v0.3. Histograms and gauges are obvious follow-ups; we defer until a starter filter requests them.
- **Filter chains for the MCP capability broker** (v0.5). The broker is a different trait surface ([Options `026`](../05-options/026-mcp-orchestration.md)); the filter chain remains the byte-level surface. The two coexist; v0.5 designs the integration.

## Data structures worth citing

The filter subsystem is a meeting point for a small handful of classical structures.

### Linear chains and ordered execution

Reference: the standard pattern (Chain-of-Responsibility, GoF), ordered filter pipelines (Apache Camel, Netty `ChannelPipeline`, Envoy filter chain).

Riftgate's `FilterChain` is a `Vec<Box<dyn Filter>>` driven in order on the request side and in reverse on the response side. The short-circuit semantics — `Terminate` stops the chain — are exactly the Chain-of-Responsibility contract. The order matters and is operator-controlled.

### AOT-compiled WASM components and instance pools

Reference: wasmtime's `Engine::precompile_component` and `PoolingAllocationConfig`.

The hot path is `Instance::call_typed` against a pre-instantiated `wasmtime::Instance`. The instance pool is constructed at config-load with `PoolingAllocationConfig`; each pool slot reserves a linear-memory region of `memory` size (per the filter's config), and the pool returns one slot per concurrent in-flight invocation of that filter. AOT compilation is a one-time cost at config-load; reloads re-precompile only the filters whose `.wasm` hash changed.

### Capability grants via WIT host imports

Reference: the WebAssembly Component Model's `world` and `import` semantics; capability-based-security lineage (KeyKOS / EROS / seL4 — see also [Options `026` §5](../05-options/026-mcp-orchestration.md)).

A filter's `world` declares which host functions it imports; the runtime grants exactly those and no more. There is no global namespace and no ambient authority. This is the same property the v0.5 MCP capability broker provides at a higher level; v0.3 filters provide it at the protocol level.
