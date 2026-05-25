# 016. Extension mechanism

> **Status:** `recommended` — `v0.3` ships a WASM filter chain backed by `wasmtime` with a fixed component-model ABI; native-Rust `Filter` impls remain a supported first-class path. Lua, JavaScript, and dynamically-loaded native plugins are catalogued and rejected. See [ADR `0019`](../06-adrs/0019-wasm-extension-mechanism.md).
> **Foundational topics:** sandboxed extension surfaces (WASM via `wasmtime`), the WebAssembly component model and WASI Preview 2, sidecar / ambassador deployment patterns (Microsoft *Cloud Design Patterns*; Hohpe *EIP*), Envoy's WASM filter precedent, capability-based hosting (host functions as explicit grants), AOT compilation and code-signing of untrusted code
> **Related options:** [`010 — routing strategy`](010-routing-strategy.md) and [`025 — v0.3 routing strategies`](025-v03-routing-strategies.md) (routers consume the same extension surface), [`024 — stream cancellation`](024-stream-cancellation.md) (filters that abort mid-stream observe the cancellation contract), [`015 — config model`](015-config-model.md) (filter wiring lives in TOML)
> **Related ADR:** [ADR `0019`](../06-adrs/0019-wasm-extension-mechanism.md)

## 1. The decision in one sentence

> In which mechanism — none, Lua, JavaScript, native dynamic plugins, or WASM — does Riftgate `v0.3` express user-provided request and response filters, and what is the host ABI that protects the data plane from misbehaving extensions?

## 2. Context — what forces this decision

`v0.1` shipped the `Filter` trait in `riftgate-core` with two in-tree impls — `IdentityFilter` and `LoggingFilter` (see [`crates/riftgate-core/src/filter.rs`](../../crates/riftgate-core/src/filter.rs)). The trait shape — `on_request(&self, req: &mut Request) -> FilterAction` and an optional `on_response` — was deliberately sized to support a v0.3 filter chain executor without surgery. The in-code comment at the top of that module says so directly: *"filter chain executor (lands in `riftgate-filter` in v0.3) drives the chain in order on the request side and reverse order on the response side."*

`v0.3`'s Programmability pillar promises a real extension surface — PII redaction, prompt template substitution, output schema validation, cost guards, token-budget guards — without requiring a recompile of the gateway. This is the single largest piece of `v0.3`'s code surface area; getting the extension-mechanism choice wrong means rewriting every starter filter when we change ABIs. So we make this decision once and ship behind it.

Five forces frame the choice:

1. **The data plane cannot trust extension authors.** Persona P1 (Pia, platform engineer) writes filters; Persona P3 (Maya, learner) reads them; neither is a kernel author. A filter that loops forever, reads arbitrary memory, or shells out must not be able to take the gateway with it. The mechanism must enforce — not request — sandboxing.
2. **`v0.3` is the first milestone where pluggability is the headline.** Vision [§3.1](../00-vision.md) lists "programmable Rust core + WASM extensions" as differentiation pillar #1. We name the mechanism explicitly so reviewers, contributors, and prospective adopters know what they are buying.
3. **Envoy already ran this experiment, twice.** Envoy shipped Lua filters first (low ceremony, low isolation) and then WASM filters (proxy-WASM ABI, higher ceremony, real isolation). The migration was painful. We benefit from being late and choosing once.
4. **The component model and WASI Preview 2 stabilised in 2024.** The WebAssembly Component Model and WASI Preview 2 reached widespread runtime support (wasmtime, jco, wasmCloud) before this decision. We are not betting on a vapourware ABI; we are adopting a stabilised one.
5. **Riftgate is documentation-first.** The mechanism must be teachable. A filter author writing their first Riftgate filter should be able to read the LLD, write Rust that compiles to a component, drop the `.wasm` into the config, and run. Anything that requires `unsafe` or hand-rolled FFI to author a filter is too heavy.

