# 04.j LLD — MCP Capability Broker

> First-class MCP (Model Context Protocol) support: parse MCP requests, enforce per-tenant tool and resource allowlists, audit every invocation via the existing WAL. The gateway as the capability ledger of the agentic era.
>
> Status: **outline-stage**. Filled out as `v0.5` lands.

## Purpose

Authorize every [Model Context Protocol](https://modelcontextprotocol.io/) request that passes through Riftgate against a per-tenant allowlist of tools and resources, emit structured attestation headers on allowed requests, and write every decision (allow or deny) to a durable audit ledger.

Explicitly in scope: parsing, allowlist evaluation, attestation-header emission, audit-log append, dry-run mode.

Explicitly NOT in scope for `v0.5`: rewriting MCP payloads (the mediator posture — see Options [`026` §3.4](../05-options/026-mcp-orchestration.md)), tool-dependency DAG evaluation, external policy-engine delegation (OPA / Cedar).

## Trait surface

```rust
// Sketch — actual signatures in riftgate-core
pub enum CapabilityDecision {
    Allow { attestation: AttestationHeaders },
    Deny { reason: DenialReason, retry: RetrySemantics },
}

pub struct AttestationHeaders {
    pub caller: TenantId,
    pub tool: String,
    pub decision: &'static str,   // "allow" | "deny"
    pub signature: HmacSignature, // gateway-signed so downstream MCP servers can verify
}

pub enum DenialReason {
    NotInAllowlist,
    InDenylist,
    TimeBoundGrantExpired,
    MalformedMcpRequest,
    TenantUnknown,
}

pub trait CapabilityBroker: Send + Sync {
    fn authorize(&self, mcp_request: &McpRequest, identity: &TenantIdentity)
        -> CapabilityDecision;
}
```

## Implementations

| Impl | Status | Source crate | Notes |
|------|--------|--------------|-------|
| `AllowlistBroker` | `v0.5` | `riftgate-mcp` | Bit-set + prefix-trie + interval-tree composition (see §Allowlist data structures below). |
| `DryRunBroker` | `v0.5` | `riftgate-mcp` | Wraps any inner broker; logs the decision but always returns `Allow`. Enabled by `enforce = false` in config. |
| `ExternalPolicyBroker` | future | `riftgate-mcp-opa` (not yet) | Delegates to OPA / Cedar over gRPC. Behind a feature flag. |

Decision rationale, candidates, and rejected alternatives: see [Options `026` (MCP orchestration)](../05-options/026-mcp-orchestration.md).

Foundational principles: ambassador pattern (Microsoft *Cloud Design Patterns*; Hohpe, *Enterprise Integration Patterns*); object-capability security (KeyKOS / EROS / seL4; Mark Miller, *Robust Composition*). Supplementary data-structure references inline below.

## Component context

### Architecture and dependencies

The capability broker sits in the extension plane, after the request-side filter chain and before the router:

```text
Client -> IO -> Parser -> Queue -> RateLimiter -> Scheduler -> Filter chain -> CapabilityBroker -> Router -> Backend
                                                                                        |
                                                                                        +-> WAL (audit event)
                                                                                        +-> OTel (structured log)
```

For an MCP `tools/call` request, the broker returns either `Allow { attestation }` (request proceeds with attestation headers) or `Deny { reason, retry }` (request is terminated with a structured `403` and a `riftgate-mcp-reason` header).

Dependencies:

- [WAL](lld-storage.md) for audit-event persistence — reuses the same append-only primitive used by the request log.
- [Observability plane](lld-observability.md) for structured-log emission of decisions.
- [Multitenancy config](../05-options/017-multitenancy.md) for tenant identity resolution.
- [Filter chain](../03-architecture/extension-plane.md) — the broker runs after filters so PII redaction and prompt rewrites have already happened when the authorization decision is made.

The broker does NOT depend on the router, the scheduler, or the IO subsystem.

### Parser

The MCP parser in `crates/riftgate-mcp` understands the four core MCP message families:

- `tools/list`, `tools/call`
- `resources/list`, `resources/read`
- `prompts/list`, `prompts/get`
- `initialize`, `ping`, `shutdown` (protocol lifecycle)

It emits typed `McpRequest` values. It does NOT rewrite payloads.

### Allowlist data structures

A production `AllowlistBroker` composes three structures, each chosen for its case:

- **Bit-set over a fixed tool registry** (the standard data-structures literature on bit-vectors and bitmap indexes). O(1) allow/deny on the common path when the tool vocabulary is small and enumerable (e.g. 64-128 tools). Memory is `⌈|tools| / 64⌉` u64 words per tenant.
- **Prefix trie for resource URIs** (Knuth TAOCP §6.3 on digital searching; the radix-tree / patricia-tree literature). O(|path|) match for patterns like `s3://acme-datasets/*` or `https://docs.acme.com/*`. Handles hierarchical grants naturally.
- **Interval tree for time-bounded grants** (CLRS ch. 14; segment-tree and Fenwick-tree variants for range queries). O(log n) lookup for "this permission expires at T." Supports the common `time_bounded_grants` config shape.

The composition order on a `tools/call`:

1. Is the tool in the tenant's bit-set allowlist? If no → `Deny`.
2. Is the tool in the bit-set denylist? If yes → `Deny`.
3. Do all referenced resource URIs match the tenant's trie prefixes? If no → `Deny`.
4. Are any time-bounded grants required? Check the interval tree; if expired → `Deny`.
5. Otherwise → `Allow { attestation }`.

### Audit ledger

Every `authorize()` call produces an `McpAuditEvent`:

```rust
pub struct McpAuditEvent {
    pub correlation_id: RequestId,
    pub tenant: TenantId,
    pub tool: String,
    pub argument_hash: Sha256,   // opaque, for forensics without leaking PII
    pub decision: CapabilityDecision,
    pub timestamp: Instant,
}
```

Events are appended to the WAL (reusing `riftgate-replay`) and emitted to OTel as structured logs. The WAL append is fire-and-forget — the broker does NOT block on durability. If the WAL is saturated, the drop-counter on the observability plane increments; decisions are still made.

### Attestation headers

On `Allow`, Riftgate injects three headers before the request reaches the backend MCP server:

- `riftgate-mcp-caller: <tenant-id>`
- `riftgate-mcp-tool: <tool-name>`
- `riftgate-mcp-decision: allow`

Plus an HMAC signature header signed with a gateway-owned key:

- `riftgate-mcp-attestation: <hmac-sha256(headers || request-id)>`

Downstream MCP servers can verify the signature to cross-check the gateway's authorization — a modest defense-in-depth mechanism.

### Patterns and conventions

- **Broker is pure.** Same input (MCP request + tenant identity) must yield the same decision, assuming the configured allowlist is unchanged. No timing-dependent logic except time-bounded-grant checks.
- **Decisions are small.** An `Allow` carries attestation; a `Deny` carries a reason enum. Neither is allowed to carry raw request bytes.
- **Audit is fire-and-forget.** The hot path never blocks on durability.
- **Dry-run is a wrapper, not a mode.** `DryRunBroker` implements the same trait as `AllowlistBroker` — no conditional code inside the allowlist path.
- **Configuration is declarative.** Policy loads from `riftgate.toml` at `v0.5`, from CRDs at `v1.0`. The config schema is versioned.

### Pitfalls

- **Stale policy after hot reload.** If a tenant's allowlist is updated, in-flight requests still use the old policy. Document this clearly; a "drain before reload" mode may be required for some deployments.
- **Oversized argument hashes.** A tool whose arguments include a 1MB file hash blows the WAL event size. Bound the hashed region and note when it was truncated.
- **Attestation-signature key rotation.** The HMAC key needs a rotation story; we start with static key per-gateway-process and document the upgrade path.
- **MCP protocol evolution.** Every new protocol version is a parser update. The LLD for the parser (embedded here; may split out later) documents which MCP spec version we target at ship time.
- **Denylist vs allowlist precedence.** Standardize on "denylist wins." Make this explicit in the docs and tests.
- **"Why was I denied?" DX.** The `riftgate-mcp-reason` header must be structured enough that a debugging operator can act on it without reading logs.

### Standards and review gates

- Parser changes require a corpus-based protocol-fidelity test (replay recorded MCP traffic against a reference server and the Riftgate parser; diff).
- Changes to `CapabilityDecision` or `DenialReason` require an ADR — they are part of the public trait surface.
- Any impl that delegates policy externally (like `ExternalPolicyBroker`) must emit the same audit events as the in-proc `AllowlistBroker`.
- Every policy-denial case requires a structured OTel log entry; generic "policy denied" is not acceptable.

## Testing strategy

- **Parser conformance** — replay a captured MCP corpus; diff against the reference server's response shape.
- **Allowlist property tests** — generated tenant configs and requests; verify monotonicity (denylist-adds never allow more, allowlist-adds never deny more).
- **WAL audit round-trip** — decide 10k invocations, read the WAL back via `riftgate-replay`, verify every event is present.
- **Attestation verification** — downstream-server test harness validates the HMAC signature.
- **Dry-run parity** — `DryRunBroker(AllowlistBroker)` must emit the same audit events as `AllowlistBroker` would (differing only in outcome).
- **Soak with config churn** — 1h of continuous policy reloads under load; verify no dropped audit events.

## Open questions

- Should the broker emit a tenant-scoped Prometheus counter for denials (`riftgate_mcp_denial_total{tenant, reason}`)? Recommend yes; cardinality is bounded by tenant count.
- Should we support session-scoped capability grants (a tenant can reach tool X only within a specific MCP session)? Recommend no for `v0.5`; revisit in `v1.0` once session semantics in MCP itself stabilize.
- Should the attestation signature include the argument hash? Recommend yes — otherwise the gateway attests "this tenant was allowed to call X" without committing to *with what arguments*.
- How do we evolve policy format without breaking deployed configs? Recommend a `policy_version` field in the config; broker supports the N-1 version for one minor release.
