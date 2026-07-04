# Upgrading Riftgate

This document covers configuration and behavioral changes operators need to
know about when upgrading between Riftgate versions. It is organized by the
version you are upgrading **to**.

Riftgate is distributed from the GitHub repository as source code. There is no
`cargo install` or crates.io package. To upgrade, pull the tag and rebuild:

```bash
git fetch --tags
git checkout v<target>
cargo build --release -p riftgate
```

---

## Upgrading to v1.0

### New: `[multitenancy]` config section (ADR 0029)

A new `[multitenancy]` section is available in `riftgate.toml`. It is **optional
and defaults to `trusted-header` mode**, which preserves the existing behavior
where `x-riftgate-tenant` is read directly from the request header.

To enable API key authentication (recommended for internet-facing deployments):

```toml
[multitenancy]
mode = "api-key"

[multitenancy.api_keys]
# Keys are stored as SHA-256 hex of the raw Bearer token.
# Value is the tenant name (mapped to TenantId via FNV-1a hash).
"sha256:<64-hex-chars>" = "acme"
```

**No action required** for existing deployments running in trusted internal
networks. Existing `x-riftgate-tenant` header behavior continues unchanged
under the default `trusted-header` mode.

### New: `[mcp]` config section (ADR 0015)

The MCP capability broker is now active when `[mcp.tenants]` entries are
configured. Without any `[mcp.tenants]` entries the broker is disabled and
all MCP-shaped requests pass through unchanged — **no behavior change for
deployments that do not use MCP**.

```toml
[mcp]
enforce = true          # false = dry-run mode

[mcp.tenants."1"]       # numeric id or FNV-1a hashed name
allowed_tools = ["search-web"]
denied_tools  = ["filesystem-write"]
```

### New: Kubernetes operator (`crates/riftgate-operator`, ADR 0030)

The `riftgate-operator` binary and Helm chart are new in v1.0. They are
**additive** — the standalone binary continues to work unchanged. The operator
is opt-in; no existing deployment needs to migrate.

To install via Helm:
```bash
helm install riftgate-operator deploy/helm/riftgate-operator/
```

CRD API group: `riftgate.io/v1alpha1`. Stability is `v1alpha1`; breaking
schema changes are possible until promotion to `v1beta1`.

---

## Upgrading to v0.5

### Breaking: `CapabilityBroker` trait method signature changed

`CapabilityBroker::check_tool` and `check_resource` (v0.1 placeholders) were
replaced by a single `authorize(request, identity) -> CapabilityDecision` method.

**Impact:** code that directly implemented `CapabilityBroker` must be updated.
The v0.1 placeholder impls in `riftgate-core` are removed. Use
`AllowlistBroker` or `DryRunBroker` from `crates/riftgate-mcp`.

### New: MCP parser crate

`crates/riftgate-mcp` is new. Add it as a dependency if you depend on the
`CapabilityBroker` trait:

```toml
[dependencies]
riftgate-mcp = { path = "crates/riftgate-mcp" }
```

---

## Upgrading to v0.4

### New: `BpfSink` feature flag

The `bpf` feature on the `riftgate` binary is new. It is **opt-in**:

```bash
cargo build --release -p riftgate --features bpf
```

Without `--features bpf` the binary compiles identically to v0.3. With it,
set `RIFTGATE_ENABLE_BPF=1` at runtime to load Aya BPF programs.

### New: GPU pressure polling

Set `RIFTGATE_GPU_DCGM_ENDPOINT=http://<host>:9400/metrics` to enable background
GPU pressure polling into the routing signal store. Without this env var, routing
behavior is unchanged from v0.3.

---

## Upgrading to v0.3

### New: filter chain config

`crates/riftgate-filter` is new. WASM filters are loaded via config:

```toml
[[filter]]
path = "/path/to/filter.wasm"
```

If no `[[filter]]` entries are present, the filter chain is an identity
pass-through — **no behavior change from v0.2**.

### New: `KvAwareRouter` and `HedgedRouter`

These are additive routing strategies. Existing deployments using round-robin
or weighted-random routing are unaffected.

---

## Upgrading to v0.2

### New: `io-uring` feature (Linux only)

`--features io-uring` on `crates/riftgate-io-uring` enables the io_uring IO
backend. Not enabled by default. On non-Linux platforms the crate compiles to
an empty surface.

### Config changes

`v0.2` adds optional config fields to `riftgate.toml`:

```toml
# New in v0.2 — all optional; existing configs continue to work.
[backend]
# timeout_ms was already present; no new required fields.

# Rate limiting is new and disabled by default (no [rate_limit] section needed).
```

No existing `v0.1` config files need changes to run under v0.2.

---

## Upgrading to v0.1

v0.1 is the initial release. No prior version to upgrade from.
