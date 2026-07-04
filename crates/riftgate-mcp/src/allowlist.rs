// riftgate-mcp/src/allowlist.rs
//
// AllowlistBroker: per-tenant tool + resource capability enforcement.
//
// Data structures per lld-mcp-capability.md:
//   - HashSet<String> for allowed/denied tools  (O(1) membership test)
//   - Sorted Vec<String> of resource URI prefixes (O(log n + prefix_len) match)
//   - Sorted Vec<(SystemTime, String)> of time-bounded grants (O(log n) skip expired)
//
// Every authorize() call writes an McpAuditEvent to the WAL with Durability::Fsync
// before returning. The caller does not need to arrange audit independently.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use riftgate_core::capability::{
    AuditDecision, CapabilityBroker, CapabilityDecision, DenialReason,
    LifecycleMethod, McpAuditEvent, McpRequest, TenantIdentity,
};
use riftgate_core::types::RequestId;
use riftgate_core::wal::WAL;
use serde::Deserialize;
use tracing::warn;

use crate::attestation::{sign, SigningKey};
use crate::audit;

// ---------------------------------------------------------------------------
// Configuration types (deserializable from TOML via riftgate-config)
// ---------------------------------------------------------------------------

/// Per-tenant allowlist configuration, loaded from the gateway TOML config.
///
/// ```toml
/// [mcp.tenants.acme]
/// allowed_tools             = ["search-web", "read-file"]
/// denied_tools              = ["filesystem-write"]
/// allowed_resource_prefixes = ["s3://acme-datasets/*", "https://docs.acme.com/*"]
/// time_bounded_grants = [
///   { tool = "send-email", until_unix_secs = 1780000000 },
/// ]
/// ```
#[derive(Debug, Clone, Deserialize, Default)]
pub struct TenantAllowlist {
    /// Tools this tenant may invoke.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    /// Tools explicitly denied (takes precedence over `allowed_tools`).
    #[serde(default)]
    pub denied_tools: Vec<String>,
    /// Resource URI prefixes this tenant may read. A trailing `*` is treated
    /// as a prefix glob and stripped before storage.
    #[serde(default)]
    pub allowed_resource_prefixes: Vec<String>,
    /// Temporary tool grants that expire at a given Unix timestamp.
    #[serde(default)]
    pub time_bounded_grants: Vec<TimeBoundedGrant>,
}

/// A temporary tool grant that is active until `until_unix_secs`.
#[derive(Debug, Clone, Deserialize)]
pub struct TimeBoundedGrant {
    /// Tool name this grant covers.
    pub tool: String,
    /// Grant is active while `SystemTime::now() < UNIX_EPOCH + until_unix_secs`.
    pub until_unix_secs: u64,
}

// ---------------------------------------------------------------------------
// Compiled (ready-to-query) per-tenant allowlist
// ---------------------------------------------------------------------------

struct CompiledAllowlist {
    allowed_tools: HashSet<String>,
    denied_tools: HashSet<String>,
    /// Sorted lexicographically; trailing `*` stripped.
    resource_prefixes: Vec<String>,
    /// Sorted by expiry ascending for binary-search skip.
    time_bounded_grants: Vec<(SystemTime, String)>,
}

impl CompiledAllowlist {
    fn from_config(cfg: &TenantAllowlist) -> Self {
        let mut prefixes: Vec<String> = cfg
            .allowed_resource_prefixes
            .iter()
            .map(|p| p.trim_end_matches('*').to_owned())
            .collect();
        prefixes.sort_unstable();

        let mut grants: Vec<(SystemTime, String)> = cfg
            .time_bounded_grants
            .iter()
            .map(|g| {
                let expiry =
                    SystemTime::UNIX_EPOCH + Duration::from_secs(g.until_unix_secs);
                (expiry, g.tool.clone())
            })
            .collect();
        grants.sort_unstable_by_key(|(t, _)| *t);

        Self {
            allowed_tools: cfg.allowed_tools.iter().cloned().collect(),
            denied_tools: cfg.denied_tools.iter().cloned().collect(),
            resource_prefixes: prefixes,
            time_bounded_grants: grants,
        }
    }

