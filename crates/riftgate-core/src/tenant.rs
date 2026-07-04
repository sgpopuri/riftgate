//! `TenantResolver` trait — per-request tenant identity resolution.
//!
//! Per [ADR 0029](../../../docs/06-adrs/0029-api-key-tenant-resolver.md):
//! maps an incoming request's HTTP headers to a [`TenantIdentity`] so the
//! capability broker, rate limiter, and observability pipeline can apply
//! per-tenant policy without trusting raw, unvalidated header values.
//!
//! ## Implementations
//!
//! | Impl | Crate | When to use |
//! |------|-------|-------------|
//! | [`HeaderTenantResolver`] | `riftgate-core` | Trusted internal mesh; backwards-compatible v0.5 mode |
//! | `ApiKeyTenantResolver` | `riftgate-config` | Production default; validates `Authorization: Bearer <key>` |
//! | `JwtTenantResolver` | future | SSO / short-lived tokens |

use crate::capability::TenantIdentity;
use crate::types::TenantId;
use http::HeaderMap;

/// Resolve a request's tenant identity from its HTTP headers.
///
/// **`Send + Sync`** — one resolver per process, shared via `Arc`.
///
/// Returns `None` when:
/// - No recognizable credential is present.
/// - The credential is present but invalid (unknown key, expired token, etc.).
///
/// Returning `None` causes the proxy to respond with `401 Unauthorized`.
///
/// Trait object safety: yes.
pub trait TenantResolver: Send + Sync {
    /// Attempt to resolve the tenant identity from `headers`.
    fn resolve(&self, headers: &HeaderMap) -> Option<TenantIdentity>;
}

// ---------------------------------------------------------------------------
// HeaderTenantResolver — trusted-network / v0.5 compatibility mode
// ---------------------------------------------------------------------------

/// Reads `x-riftgate-tenant: <value>` directly as the tenant identity.
///
/// `<value>` may be:
/// - A decimal `u32` (used as-is as the `TenantId`).
/// - A non-numeric string (hashed via FNV-1a 32-bit to derive a stable `TenantId`).
///
/// Falls back to `TenantId(0)` when the header is absent.
///
/// **Only safe on trusted networks where all callers are already authenticated
/// by a mesh or sidecar before reaching Riftgate.** Do not use as the default
/// on internet-facing deployments.
///
/// Enable via `[multitenancy] mode = "trusted-header"` in the gateway config.
pub struct HeaderTenantResolver;

impl TenantResolver for HeaderTenantResolver {
    fn resolve(&self, headers: &HeaderMap) -> Option<TenantIdentity> {
        let raw = headers
            .get("x-riftgate-tenant")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("0");
        let tenant = TenantId(tenant_id_from_str(raw));
        Some(TenantIdentity {
            tenant,
            principal: raw.to_owned(),
        })
    }
}

/// Map a string key to a `u32` `TenantId`.
///
/// Numeric strings (`"1"`, `"42"`) parse directly. Non-numeric strings
/// (`"acme"`) are hashed with FNV-1a 32-bit for a deterministic stable id.
/// Mirrors the `mcp.rs` `tenant_id_from_key` logic so MCP config and
/// tenant resolver config share a consistent name-to-id mapping.
pub fn tenant_id_from_str(s: &str) -> u32 {
    if let Ok(n) = s.parse::<u32>() {
        return n;
    }
    let mut hash: u32 = 2_166_136_261;
    for b in s.bytes() {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(16_777_619);
    }
    if hash == 0 { 1 } else { hash }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderMap;

    fn headers_with(key: &str, val: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            http::HeaderName::from_bytes(key.as_bytes()).unwrap(),
            http::HeaderValue::from_str(val).unwrap(),
        );
        h
    }

    #[test]
    fn header_resolver_numeric_key() {
        let r = HeaderTenantResolver;
        let h = headers_with("x-riftgate-tenant", "42");
        assert_eq!(r.resolve(&h).unwrap().tenant, TenantId(42));
    }

    #[test]
    fn header_resolver_named_key_is_deterministic() {
        let r = HeaderTenantResolver;
        let h1 = headers_with("x-riftgate-tenant", "acme");
        let h2 = headers_with("x-riftgate-tenant", "acme");
        let id1 = r.resolve(&h1).unwrap().tenant;
        let id2 = r.resolve(&h2).unwrap().tenant;
        assert_eq!(id1, id2);
        assert_ne!(id1, TenantId(0));
    }

    #[test]
    fn header_resolver_absent_header_returns_zero_tenant() {
        let r = HeaderTenantResolver;
        let id = r.resolve(&HeaderMap::new()).unwrap().tenant;
        assert_eq!(id, TenantId(0));
    }

    #[test]
    fn tenant_id_from_str_numeric() {
        assert_eq!(tenant_id_from_str("7"), 7);
    }

    #[test]
    fn tenant_id_from_str_nonzero_for_name() {
        // FNV-1a hash of any name should produce a non-zero id.
        assert_ne!(tenant_id_from_str("acme"), 0);
        assert_ne!(tenant_id_from_str("bigcorp"), 0);
    }

    #[test]
    fn tenant_id_from_str_different_names_differ() {
        assert_ne!(tenant_id_from_str("acme"), tenant_id_from_str("bigcorp"));
    }
}