The forces all point in the same direction: a sandboxed bytecode with a typed host ABI and a mature runtime. The Options doc still walks the rejected paths because rejecting them publicly is the discipline that turns this decision from "we picked WASM" into "we picked WASM for these reasons, and the alternatives die here."

The requirements this is load-bearing for:

- **`FR-201..205`** — the v0.3 programmability functional requirements ([`docs/01-requirements/functional.md`](../01-requirements/functional.md)).
- **`NFR-S03`** — extensions must not be able to read or write outside their declared sandbox.
- **`NFR-P09`** — filter dispatch on the hot path must add ≤ 50µs per filter at p99 for the starter filter library.
- **`NFR-OBS04`** — every filter decision (continue / terminate / mutate) must be observable as an OTel span attribute.

## 3. Candidates

We evaluate five candidates, ordered from least to most isolation.

### 3.1. None — in-tree native filters only

**What it is.** Operators want a new filter? They fork the repo, write a `Filter` impl, recompile the binary, and deploy. The `Filter` trait stays; no executor, no plugin discovery, no ABI.

**Why it's interesting.**
- **Zero new surface.** The trait already exists; the work is "do nothing new" and ship better in-tree filters.
- **Maximum performance.** No serialization, no sandbox cost, no foreign-function-interface dispatch. The compiler inlines.
- **Maximum power.** Filters are Rust. They can use any crate.
- **Honest framing.** "If you want extensibility, fork us; if you want speed, stay with us." Some projects (Caddy module system aside) succeed exactly this way.

**Where it falls short.**
- **Defeats the v0.3 pillar.** "Programmable AI data plane" cannot mean "fork the source." Vision [§3.1](../00-vision.md) becomes a lie.
- **No tenant isolation story.** A multi-tenant operator who wants per-tenant filters has nowhere to put them; recompile-per-tenant is operator-hostile.
- **Concentrates security risk on the maintainer.** Every accepted PR for a filter is a security review of foreign code that runs inside the data plane.
- **No operator workflow for sharing filters across deployments.** Each org reinvents the same PII redactor.

**Real-world systems that use it.** HAProxy historically (Lua came later); Cloudflare's earliest gateway internals before Workers; many embedded proxies.

### 3.2. Lua, embedded via `mlua` or `rlua`

**What it is.** A Lua interpreter is embedded in the gateway. Filters are Lua scripts loaded at startup or on config reload. The host exposes a typed API (`request.headers`, `request.body`, `terminate(status, body)`) via the `mlua` bindings.

**Why it's interesting.**
- **Low ceremony for filter authors.** Lua is small, well-documented, fast to learn.
- **Hot-reload friendly.** Reloading a Lua script is cheap; no compile step.
- **Mature embedding story.** Nginx, HAProxy, Envoy (v1), Redis all embed Lua; the patterns are well-understood.
- **Real-world precedent.** Cloudflare's original edge logic was Lua; OpenResty made the case.

**Where it falls short.**
- **Sandboxing is by convention, not by mechanism.** A Lua filter can call `os.execute` if the host forgot to remove it. Stripping the standard library to a safe subset is doable but is *the gateway author's burden*, not the language's. We pay the cost forever.
- **No ahead-of-time type checking.** Filter bugs surface in production. The starter-filter quality bar requires more.
- **Performance ceiling.** LuaJIT is fast for Lua but slower than native or WASM-AOT for the request-mutation hot path. The published Envoy comparisons of Lua-vs-WASM (Envoy blog, 2019) showed Lua's dispatch cost is significant per filter.
- **No declarative resource limits.** Bounding a Lua filter's CPU or memory requires running an interpreter step counter; this is also gateway-author burden.
- **Migration risk.** Envoy ran this exact experiment, found Lua's isolation insufficient, and migrated to WASM. We would knowingly walk into the same migration.

**Real-world systems that use it.** OpenResty / Nginx-Lua; HAProxy with `lua-load`; Envoy v1 Lua filter (now deprecated in favour of WASM); Redis scripting.

