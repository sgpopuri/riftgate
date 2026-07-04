// riftgate-mcp/src/attestation.rs
//
// HMAC-SHA256 signing key and attestation header generation.
//
// The gateway signs each allowed MCP decision with a per-process HMAC key so
// downstream MCP servers can independently verify the authorization decision
// (defense-in-depth per ADR 0015).
//
// The signing key is loaded from config at startup and never logged or exposed
// on the wire. For verification, the downstream server needs the same key
// (shared secret model; key rotation is a v1.0+ concern per ADR 0015 Notes).

use hmac::{Hmac, Mac};
use riftgate_core::capability::{AttestationHeaders, HmacSignature};
use riftgate_core::types::TenantId;
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// HMAC-SHA256 signing key. Loaded once at startup from gateway config.
///
/// Never logged or included in error messages. Clone is intentional:
/// `AllowlistBroker` owns one instance by value.
#[derive(Clone)]
pub struct SigningKey(pub [u8; 32]);

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SigningKey([redacted])")
    }
}

/// Compute HMAC-SHA256 attestation headers for an allowed MCP request.
///
/// The message signed is `tenant_id_le || "|" || subject || "|" || decision`.
/// The `decision` is always `"allow"` for non-dry-run attestations.
pub fn sign(caller: TenantId, subject: &str, key: &SigningKey) -> AttestationHeaders {
    let mut mac =
        HmacSha256::new_from_slice(&key.0).expect("HMAC accepts keys of any length");
    mac.update(&caller.0.to_le_bytes());
    mac.update(b"|");
    mac.update(subject.as_bytes());
    mac.update(b"|allow");
    let result: [u8; 32] = mac.finalize().into_bytes().into();
    AttestationHeaders {
        caller,
        subject: subject.to_owned(),
        decision: "allow",
        signature: HmacSignature(result),
    }
}

/// Verify an attestation header against the given signing key.
///
/// Returns `true` if the signature is valid. Used in downstream server
/// harnesses and integration tests; not called on the hot path.
pub fn verify(headers: &AttestationHeaders, key: &SigningKey) -> bool {
    let mut mac =
        HmacSha256::new_from_slice(&key.0).expect("HMAC accepts keys of any length");
    mac.update(&headers.caller.0.to_le_bytes());
    mac.update(b"|");
    mac.update(headers.subject.as_bytes());
    mac.update(b"|");
    mac.update(headers.decision.as_bytes());
    mac.verify_slice(&headers.signature.0).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::types::TenantId;

    #[test]
    fn sign_then_verify_round_trip() {
        let key = SigningKey([0xABu8; 32]);
        let caller = TenantId(42);
        let headers = sign(caller, "search-web", &key);
        assert_eq!(headers.decision, "allow");
        assert_eq!(headers.subject, "search-web");
        assert!(verify(&headers, &key));
    }

    #[test]
    fn wrong_key_fails_verification() {
        let key1 = SigningKey([0x01u8; 32]);
        let key2 = SigningKey([0x02u8; 32]);
        let headers = sign(TenantId(1), "search-web", &key1);
        assert!(!verify(&headers, &key2));
    }

    #[test]
    fn different_subjects_produce_different_signatures() {
        let key = SigningKey([0xCDu8; 32]);
        let h1 = sign(TenantId(1), "tool-a", &key);
        let h2 = sign(TenantId(1), "tool-b", &key);
        assert_ne!(h1.signature.0, h2.signature.0);
    }
}