    /// Returns `Some(denial_reason)` if the tool should be denied, `None` if allowed.
    fn check_tool(&self, tool: &str, now: SystemTime) -> Option<DenialReason> {
        if self.denied_tools.contains(tool) {
            return Some(DenialReason::InDenylist);
        }
        if self.allowed_tools.contains(tool) {
            return None;
        }
        if self.has_active_grant(tool, now) {
            return None;
        }
        Some(DenialReason::NotInAllowlist)
    }

    /// Returns `Some(denial_reason)` if the resource URI should be denied.
    fn check_resource(&self, uri: &str) -> Option<DenialReason> {
        if self.resource_prefix_matches(uri) {
            None
        } else {
            Some(DenialReason::NotInAllowlist)
        }
    }

    // O(log n + prefix_len) prefix match over the sorted prefix vec.
    fn resource_prefix_matches(&self, uri: &str) -> bool {
        if self.resource_prefixes.is_empty() {
            return false;
        }
        let pos = self.resource_prefixes.partition_point(|p| p.as_str() <= uri);
        (pos > 0 && uri.starts_with(&self.resource_prefixes[pos - 1]))
            || (pos < self.resource_prefixes.len()
                && uri.starts_with(&self.resource_prefixes[pos]))
    }

    // Binary-search past expired grants, then scan active ones for a name match.
    fn has_active_grant(&self, tool: &str, now: SystemTime) -> bool {
        let active_start = self
            .time_bounded_grants
            .partition_point(|(expiry, _)| *expiry <= now);
        self.time_bounded_grants[active_start..]
            .iter()
            .any(|(_, t)| t == tool)
    }
}

// ---------------------------------------------------------------------------
// AllowlistBroker
// ---------------------------------------------------------------------------

/// Production capability broker: per-tenant allowlist with WAL-backed audit.
///
/// Implements the gateway-as-broker posture from ADR 0015. One instance per
/// gateway process, shared via `Arc<dyn CapabilityBroker>`.
pub struct AllowlistBroker {
    tenants: HashMap<u32, CompiledAllowlist>,
    signing_key: SigningKey,
    wal: Arc<dyn WAL>,
}

impl AllowlistBroker {
    /// Construct from per-tenant config, a signing key, and an open WAL.
    ///
    /// Tenant ids are the numeric `TenantId` values used by the rest of the
    /// kernel. Config is compiled into ready-to-query form at construction time.
    pub fn new(
        configs: &HashMap<u32, TenantAllowlist>,
        signing_key: SigningKey,
        wal: Arc<dyn WAL>,
    ) -> Self {
        let tenants = configs
            .iter()
            .map(|(&tid, cfg)| (tid, CompiledAllowlist::from_config(cfg)))
            .collect();
        Self { tenants, signing_key, wal }
    }
}