### 3.3. JavaScript via QuickJS or V8

**What it is.** A JS engine — QuickJS for footprint, V8 for performance — is embedded in the gateway. Filters are ES modules.

**Why it's interesting.**
- **Largest possible filter-author pool.** JavaScript is the most widely-known language.
- **Strong ecosystem.** AJV, JSON schema, validation libraries, regex are all just `npm install`.
- **Cloudflare Workers proves the model.** The Workers runtime is V8-isolates; the model works at planetary scale.

**Where it falls short.**
- **Engine size and startup cost.** V8 is a 50+ MB dependency with a non-trivial cold-start. QuickJS is small but slow. Neither matches WASM-AOT.
- **The component model isn't there.** JavaScript's interop with native code is uneven; the right ABI for a Riftgate filter (typed request/response objects, bytes views) is not a first-class JS concept.
- **Isolation depends on the engine.** V8 isolates are real; QuickJS sandboxing is weaker. Choosing V8 means embedding a multi-hundred-thousand-line C++ codebase, which is a security and supply-chain liability for a Rust gateway.
- **Operator surprise.** A Rust gateway that embeds V8 is not what an operator expects; the binary size and memory footprint visibly change.

**Real-world systems that use it.** Cloudflare Workers (V8 isolates); Deno (V8); Fastly's first iteration of Compute@Edge (V8) — Fastly migrated away to WASM, citing exactly the isolation and footprint trade.

### 3.4. Native dynamic plugins (`.so` / `.dylib` via `libloading`)

**What it is.** Filters compile to platform-native shared libraries; the gateway opens them with `dlopen` and resolves a stable symbol that returns a `Box<dyn Filter>`.

**Why it's interesting.**
- **No interpreter, no JIT, no sandbox cost.** Native code runs native speed.
- **Filter authors write Rust.** Same language as the gateway; no FFI mental tax.
- **Real precedent.** Nginx modules are this shape; HAProxy filters; Apache modules.

**Where it falls short.**
- **No sandbox at all.** A native plugin can do anything the process can: read `/etc/passwd`, mutate the heap of the gateway, segfault the binary. This is acceptable for Nginx-the-webserver where modules are vendor-authored; it is not acceptable for Riftgate-the-AI-gateway where filters may come from tenants.
- **ABI stability across compiler versions is fragile.** Rust does not have a stable ABI; a plugin built against rustc N may not load against a gateway built with rustc N+1. The mitigations (C-ABI shims, abi_stable crate) are real but add up to a parallel ABI surface.
- **Distribution and signing burden falls on us.** We would need to design a code-signing scheme to make this safe.
- **Cross-platform pain.** `.so` on Linux, `.dylib` on macOS, `.dll` on Windows. Operators handling three artifact types per filter is gratuitous.

**Real-world systems that use it.** Nginx modules; Apache modules; HAProxy SPOA (Stream Processing Offload Agent). All from an era before sandboxed bytecode was viable.

### 3.5. WebAssembly via `wasmtime`, component-model ABI

**What it is.** Filters compile to WebAssembly components (WASI Preview 2). The gateway hosts a `wasmtime::Engine` and instantiates one component instance per filter chain entry. The host exposes a fixed component-model interface — `riftgate:filter/v1` — with typed request and response objects, bytes views, and an explicit set of host functions (logging, getting the current timestamp, returning a structured terminate decision). Everything else is denied by construction: no filesystem, no network, no `process::exit`, no allocation outside the component's linear memory.

**Why it's interesting.**

