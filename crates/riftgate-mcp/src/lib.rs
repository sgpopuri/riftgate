// riftgate-mcp/src/lib.rs
//
// MCP capability broker: parser, allowlist enforcement, WAL audit, HMAC attestation.
//
// This crate implements the gateway-as-broker posture from
// ADR 0015 (docs/06-adrs/0015-mcp-extension-plane-broker.md):
// parse every MCP request, check it against a per-tenant allowlist, write a
// durable audit event, and return either Allow (with signed attestation headers)
// or Deny (with a typed reason).
//
// Crate layout:
//   parser      -- JSON-RPC 2.0 + MCP message parsing (bytes -> McpRequest)
//   allowlist   -- AllowlistBroker: per-tenant bit-set + prefix-set + time-bounded grants
//   dryrun      -- DryRunBroker: wraps any broker; logs would-be denials, always passes
//   attestation -- HMAC-SHA256 signing key and attestation header generation
//   audit       -- serialize McpAuditEvent as NDJSON and append to the WAL

//! MCP capability broker for the Riftgate gateway.
//!
//! Implements the gateway-as-broker posture from
//! [ADR 0015](../../docs/06-adrs/0015-mcp-extension-plane-broker.md):
//! parse every MCP request, authorize it against a per-tenant allowlist, write
//! a durable audit event to the WAL, and return either `Allow` (with
//! HMAC-signed attestation headers) or `Deny` (with a typed reason).

#![warn(missing_docs)]

/// Per-tenant allowlist enforcement: bit-set tools, prefix-set resources, time-bounded grants.
pub mod allowlist;
/// HMAC-SHA256 signing key and attestation header generation/verification.
pub mod attestation;
/// Serialize [`riftgate_core::capability::McpAuditEvent`] as NDJSON and append to the WAL.
pub mod audit;
/// Dry-run wrapper: logs would-be denials but always returns `Allow`.
pub mod dryrun;
/// JSON-RPC 2.0 + MCP message parser (`bytes -> McpRequest`).
pub mod parser;

pub use allowlist::{AllowlistBroker, TenantAllowlist, TimeBoundedGrant};
pub use attestation::SigningKey;
pub use dryrun::DryRunBroker;
pub use parser::{parse, ParseError};
