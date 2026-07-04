// riftgate-mcp/src/audit.rs
//
// Serialize McpAuditEvent as newline-delimited JSON and append to the WAL.
//
// Every authorize() call — allow or deny — must produce a durable audit record
// (ADR 0015, NFR-OBS07). The WAL is appended with Durability::Fsync so the
// record survives a crash before the response is sent.
//
// Argument bytes are never stored; only the SHA-256 hash appears in the record
// to keep PII out of the audit log while preserving forensic traceability.

use std::time::SystemTime;

use riftgate_core::capability::{AuditDecision, McpAuditEvent};
use riftgate_core::wal::{Durability, WAL};
use serde::Serialize;

/// Serialize `event` as a newline-terminated JSON record and append to `wal`
/// with `Durability::Fsync`.
///
/// WAL errors are returned to the caller; it is the broker's responsibility
/// to decide whether to propagate or log-and-continue.
pub fn write(event: &McpAuditEvent, wal: &dyn WAL) -> std::io::Result<()> {
    let record = AuditRecord {
        correlation_id: format!("{}", event.correlation_id),
        tenant: event.tenant.0,
        subject: &event.subject,
        argument_hash: hex_encode(&event.argument_hash),
        decision: match event.decision {
            AuditDecision::Allow => "allow",
            AuditDecision::Deny => "deny",
        },
        timestamp_unix_secs: event
            .timestamp
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    let mut bytes = serde_json::to_vec(&record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    bytes.push(b'\n');
    wal.append(&bytes, Durability::Fsync)?;
    Ok(())
}

/// Inline hex encoder — avoids a hex crate dependency.
fn hex_encode(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[derive(Serialize)]
struct AuditRecord<'a> {
    correlation_id: String,
    tenant: u32,
    subject: &'a str,
    argument_hash: String,
    decision: &'static str,
    timestamp_unix_secs: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use riftgate_core::capability::AuditDecision;
    use riftgate_core::types::{RequestId, TenantId};

    struct VecWal(std::sync::Mutex<Vec<Vec<u8>>>);

    impl WAL for VecWal {
        fn append(
            &self,
            bytes: &[u8],
            _d: Durability,
        ) -> std::io::Result<riftgate_core::wal::WalEntryId> {
            self.0.lock().unwrap().push(bytes.to_vec());
            Ok(riftgate_core::wal::WalEntryId(0))
        }
        fn flush(&self, _t: std::time::Duration) -> std::io::Result<()> {
            Ok(())
        }
        fn last_durable(&self) -> Option<riftgate_core::wal::WalEntryId> {
            None
        }
    }

    #[test]
    fn write_produces_valid_ndjson() {
        let wal = VecWal(std::sync::Mutex::new(Vec::new()));
        let event = McpAuditEvent {
            correlation_id: RequestId::next(),
            tenant: TenantId(7),
            subject: "search-web".to_owned(),
            argument_hash: [0xFFu8; 32],
            decision: AuditDecision::Allow,
            timestamp: SystemTime::now(),
        };
        write(&event, &wal).unwrap();
        let records = wal.0.lock().unwrap();
        assert_eq!(records.len(), 1);
        let line = std::str::from_utf8(&records[0]).unwrap();
        assert!(line.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(v["tenant"], 7);
        assert_eq!(v["subject"], "search-web");
        assert_eq!(v["decision"], "allow");
        assert_eq!(v["argument_hash"].as_str().unwrap().len(), 64);
    }
}