- **Sandboxing is mechanical, not conventional.** A WASM component cannot do anything not granted by the host. The Riftgate host grants exactly what the filter ABI declares; the rest of the universe is unreachable. This is the property neither Lua, JS-without-V8, nor native plugins offer.
- **Component model is the right ABI shape.** WIT (`WebAssembly Interface Types`) describes the request/response/host-functions surface in a language-neutral IDL. A filter author writes Rust that compiles via `cargo component build`; nothing in the source code knows or cares that the host is Rust.
- **Hot path is sub-microsecond per filter.** wasmtime's AOT-compiled (`cwasm`) mode compiles the component once at config-load; each invocation is a typed cross-module call into native code. Published Envoy proxy-wasm numbers and wasmtime's own benchmarks put dispatch in the low microseconds, well inside our `NFR-P09` 50µs budget.
- **Declarative resource limits.** wasmtime exposes per-instance fuel (instruction counting) and memory caps; a filter that loops or allocates without bound is killed by the runtime, not by the gateway author writing defensive code.
- **The ecosystem already exists for filters.** Envoy proxy-wasm is the precedent. Fastly Compute@Edge runs production WASM filters. Cloudflare added WASI Preview 2 support to Workers in 2025. Spin (Fermyon), wasmCloud, and Lunatic are all live runtimes built on the same primitives.
- **Multi-language filter authoring.** Rust is the first-class authoring path (because we will dogfood it); Go, C, JavaScript-to-WASM (`jco`), and Python (`componentize-py`) all have working component toolchains. A platform team standardising on Go for ops tooling can still write filters.
- **Code signing and supply chain are tractable.** A `.wasm` artifact is a single immutable blob; signing it with `cosign` or `sigstore` is exactly the workflow operators already run for container images.

**Where it falls short.**

- **Component-model toolchains are still maturing.** Some language toolchains (Python, Java) are workable but rough; the rough edges live in the filter-author's world, not ours. We document the supported languages and tell others to wait.
- **Cold instantiation is not free.** A `wasmtime::Instance::new` is in the millisecond range, which would be unacceptable on the hot path. The mitigation — instance pooling per filter — is well-understood and is what `riftgate-filter` will ship.
- **Linear memory means data crosses the boundary.** The host must copy or expose memory views into the component's linear memory. We use `wasmtime`'s typed accessors; we avoid unsafe raw pointer juggling. There is a per-filter copy cost; benchmarks will measure it.
- **Debugging story is less mature than native.** A Rust filter that panics inside WASM produces a backtrace that names component-model frames, not gateway frames. We document the workflow and ship `cargo component check` recipes.

**Real-world systems that use it.** Envoy proxy-wasm; Fastly Compute@Edge; Cloudflare Workers (WASI Preview 2 support, 2025); Spin; wasmCloud; Lunatic; Lapce/Zed plugins.

**Code or config sketch (optional).** The filter ABI in WIT:

```wit
// wit/riftgate-filter.wit
package riftgate:filter@1.0.0;

interface request {
    record headers { entries: list<tuple<string, string>>; }
    record body    { bytes: list<u8>; }
}

interface response {
    record status   { code: u16; }
    record headers  { entries: list<tuple<string, string>>; }
    record body     { bytes: list<u8>; }
}

interface filter {
    use request.{headers as req-headers, body as req-body};
    use response.{status, headers as resp-headers, body as resp-body};

    variant action {
        continue,
        terminate(status),
        mutate-headers(req-headers),
        mutate-body(req-body),
    }

    on-request:  func(headers: req-headers, body: req-body) -> action;
    on-response: func(status: status, headers: resp-headers, body: resp-body) -> action;
}

world riftgate-filter {
    import host: interface {
        log:           func(level: string, message: string);
        now-millis:    func() -> u64;
        emit-counter:  func(name: string, value: u64);
    }
    export filter;
}
```

Configuration:

```toml
# riftgate.toml (sketch)
[[filter]]
name   = "pii-redactor"
path   = "/etc/riftgate/filters/pii-redactor-1.2.0.wasm"
fuel   = 5_000_000          # AOT instruction budget per request
memory = "16MiB"            # linear-memory cap
on     = ["request"]        # request-only; skip response

[[filter]]
name   = "schema-validator"
path   = "/etc/riftgate/filters/schema-validator-0.4.1.wasm"
fuel   = 2_000_000
memory = "8MiB"
on     = ["response"]
```

