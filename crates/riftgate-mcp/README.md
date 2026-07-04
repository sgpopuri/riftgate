# riftgate-mcp

MCP capability broker for the Riftgate gateway.

Implements the gateway-as-broker posture from
[ADR 0015](../../docs/06-adrs/0015-mcp-extension-plane-broker.md):
parse every MCP (Model Context Protocol) request, authorize it against a
per-tenant allowlist, write a durable WAL audit event, and return either an
`Allow` with HMAC-signed attestation headers or a `Deny` with a typed reason.

## Crate layout

| Module | Purpose |
|--------|---------|
| `parser` | JSON-RPC 2.0 + MCP message parsing (`parse(bytes) -> McpRequest`) |
| `allowlist` | `AllowlistBroker` — per-tenant bit-set, prefix-set, time-bounded grants |
| `dryrun` | `DryRunBroker` — wraps any broker; logs would-be denials, always passes |
| `attestation` | HMAC-SHA256 signing key and attestation header generation / verification |
| `audit` | Serialize `McpAuditEvent` as NDJSON and append to the WAL |

## Quick start

```rust
use std::collections::HashMap;
use std::sync::Arc;
use riftgate_mcp::{AllowlistBroker, SigningKey, TenantAllowlist};
use riftgate_core::capability::CapabilityBroker;

let mut tenants = HashMap::new();
tenants.insert(1u32, TenantAllowlist {
    allowed_tools: vec!["search-web".to_owned()],
    ..Default::default()
});

let broker = AllowlistBroker::new(&tenants, SigningKey([0u8; 32]), wal);
```

## MCP config schema (TOML)

```toml
[mcp.tenants.acme]
allowed_tools             = ["search-web", "read-file"]
denied_tools              = ["filesystem-write"]
allowed_resource_prefixes = ["s3://acme-datasets/*"]
time_bounded_grants = [
  { tool = "send-email", until_unix_secs = 1780000000 },
]
```

Set `enforce = false` under `[mcp]` to enable dry-run mode (decisions are logged
but every request passes through).

## Design

See [`docs/04-design/lld-mcp-capability.md`](../../docs/04-design/lld-mcp-capability.md)
for the full design rationale, data-structure choices, and agent guidance.
