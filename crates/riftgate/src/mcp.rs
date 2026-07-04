// riftgate/src/mcp.rs
//
// Build the MCP capability broker from the loaded gateway config.
// Called once at binary startup; the result is Arc'd into HandlerState.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use riftgate_config::Config;
use riftgate_core::capability::CapabilityBroker;
use riftgate_core::wal::{Durability, WAL, WalEntryId};
use riftgate_mcp::{AllowlistBroker, DryRunBroker, SigningKey, TenantAllowlist, TimeBoundedGrant};
use riftgate_replay::{FileWal, FileWalConfig};
use tracing::{info, warn};

/// Build a capability broker from `config.mcp`.
///
/// Returns `None` when no tenants are enrolled under `[mcp.tenants]`.
/// Returns `Some(DryRunBroker<AllowlistBroker>)` when `enforce = false`.
pub fn build_mcp_broker(config: &Config) -> Option<Arc<dyn CapabilityBroker>> {
    let mcp = &config.mcp;
    if mcp.tenants.is_empty() {
        info!("MCP broker disabled (no tenants configured under [mcp.tenants])");
        return None;
    }

    let signing_key = match &mcp.signing_key_hex {
        Some(hex) => SigningKey(parse_hex_key(hex)),
        None => {
            warn!(
                "mcp.signing_key_hex not set; generating an ephemeral random key \
                 (not persistent across restarts; pin the key in production)"
            );
            SigningKey(rand::random::<[u8; 32]>())
        }
    };

    // WAL: use FileWal when mcp.wal_path is configured, NoopWal otherwise.
    let wal: Arc<dyn WAL> = match &mcp.wal_path {
        Some(path) => {
            let cfg = FileWalConfig {
                root: path.into(),
                shards: 1,
                ..Default::default()
            };
            match FileWal::open(cfg) {
                Ok(w) => {
                    info!(path, "MCP audit WAL opened (FileWal)");
                    w
                }
                Err(e) => {
                    warn!(path, error = %e, "failed to open MCP audit WAL; falling back to NoopWal");
                    Arc::new(NoopWal)
                }
            }
        }
        None => {
            info!(
                "mcp.wal_path not set; audit events computed but not persisted \
                 (set mcp.wal_path in config for durable audit)"
            );
            Arc::new(NoopWal)
        }
    };

    // Map tenant keys to u32 TenantIds. Numeric keys ("1") parse directly;
    // name keys ("acme") are hashed with FNV-1a for a deterministic stable id.
    let tenant_configs: HashMap<u32, TenantAllowlist> = mcp
        .tenants
        .iter()
        .map(|(k, v)| {
            let id = tenant_id_from_key(k);
            let al = TenantAllowlist {
                allowed_tools: v.allowed_tools.clone(),
                denied_tools: v.denied_tools.clone(),
                allowed_resource_prefixes: v.allowed_resource_prefixes.clone(),
                time_bounded_grants: v
                    .time_bounded_grants
                    .iter()
                    .map(|g| TimeBoundedGrant {
                        tool: g.tool.clone(),
                        until_unix_secs: g.until_unix_secs,
                    })
                    .collect(),
            };
            (id, al)
        })
        .collect();

    let mode = if mcp.enforce { "enforce" } else { "dry-run" };
    info!(
        tenants = tenant_configs.len(),
        mode, "MCP capability broker enabled"
    );

    let inner = AllowlistBroker::new(&tenant_configs, signing_key, wal);
    Some(if mcp.enforce {
        Arc::new(inner) as Arc<dyn CapabilityBroker>
    } else {
        Arc::new(DryRunBroker::new(inner)) as Arc<dyn CapabilityBroker>
    })
}

/// Map a config tenant key to a `u32` `TenantId`.
///
/// Numeric keys (`"1"`) are used directly. Non-numeric names (`"acme"`) are
/// hashed with FNV-1a 32-bit for a deterministic stable id. Collision
/// probability is 1/2^32 per distinct name pair.
fn tenant_id_from_key(key: &str) -> u32 {
    if let Ok(n) = key.parse::<u32>() {
        return n;
    }
    let mut hash: u32 = 2_166_136_261;
    for b in key.bytes() {
        hash ^= u32::from(b);
        hash = hash.wrapping_mul(16_777_619);
    }
    if hash == 0 { 1 } else { hash }
}

/// Parse a 64-char hex string into a 32-byte signing key.
fn parse_hex_key(hex: &str) -> [u8; 32] {
    let hex = hex.trim();
    if hex.len() != 64 {
        warn!(
            len = hex.len(),
            "mcp.signing_key_hex must be 64 hex chars; using zero key"
        );
        return [0u8; 32];
    }
    let mut key = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        match std::str::from_utf8(chunk)
            .ok()
            .and_then(|s| u8::from_str_radix(s, 16).ok())
        {
            Some(b) => key[i] = b,
            None => {
                warn!("mcp.signing_key_hex has non-hex chars; using zero key");
                return [0u8; 32];
            }
        }
    }
    key
}

/// No-op WAL — discards all append calls.
/// Used when `mcp.wal_path` is not set.
struct NoopWal;

impl WAL for NoopWal {
    fn append(&self, _: &[u8], _: Durability) -> std::io::Result<WalEntryId> {
        Ok(WalEntryId(0))
    }
    fn flush(&self, _: Duration) -> std::io::Result<()> {
        Ok(())
    }
    fn last_durable(&self) -> Option<WalEntryId> {
        None
    }
}