## 4. Tradeoff matrix

| Property | 3.1 None | 3.2 Lua | 3.3 JS (V8/QuickJS) | 3.4 Native `.so` | 3.5 WASM (wasmtime) | Why it matters |
|---|---|---|---|---|---|---|
| Sandbox is mechanical, not conventional | n/a | no | yes (V8) / partial (QuickJS) | no | **yes** | `NFR-S03` requires real isolation. |
| Declarative resource limits (fuel, memory) | n/a | no | partial | no | **yes** | Bounding misbehaving filters without gateway-author code. |
| Hot-path dispatch cost | best | ~µs+ | µs–ms | best (native) | low µs | `NFR-P09` 50µs/filter budget. |
| Cold-instantiation cost | n/a | µs | ms+ | ms (dlopen) | ms (mitigated by pooling) | First request after deploy. |
| ABI stability across host versions | n/a | excellent | excellent | poor (no stable Rust ABI) | excellent (WIT IDL) | Operators upgrade us; filters keep working. |
| Multi-language authoring | no | only Lua | only JS | only Rust (in practice) | yes (Rust/Go/JS/Py/C) | Filter authors are not the kernel team. |
| Supply-chain story | n/a | weak | weak | code-sign per-platform | single signed `.wasm` blob | Operators already run container signing. |
| Operator-perceived footprint | smallest | small | large (V8) | small | small | Binary size matters. |
| Tenant-isolation story | none | by-convention | by-engine (V8) | none | yes | Multi-tenant filter rules. |
| Documentation depth available | irrelevant | mature | mature | mature | mature | Filter authors can self-teach. |
| Migration regret risk | n/a | high (Envoy precedent) | medium (Fastly precedent) | medium (ABI churn) | low (industry converged here) | We choose once. |

## 5. Foundational principles

**Capability-based hosting (KeyKOS / EROS / seL4 lineage; Miller, *Robust Composition*).** The same principle that justifies the v0.5 MCP capability broker ([Options `026`](026-mcp-orchestration.md) §5) applies inside the filter ABI: a filter has only the capabilities the host grants. The host functions `log`, `now-millis`, `emit-counter` are the entire ambient authority. Filesystem, network, syscalls — denied by construction. This is why WASM beats Lua here: Lua's denial is by stripping `os`, `io`, `package` from the global table; WASM's denial is the absence of an imported function.

**Sandboxed extension surfaces (WASM via `wasmtime`).** WebAssembly was designed from the start as a sandboxed bytecode for untrusted code: linear memory, no raw pointers, no syscalls, structured control flow, deterministic-on-the-same-input semantics. The component model (WASI Preview 2, 2024) adds typed interface composition on top, which is what makes a host ABI like `riftgate:filter/v1` declarative rather than a hand-rolled FFI surface.

**Envoy's lesson (`proxy-wasm`).** Envoy shipped Lua filters first (2018), found the sandboxing-by-convention model insufficient for multi-tenant deployments, and migrated to proxy-wasm (2019–2021). The migration's pain — rewriting every filter, retraining every operator — is *exactly* the cost we save by starting on WASM. Envoy's experience is the strongest empirical argument here, and it is publicly documented in the Envoy blog and the proxy-wasm spec.

**Sidecar / ambassador pattern (Microsoft *Cloud Design Patterns*; Hohpe *EIP*).** Filters are the ambassador's protocol-aware policy hooks. The pattern's discipline — ambassadors enforce policy on behalf of an application without becoming the application — maps cleanly: a filter rewrites a request, denies it, or annotates it, but it does not originate traffic. This shapes the WIT ABI: no `send`-like host function, no outbound HTTP capability for filters in `v0.3`. Future scope (an MCP filter that calls a sidecar policy engine) is a deliberate, separate decision.

