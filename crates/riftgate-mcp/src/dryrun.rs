// riftgate-mcp/src/dryrun.rs
//
// DryRunBroker: wraps any CapabilityBroker; logs would-be denials but always
// returns Allow. Enabled via `enforce = false` in the MCP config section.
//
// Operators use dry-run mode to calibrate allowlists before flipping to
// enforce. The wrapped inner broker still writes audit events (with the
// true would-be decision) so the audit log reflects policy intent, not
// the permissive override.

use riftgate_core::capability::{
    AttestationHeaders, CapabilityBroker, CapabilityDecision, HmacSignature, McpRequest,
    TenantIdentity,
};
use tracing::info;

/// Dry-run wrapper: always returns `Allow`, regardless of the inner decision.
///
/// The inner broker is still called and still writes the audit event with the
/// real would-be outcome. This ensures the audit log is accurate even in dry-run
/// mode — operators see what *would* have been denied.
pub struct DryRunBroker<B: CapabilityBroker> {
    inner: B,
}

impl<B: CapabilityBroker> DryRunBroker<B> {
    /// Wrap any `CapabilityBroker` in dry-run mode.
    pub fn new(inner: B) -> Self {
        Self { inner }
    }
}

impl<B: CapabilityBroker> CapabilityBroker for DryRunBroker<B> {
    fn authorize(
        &self,
        request: &McpRequest,
        identity: &TenantIdentity,
    ) -> CapabilityDecision {
        let inner_decision = self.inner.authorize(request, identity);

        match inner_decision {
            allow @ CapabilityDecision::Allow { .. } => allow,
            CapabilityDecision::Deny { ref reason } => {
                info!(
                    tenant = identity.tenant.0,
                    principal = %identity.principal,
                    would_deny_reason = reason.as_header_value(),
                    "MCP dry-run: deny suppressed; request will pass"
                );
                // Synthesize an Allow with a zero signature to signal dry-run.
                // The zero signature is detectable by a verifying downstream server;
                // in dry-run mode, downstream policy must not depend on signature validity.
                CapabilityDecision::Allow {
                    attestation: AttestationHeaders {
                        caller: identity.tenant,
                        subject: request_subject(request),
                        decision: "allow-dry-run",
                        signature: HmacSignature([0u8; 32]),
                    },
                }
            }
        }
    }
}

fn request_subject(request: &McpRequest) -> String {
    use riftgate_core::capability::LifecycleMethod;
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

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::capability::{
        CapabilityDecision, DenialReason,
    };
    use riftgate_core::types::TenantId;

    struct AlwaysDeny;
    impl CapabilityBroker for AlwaysDeny {
        fn authorize(&self, _: &McpRequest, _: &TenantIdentity) -> CapabilityDecision {
            CapabilityDecision::Deny { reason: DenialReason::NotInAllowlist }
        }
    }

    struct AlwaysAllow;
    impl CapabilityBroker for AlwaysAllow {
        fn authorize(&self, _: &McpRequest, _: &TenantIdentity) -> CapabilityDecision {
            CapabilityDecision::Allow {
                attestation: AttestationHeaders {
                    caller: TenantId(0),
                    subject: "tool".to_owned(),
                    decision: "allow",
                    signature: HmacSignature([1u8; 32]),
                },
            }
        }
    }

    fn identity() -> TenantIdentity {
        TenantIdentity { tenant: TenantId(1), principal: "test".to_owned() }
    }

    #[test]
    fn deny_is_overridden_to_allow() {
        let b = DryRunBroker::new(AlwaysDeny);
        let req = McpRequest::ToolList;
        assert!(matches!(b.authorize(&req, &identity()), CapabilityDecision::Allow { .. }));
    }

    #[test]
    fn dry_run_allow_carries_zero_signature() {
        let b = DryRunBroker::new(AlwaysDeny);
        let req = McpRequest::ToolCall {
            tool: riftgate_core::capability::ToolId::from("blocked"),
            argument_hash: [0u8; 32],
        };
        match b.authorize(&req, &identity()) {
            CapabilityDecision::Allow { attestation } => {
                assert_eq!(attestation.decision, "allow-dry-run");
                assert_eq!(attestation.signature.0, [0u8; 32]);
            }
            other => panic!("expected Allow, got {other:?}"),
        }
    }

    #[test]
    fn real_allow_passes_through_unchanged() {
        let b = DryRunBroker::new(AlwaysAllow);
        let req = McpRequest::ToolList;
        match b.authorize(&req, &identity()) {
            CapabilityDecision::Allow { attestation } => {
                assert_eq!(attestation.decision, "allow");
                assert_eq!(attestation.signature.0, [1u8; 32]);
            }
            other => panic!("expected Allow, got {other:?}"),
        }
    }
}
