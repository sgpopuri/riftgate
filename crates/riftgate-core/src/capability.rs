//! `CapabilityBroker` trait and MCP request types — extension-plane surface for v0.5.
//!
//! Per [ADR 0015](../../../docs/06-adrs/0015-mcp-extension-plane-broker.md)
//! (accepted 2026-07-04) the v0.5 default impl is `AllowlistBroker` in
//! `crates/riftgate-mcp`. The v0.1 placeholder `check_tool`/`check_resource`
//! methods are superseded by the single `authorize()` method defined here.
//!
//! See [Options 026](../../../docs/05-options/026-mcp-orchestration.md) and
//! [`docs/04-design/lld-mcp-capability.md`](../../../docs/04-design/lld-mcp-capability.md).

use std::time::SystemTime;

use crate::types::{RequestId, TenantId};

/// Identifier for a tool the gateway is brokering access to.
///
/// In MCP terms this is the `name` field of a `tools/call` request.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ToolId(pub String);

impl ToolId {
    /// Borrow the underlying tool name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ToolId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ToolId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Identifier for a resource the gateway is brokering access to.
///
/// MCP resources are URI-shaped (e.g. `s3://datasets/train.jsonl`).
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct ResourceId(pub String);

impl ResourceId {
    /// Borrow the underlying URI as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for ResourceId {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for ResourceId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// A parsed MCP request — the logical representation the capability broker sees.
///
/// Produced by the parser in `crates/riftgate-mcp`; kept in `riftgate-core`
/// so `CapabilityBroker` can reference it without a circular crate dependency.
/// Carries only what the broker needs for authorization; the full JSON-RPC
/// payload never crosses the broker boundary.
#[derive(Debug, Clone)]
pub enum McpRequest {
    /// `tools/call` — invoke a named tool with arguments.
    ToolCall {
        /// The tool being invoked.
        tool: ToolId,
        /// SHA-256 of the serialized arguments; opaque for audit without leaking PII.
        argument_hash: [u8; 32],
    },
    /// `tools/list` — enumerate available tools.
    ToolList,
    /// `resources/read` — read a named resource.
    ResourceRead {
        /// The resource URI.
        resource: ResourceId,
    },
    /// `resources/list` — enumerate available resources.
    ResourceList,
    /// `prompts/get` — retrieve a named prompt.
    PromptGet {
        /// The prompt name.
        name: String,
    },
    /// `prompts/list` — enumerate available prompts.
    PromptList,
    /// Protocol-lifecycle methods (initialize, ping, shutdown).
    Lifecycle {
        /// Which lifecycle method.
        method: LifecycleMethod,
    },
}

/// Protocol-lifecycle method variant.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum LifecycleMethod {
    /// MCP `initialize` handshake.
    Initialize,
    /// MCP `ping` keepalive.
    Ping,
    /// MCP `shutdown` teardown.
    Shutdown,
}

/// Resolved tenant identity at the time of capability check.
#[derive(Debug, Clone)]
pub struct TenantIdentity {
    /// Numeric tenant identifier.
    pub tenant: TenantId,
    /// Authenticated caller principal (JWT `sub`, API key id, etc.).
    pub principal: String,
}

/// Outcome of a capability authorization check.
#[derive(Debug, Clone)]
pub enum CapabilityDecision {
    /// Request is authorized; carries attestation headers for the downstream MCP server.
    Allow {
        /// Signed headers to forward downstream.
        attestation: AttestationHeaders,
    },
    /// Request is denied.
    Deny {
        /// Reason for the denial.
        reason: DenialReason,
    },
}

/// Attestation headers forwarded on allowed MCP requests.
///
/// The downstream MCP server can verify `signature` using the shared gateway
/// signing key to confirm the gateway authorized this request.
#[derive(Debug, Clone)]
pub struct AttestationHeaders {
    /// Tenant that originated the request.
    pub caller: TenantId,
    /// Tool or resource subject being accessed.
    pub subject: String,
    /// Always `"allow"` on an `Allow` variant; `"allow-dry-run"` in dry-run mode.
    pub decision: &'static str,
    /// HMAC-SHA256 of `caller || subject || decision` under the gateway signing key.
    pub signature: HmacSignature,
}

/// 32-byte HMAC-SHA256 signature (raw bytes).
#[derive(Clone)]
pub struct HmacSignature(pub [u8; 32]);

impl std::fmt::Debug for HmacSignature {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HmacSignature([redacted])")
    }
}

/// Reason for a capability denial, surfaced as `riftgate-mcp-reason` header.
#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DenialReason {
    /// Tool is not in the tenant's allowed-tool set.
    NotInAllowlist,
    /// Tool is explicitly in the tenant's denied-tool set.
    InDenylist,
    /// A time-bounded grant for this tool has expired.
    TimeBoundGrantExpired,
    /// The MCP request could not be parsed.
    MalformedMcpRequest,
    /// The tenant identity could not be resolved.
    TenantUnknown,
}

impl DenialReason {
    /// Short string for the `riftgate-mcp-reason` response header.
    pub fn as_header_value(&self) -> &'static str {
        match self {
            Self::NotInAllowlist => "not_in_allowlist",
            Self::InDenylist => "in_denylist",
            Self::TimeBoundGrantExpired => "time_bound_grant_expired",
            Self::MalformedMcpRequest => "malformed_mcp_request",
            Self::TenantUnknown => "tenant_unknown",
        }
    }
}

/// Structured audit event written to the WAL for every `authorize()` call.
///
/// Serialized as newline-delimited JSON by `AllowlistBroker` in
/// `crates/riftgate-mcp` and appended via `WAL::append` with `Durability::Fsync`.
#[derive(Debug, Clone)]
pub struct McpAuditEvent {
    /// Correlates to the in-flight request.
    pub correlation_id: RequestId,
    /// Tenant that originated the request.
    pub tenant: TenantId,
    /// Tool name, resource URI, or method name — what was being accessed.
    pub subject: String,
    /// SHA-256 of the arguments (opaque; keeps PII out of the audit log).
    pub argument_hash: [u8; 32],
    /// The authorization outcome.
    pub decision: AuditDecision,
    /// Wall-clock time of the decision.
    pub timestamp: SystemTime,
}

/// Audit-log-safe representation of the authorization outcome.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum AuditDecision {
    /// Request was allowed.
    Allow,
    /// Request was denied.
    Deny,
}

/// Per-tenant MCP capability broker.
///
/// The single `authorize()` method checks an incoming MCP request against
/// the tenant's configured allowlist, records the decision to the WAL, and
/// returns either an `Allow` (with HMAC-signed attestation headers) or a
/// `Deny` (with a typed reason).
///
/// **`Send + Sync`** — one broker instance per process, shared via `Arc`.
///
/// Every decision MUST be audited via the WAL; the `AllowlistBroker` impl
/// in `crates/riftgate-mcp` handles this internally. Callers are NOT
/// responsible for writing the audit event.
///
/// Trait object safety: yes.
pub trait CapabilityBroker: Send + Sync {
    /// Authorize an MCP request for the given tenant identity.
    ///
    /// Returns `Allow { attestation }` if the request should proceed, or
    /// `Deny { reason }` if it should be rejected with a `403`.
    fn authorize(
        &self,
        request: &McpRequest,
        identity: &TenantIdentity,
    ) -> CapabilityDecision;
}