**AOT compilation and code-signing.** wasmtime's `Engine::precompile_component` produces a serialized native artifact (`*.cwasm`) that the gateway loads at startup. This shifts compilation out of the hot path and produces a binary that can be hashed and signed. The same operational pattern that secures container images (cosign, sigstore, SLSA provenance) applies to `.cwasm` artifacts.

## 6. Recommendation

**`v0.3` ships a WASM filter chain in a new crate `crates/riftgate-filter`, backed by `wasmtime` with the component-model ABI defined by `wit/riftgate-filter.wit` at version `1.0.0`. Native in-tree `Filter` impls remain a supported first-class path. Lua, JS, and native dynamic plugins are rejected.**

Concretely:

1. **New crate `crates/riftgate-filter`** depends on `wasmtime` with the `component-model` and `pooling-allocator` features. Empty-on-non-Linux is not required (wasmtime is cross-platform); the crate builds on macOS and Linux equally.

2. **The `Filter` trait in `riftgate-core` is unchanged.** `IdentityFilter` and `LoggingFilter` continue to work. A new struct `WasmFilter` in `riftgate-filter` implements the same `Filter` trait by delegating to a `wasmtime::Component`. The filter chain executor is a thin wrapper that runs in-tree filters and `WasmFilter`s through the same trait surface.

3. **The host ABI is frozen at `riftgate:filter/v1` for `v0.3`.** Breaking changes require a new WIT package version (`v2`) and a new ADR. Filters declare which ABI version they target.

4. **Resource limits per filter:**
   - `fuel`: AOT instruction budget per request invocation. Default 5 million. Exceeding fuel returns a structured `503 Service Unavailable` with `riftgate-filter-error: fuel-exhausted`.
   - `memory`: linear-memory cap per instance. Default 16 MiB. Exceeding memory aborts the instance and records `riftgate-filter-error: memory-exhausted`.
   - `wallclock`: hard ceiling on filter wall-time, enforced by an interrupt sent from a timer (binary-heap timer subsystem, see [Options `006`](006-timer-subsystem.md)). Default 50ms.

5. **Instance pooling.** `wasmtime::PoolingAllocationConfig` is used to pre-allocate a pool of instances per filter. The hot path is `Instance::call_typed`, not `Instance::new`. Pool size defaults to `num_shards * 2`.

6. **Host functions exposed (the complete ambient authority):**
   - `log(level: string, message: string)` — emits a tracing event tagged with the filter name.
   - `now-millis() -> u64` — monotonic milliseconds since process start.
   - `emit-counter(name: string, value: u64)` — emits a counter through `ObservabilitySink`.
   - **No** filesystem, network, environment, random, or process functions in `v0.3`. Future scope (`riftgate:filter/v2` for v0.4+) adds explicitly-named capabilities under per-filter grants.

7. **AOT precompile at config-load.** The binary calls `Engine::precompile_component` and stores the result in a memory-mapped cache. Reloading a filter with the same hash skips precompilation. Reload events trigger fresh precompile in the background; the live filter chain swaps atomically when the new component is ready.

8. **Observability per filter invocation.** Every `on-request` and `on-response` call emits an OTel span with attributes `riftgate.filter.name`, `riftgate.filter.action` (`continue` / `terminate` / `mutate-headers` / `mutate-body`), `riftgate.filter.duration_us`, and on error `riftgate.filter.error` (`fuel-exhausted` / `memory-exhausted` / `wallclock-exceeded` / `panic`).

9. **Starter filter library ships under `examples/02-starter-filters/`**, not as a `crates/` crate: PII redactor, prompt template substitution, output schema validator, cost guard, token-budget guard. Each is a self-contained Rust project that compiles to a component with `cargo component build --release`. We deliberately do not productize them as a separate workspace crate — they are reference implementations.

### Conditions under which we'd revisit

