//! WAL audit round-trip integration tests for the MCP capability broker.
//!
//! Verifies that every `authorize()` call writes exactly one NDJSON audit
//! record to the WAL, that the records contain the correct fields, and that
//! allowed decisions carry valid attestation headers.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use riftgate_core::capability::{CapabilityBroker, CapabilityDecision, McpRequest, ToolId};
use riftgate_core::types::TenantId;
use riftgate_core::wal::{Durability, WAL, WalEntryId};
use riftgate_mcp::{AllowlistBroker, SigningKey, TenantAllowlist};

// In-memory WAL stub that collects appended bytes for inspection.
struct MemWal(Mutex<Vec<Vec<u8>>>);

impl WAL for MemWal {
    fn append(&self, bytes: &[u8], _: Durability) -> std::io::Result<WalEntryId> {
        let mut guard = self.0.lock().unwrap();
        let id = guard.len() as u64;
        guard.push(bytes.to_vec());
        Ok(WalEntryId(id))
    }
    fn flush(&self, _: Duration) -> std::io::Result<()> {
        Ok(())
    }
    fn last_durable(&self) -> Option<WalEntryId> {
        let guard = self.0.lock().unwrap();
        guard.len().checked_sub(1).map(|i| WalEntryId(i as u64))
    }
}

fn setup_broker(wal: Arc<MemWal>) -> AllowlistBroker {
    let mut tenants = HashMap::new();
    tenants.insert(
        1u32,
        TenantAllowlist {
            allowed_tools: vec!["search-web".to_owned(), "read-file".to_owned()],
            denied_tools: vec!["filesystem-write".to_owned()],
            allowed_resource_prefixes: vec!["s3://acme/*".to_owned()],
            ..Default::default()
        },
    );
    AllowlistBroker::new(&tenants, SigningKey([0xABu8; 32]), wal)
}

fn identity() -> riftgate_core::capability::TenantIdentity {
    riftgate_core::capability::TenantIdentity {
        tenant: TenantId(1),
        principal: "test-principal".to_owned(),
    }
}

#[test]
fn every_authorize_call_writes_one_audit_record() {
    let wal = Arc::new(MemWal(Mutex::new(Vec::new())));
    let broker = setup_broker(wal.clone());
    let id = identity();

    let requests = vec![
        McpRequest::ToolCall {
            tool: ToolId::from("search-web"),
            argument_hash: [0u8; 32],
        },
        McpRequest::ToolCall {
            tool: ToolId::from("filesystem-write"),
            argument_hash: [0u8; 32],
        },
        McpRequest::ToolList,
    ];

    for req in &requests {
        broker.authorize(req, &id);
    }

    let records = wal.0.lock().unwrap();
    assert_eq!(records.len(), requests.len(), "expected one WAL record per authorize() call");
}

#[test]
fn audit_records_are_valid_ndjson_with_correct_decisions() {
    let wal = Arc::new(MemWal(Mutex::new(Vec::new())));
    let broker = setup_broker(wal.clone());
    let id = identity();

    let allow_req = McpRequest::ToolCall {
        tool: ToolId::from("search-web"),
        argument_hash: [0xFFu8; 32],
    };
    let deny_req = McpRequest::ToolCall {
        tool: ToolId::from("filesystem-write"),
        argument_hash: [0xEEu8; 32],
    };

    broker.authorize(&allow_req, &id);
    broker.authorize(&deny_req, &id);

    let records = wal.0.lock().unwrap();
    let first: serde_json::Value =
        serde_json::from_slice(records[0].trim_ascii_end()).unwrap();
    let second: serde_json::Value =
        serde_json::from_slice(records[1].trim_ascii_end()).unwrap();

    assert_eq!(first["decision"], "allow");
    assert_eq!(first["subject"], "search-web");
    assert_eq!(first["tenant"], 1);

    assert_eq!(second["decision"], "deny");
    assert_eq!(second["subject"], "filesystem-write");

    // argument_hash is always a 64-char hex string
    assert_eq!(first["argument_hash"].as_str().unwrap().len(), 64);
}

#[test]
fn allow_decision_carries_valid_attestation_headers() {
    let wal = Arc::new(MemWal(Mutex::new(Vec::new())));
    let broker = setup_broker(wal.clone());
    let id = identity();
    let req = McpRequest::ToolCall {
        tool: ToolId::from("read-file"),
        argument_hash: [0u8; 32],
    };

    match broker.authorize(&req, &id) {
        CapabilityDecision::Allow { attestation } => {
            assert_eq!(attestation.decision, "allow");
            assert_eq!(attestation.subject, "read-file");
            assert_eq!(attestation.caller, TenantId(1));
            // Signature must be non-zero for a real allow
            assert_ne!(attestation.signature.0, [0u8; 32]);
        }
        other => panic!("expected Allow, got {other:?}"),
    }
}
