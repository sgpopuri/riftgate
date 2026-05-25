//! `CapabilityBroker` trait — defined in `v0.1`, default impl deferred to `v0.5`.
//!
//! Per [ADR 0015](../../../docs/06-adrs/0015-mcp-extension-plane-broker.md)
//! (proposed) the v0.5 default impl is a per-tenant allowlist with WAL-backed
//! audit; the v0.1 trait shape is locked in here so the future MCP-aware
//! request path compiles against it without a breaking change later.
//!
//! See [Options 026](../../../docs/05-options/026-mcp-orchestration.md) and
//! [`docs/04-design/lld-mcp-capability.md`](../../../docs/04-design/lld-mcp-capability.md).

use crate::types::TenantId;

/// Identifier for a tool the gateway is brokering access to.
///
/// In MCP terms this is the value of the `name` field in a `tools/call`
/// request; we wrap it in a newtype so a `ToolId` cannot be confused with
/// any other string at a call site.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ToolId(pub String);

/// Identifier for a resource the gateway is brokering access to.
///
/// MCP resources are URI-shaped (e.g. `file:///etc/passwd`); we keep them
/// as opaque strings for `v0.1` and let the `v0.5` impl parse / validate.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ResourceId(pub String);

/// Outcome of a capability check.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum CapabilityDecision {
    /// Tenant is permitted to invoke the tool / read the resource.
    Allow,
    /// Tenant is denied.
    Deny {
        /// Human-readable reason; surfaces as the `riftgate.mcp.reason`
        /// header on the `403` response.
        reason: String,
    },
}

/// Per-tenant tool / resource capability broker.
///
/// **Trait shape only in `v0.1`.** The v0.5 default impl
/// (`AllowlistCapabilityBroker`) lives in `crates/riftgate-mcp`.
///
/// **`Send + Sync`** — one broker instance per process, shared via `Arc`.
///
/// Every decision MUST be audited via the WAL; the `v0.5` impl writes a
/// `McpAuditEvent` for every call. Callers are not responsible for the
/// audit (the broker is); this keeps the audit path reachable from every
/// caller without per-call-site discipline.
///
/// Trait object safety: yes.
pub trait CapabilityBroker: Send + Sync {
    /// Check whether `tenant` may invoke `tool`.
    fn check_tool(&self, tenant: TenantId, tool: &ToolId) -> CapabilityDecision;

    /// Check whether `tenant` may read `resource`.
    fn check_resource(&self, tenant: TenantId, resource: &ResourceId) -> CapabilityDecision;
}