impl CapabilityBroker for AllowlistBroker {
    fn authorize(
        &self,
        request: &McpRequest,
        identity: &TenantIdentity,
    ) -> CapabilityDecision {
        let now = SystemTime::now();

        let allowlist = match self.tenants.get(&identity.tenant.0) {
            Some(a) => a,
            None => {
                emit_audit(request, identity, AuditDecision::Deny, now, &self.wal);
                return CapabilityDecision::Deny {
                    reason: DenialReason::TenantUnknown,
                };
            }
        };

        match evaluate(allowlist, request, now) {
            Some(reason) => {
                emit_audit(request, identity, AuditDecision::Deny, now, &self.wal);
                CapabilityDecision::Deny { reason }
            }
            None => {
                let subject = request_subject(request);
                let attestation = sign(identity.tenant, &subject, &self.signing_key);
                emit_audit(request, identity, AuditDecision::Allow, now, &self.wal);
                CapabilityDecision::Allow { attestation }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn evaluate(
    allowlist: &CompiledAllowlist,
    request: &McpRequest,
    now: SystemTime,
) -> Option<DenialReason> {
    match request {
        McpRequest::ToolCall { tool, .. } => allowlist.check_tool(tool.as_str(), now),
        McpRequest::ResourceRead { resource } => allowlist.check_resource(resource.as_str()),
        // List and lifecycle requests pass through without allowlist checks.
        McpRequest::ToolList
        | McpRequest::ResourceList
        | McpRequest::PromptList
        | McpRequest::PromptGet { .. }
        | McpRequest::Lifecycle { .. } => None,
    }
}

fn request_subject(request: &McpRequest) -> String {
    match request {
        McpRequest::ToolCall { tool, .. } => tool.0.clone(),
        McpRequest::ResourceRead { resource } => resource.0.clone(),
        McpRequest::PromptGet { name } => name.clone(),
        McpRequest::ToolList => "tools/list".to_owned(),
        McpRequest::ResourceList => "resources/list".to_owned(),
        McpRequest::PromptList => "prompts/list".to_owned(),
        McpRequest::Lifecycle { method } => match method {
            LifecycleMethod::Initialize => "initialize".to_owned(),
            LifecycleMethod::Ping => "ping".to_owned(),
            LifecycleMethod::Shutdown => "shutdown".to_owned(),
        },
    }
}

fn emit_audit(
    request: &McpRequest,
    identity: &TenantIdentity,
    decision: AuditDecision,
    now: SystemTime,
    wal: &Arc<dyn WAL>,
) {
    let argument_hash = match request {
        McpRequest::ToolCall { argument_hash, .. } => *argument_hash,
        _ => [0u8; 32],
    };
    let event = McpAuditEvent {
        correlation_id: RequestId::next(),
        tenant: identity.tenant,
        subject: request_subject(request),
        argument_hash,
        decision,
        timestamp: now,
    };
    if let Err(e) = audit::write(&event, wal.as_ref()) {
        warn!(error = %e, "MCP audit WAL write failed; decision was logged to stderr only");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::capability::{CapabilityBroker, ToolId};
    use riftgate_core::types::TenantId;
    use riftgate_core::wal::WalEntryId;

    struct NoopWal;
    impl WAL for NoopWal {
        fn append(&self, _: &[u8], _: riftgate_core::wal::Durability) -> std::io::Result<WalEntryId> {
            Ok(WalEntryId(0))
        }
        fn flush(&self, _: Duration) -> std::io::Result<()> {
            Ok(())
        }
        fn last_durable(&self) -> Option<WalEntryId> {
            None
        }
    }

    fn broker(cfg: HashMap<u32, TenantAllowlist>) -> AllowlistBroker {
        AllowlistBroker::new(&cfg, SigningKey([0u8; 32]), Arc::new(NoopWal))
    }

    fn identity(tid: u32) -> TenantIdentity {
        TenantIdentity { tenant: TenantId(tid), principal: "test".to_owned() }
    }

    #[test]
    fn allowed_tool_returns_allow() {
        let mut cfg = HashMap::new();
        cfg.insert(1, TenantAllowlist {
            allowed_tools: vec!["search-web".to_owned()],
            ..Default::default()
        });
        let b = broker(cfg);
        let req = McpRequest::ToolCall {
            tool: ToolId::from("search-web"),
            argument_hash: [0u8; 32],
        };
        assert!(matches!(b.authorize(&req, &identity(1)), CapabilityDecision::Allow { .. }));
    }

    #[test]
    fn denied_tool_returns_deny() {
        let mut cfg = HashMap::new();
        cfg.insert(1, TenantAllowlist {
            allowed_tools: vec!["search-web".to_owned()],
            denied_tools: vec!["filesystem-write".to_owned()],
            ..Default::default()
        });
        let b = broker(cfg);
        let req = McpRequest::ToolCall {
            tool: ToolId::from("filesystem-write"),
            argument_hash: [0u8; 32],
        };
        assert!(matches!(
            b.authorize(&req, &identity(1)),
            CapabilityDecision::Deny { reason: DenialReason::InDenylist }
        ));
    }

    #[test]
    fn tool_not_in_allowlist_returns_deny() {
        let mut cfg = HashMap::new();
        cfg.insert(1, TenantAllowlist { ..Default::default() });
        let b = broker(cfg);
        let req = McpRequest::ToolCall {
            tool: ToolId::from("unknown-tool"),
            argument_hash: [0u8; 32],
        };
        assert!(matches!(
            b.authorize(&req, &identity(1)),
            CapabilityDecision::Deny { reason: DenialReason::NotInAllowlist }
        ));
    }

    #[test]
    fn unknown_tenant_returns_deny() {
        let b = broker(HashMap::new());
        let req = McpRequest::ToolList;
        assert!(matches!(
            b.authorize(&req, &identity(99)),
            CapabilityDecision::Deny { reason: DenialReason::TenantUnknown }
        ));
    }

    #[test]
    fn resource_prefix_match_allows() {
        let mut cfg = HashMap::new();
        cfg.insert(1, TenantAllowlist {
            allowed_resource_prefixes: vec!["s3://acme-datasets/*".to_owned()],
            ..Default::default()
        });
        let b = broker(cfg);
        let req = McpRequest::ResourceRead {
            resource: riftgate_core::capability::ResourceId::from("s3://acme-datasets/train.jsonl"),
        };
        assert!(matches!(b.authorize(&req, &identity(1)), CapabilityDecision::Allow { .. }));
    }

    #[test]
    fn resource_outside_prefix_denies() {
        let mut cfg = HashMap::new();
        cfg.insert(1, TenantAllowlist {
            allowed_resource_prefixes: vec!["s3://acme-datasets/*".to_owned()],
            ..Default::default()
        });
        let b = broker(cfg);
        let req = McpRequest::ResourceRead {
            resource: riftgate_core::capability::ResourceId::from("s3://other-bucket/data"),
        };
        assert!(matches!(
            b.authorize(&req, &identity(1)),
            CapabilityDecision::Deny { reason: DenialReason::NotInAllowlist }
        ));
    }

    #[test]
    fn time_bounded_grant_allows_before_expiry() {
        let far_future = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 86400;
        let mut cfg = HashMap::new();
        cfg.insert(1, TenantAllowlist {
            time_bounded_grants: vec![TimeBoundedGrant {
                tool: "send-email".to_owned(),
                until_unix_secs: far_future,
            }],
            ..Default::default()
        });
        let b = broker(cfg);
        let req = McpRequest::ToolCall {
            tool: ToolId::from("send-email"),
            argument_hash: [0u8; 32],
        };
        assert!(matches!(b.authorize(&req, &identity(1)), CapabilityDecision::Allow { .. }));
    }

    #[test]
    fn expired_time_bounded_grant_denies() {
        let past = 1_000u64; // well in the past
        let mut cfg = HashMap::new();
        cfg.insert(1, TenantAllowlist {
            time_bounded_grants: vec![TimeBoundedGrant {
                tool: "send-email".to_owned(),
                until_unix_secs: past,
            }],
            ..Default::default()
        });
        let b = broker(cfg);
        let req = McpRequest::ToolCall {
            tool: ToolId::from("send-email"),
            argument_hash: [0u8; 32],
        };
        assert!(matches!(
            b.authorize(&req, &identity(1)),
            CapabilityDecision::Deny { reason: DenialReason::NotInAllowlist }
        ));
    }

    #[test]
    fn lifecycle_and_list_requests_always_pass() {
        let mut cfg = HashMap::new();
        cfg.insert(1, TenantAllowlist { ..Default::default() });
        let b = broker(cfg);
        let id = identity(1);
        for req in [
            McpRequest::ToolList,
            McpRequest::ResourceList,
            McpRequest::PromptList,
            McpRequest::Lifecycle { method: LifecycleMethod::Initialize },
            McpRequest::Lifecycle { method: LifecycleMethod::Ping },
        ] {
            assert!(
                matches!(b.authorize(&req, &id), CapabilityDecision::Allow { .. }),
                "expected Allow for {req:?}"
            );
        }
    }
}