- If the WIT IDL evolves in incompatible ways such that `riftgate:filter/v1` cannot represent a request without contortion (e.g. genuinely-streaming bodies for very large prompts), we open a `v2` Options doc and a paired ADR. The `v1` ABI continues to work until the next major release.
- If wasmtime's resource-limit story degrades or a sustained CVE pattern emerges, we evaluate `wasmer` as an alternative engine behind the same WIT ABI. The ABI is the moat; the engine is replaceable.
- If a credible benchmark shows hot-path dispatch in WASM regresses past 50µs/filter at p99 for the starter filter library, we revisit instance pooling and AOT-cache shape before reaching for native plugins.

## 7. What we explicitly reject

- **None (in-tree only).** Defeats the v0.3 pillar; concentrates security risk on maintainers. Reconsider only if the project decides to stop being programmable, which would be a different project.
- **Lua.** Sandboxing by convention; Envoy ran this experiment and migrated away; performance ceiling. Reconsider only if Lua acquires a host-grantable capability model and a verified-bytecode runtime — neither is on the roadmap.
- **JavaScript via V8/QuickJS.** Engine size, ABI fit, and the supply-chain cost of embedding a C++ runtime in a Rust gateway. Reconsider only if a Rust-native JS engine with verified sandboxing matures (Boa is interesting but not production-ready for this).
- **Native dynamic plugins (`.so`).** No sandbox; ABI churn; per-platform artifacts. Reconsider only if Rust acquires a stable ABI *and* tenant isolation is no longer a concern — neither will happen.
- **eBPF as a filter mechanism.** Tempting (capability-based, in-kernel, sandbox-by-verifier). Rejected because eBPF lives at the syscall/kernel layer, not the HTTP-semantics layer; filters need to read request bodies as bytes, which is structurally not eBPF's job. eBPF stays in the observability plane (`v0.4`).
- **Multiple extension mechanisms shipped in parallel.** Two extension surfaces means twice the security review, twice the docs, twice the operator confusion. We pick one and live with the consequences.
- **An RPC sidecar (Envoy `ext-proc`-style) as the v0.3 default.** Catalogued, but rejected for the default path: the network hop adds latency that defeats the value of an in-process gateway. Operators who want sidecar policy will have it as a future option (likely v1.0+) — they will not have it as the v0.3 starter.

## 8. References

1. WebAssembly Component Model specification — <https://github.com/WebAssembly/component-model>
2. WASI Preview 2 — <https://github.com/WebAssembly/WASI/blob/main/preview2/README.md>
3. wasmtime — <https://wasmtime.dev/>
4. WIT (Wasm Interface Type) reference — <https://component-model.bytecodealliance.org/design/wit.html>
5. Envoy proxy-wasm specification — <https://github.com/proxy-wasm/spec>
6. Envoy blog, *WebAssembly: The Future of Extensibility for Envoy* (2019) — the migration narrative.
7. Fastly, *Compute@Edge architecture* — <https://www.fastly.com/products/edge-compute>
8. Cloudflare, *Bringing WASI Preview 2 to Workers* (2025) — <https://blog.cloudflare.com/>
9. Spin (Fermyon) — <https://www.fermyon.com/spin>
10. wasmCloud — <https://wasmcloud.com/>
11. Mark S. Miller, *Robust Composition: Towards a Unified Approach to Access Control and Concurrency Control* (PhD thesis, Johns Hopkins, 2006).
12. Norm Hardy, *KeyKOS Architecture* (1985).
13. Microsoft Azure, *Cloud Design Patterns: Ambassador* — <https://learn.microsoft.com/en-us/azure/architecture/patterns/ambassador>.
14. Gregor Hohpe and Bobby Woolf, *Enterprise Integration Patterns* (Addison-Wesley, 2003).
15. SLSA (Supply-chain Levels for Software Artifacts) — <https://slsa.dev/>
16. Sigstore / cosign — <https://www.sigstore.dev/>
17. OpenResty / `lua-nginx-module` — <https://openresty.org/> (the Lua precedent we are declining).
18. HAProxy SPOA documentation — <https://docs.haproxy.org/2.8/management.html#9.3> (the native-plugin precedent we are declining).
