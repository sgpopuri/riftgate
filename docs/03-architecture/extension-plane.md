# 03.b Extension Plane

> Pluggable behavior: filters and routing strategies. The extension plane is what makes Riftgate a *framework* rather than a product.
>
> Status: **outline-stage**. Filled out as `v0.3` (programmability milestone) approaches.

## What lives here

- The filter chain (`Filter` trait + chain executor)
- The WASM runtime (wasmtime)
- The routing strategies (`Router` trait + built-in impls)
- The plugin loader

## Filter contract

A filter sees a typed `Request` or `Response` and returns a `FilterAction`:

```rust
// Sketch
pub enum FilterAction {
    Continue,
    Modify(Request),    // or Response
    Terminate(StatusCode, Body),
}

pub trait Filter: Send + Sync {
    fn on_request(&self, req: &mut Request) -> FilterAction { FilterAction::Continue }
    fn on_response(&self, resp: &mut Response) -> FilterAction { FilterAction::Continue }
    fn on_token(&self, token: &Token) -> FilterAction { FilterAction::Continue }  // streaming
}
```

Filters are ordered. A `Terminate` short-circuits the chain. A `Modify` propagates the modified value to subsequent filters. Filter authors are expected to be cheap and side-effect-free unless explicitly granted capabilities.

## Routing strategy contract

A router sees a `Request` and a `BackendPool`, returns a `RoutingDecision`:

```rust
// Sketch
pub enum RoutingDecision {
    Send(BackendId),
    Hedge(Vec<BackendId>),  // race; first to respond wins
    Reject(StatusCode),
}

pub trait Router: Send + Sync {
    fn route(&self, req: &Request, pool: &BackendPool) -> RoutingDecision;
    fn on_response(&self, _decision: &RoutingDecision, _outcome: &Outcome) {}
}
```

The `on_response` hook lets routers learn (e.g. update circuit-breaker state, evict failing backends).

## WASM filters

Filters can be authored in Rust (or any wasm32-wasip1-targeting language) and loaded at runtime via wasmtime. The host exports a narrow set of capabilities:

- Read/modify request headers and body
- Emit log records with bounded size
- Read configuration values declared in the manifest
- *(Not granted)* host filesystem, network, environment variables

Capability grants are explicit per-filter in config. The default is no capabilities beyond request/response access.

## Built-in starter filters (planned for `v0.3`)

- **PII redactor** — masks well-known PII patterns (emails, phone numbers, SSNs) from prompts and/or responses.
- **Prompt template substitution** — applies a templated system prompt prefix.
- **Output schema validator** — for JSON-mode responses, validates against a configured schema.
- **Cost guard** — rejects requests whose estimated cost exceeds a per-tenant or per-route budget.

## Built-in routing strategies (planned)

- **Round-robin** (`v0.1`)
- **Weighted-random** (`v0.2`)
- **KV-cache-aware** (`v0.3`) — integrates with `vllm-router`'s LMCache or uses a built-in prefix trie. See [Options 010](../05-options/010-routing-strategy.md).
- **Hedged requests** (`v0.3`) — race two backends, accept first, cancel slower mid-stream.

## Open design questions

- Should filters be allowed to spawn async work? Default: no, to keep the hot path predictable.
- How do we handle filter ordering conflicts in config? Recommend explicit order with a validation pass at startup.
- Should routing strategies see the WAL for prior decisions? Recommend yes via a read-only view; this enables learning without coupling.
