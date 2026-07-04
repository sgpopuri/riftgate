// riftgate-mcp/src/parser.rs
//
// JSON-RPC 2.0 + MCP message parser.
//
// Converts raw request bytes into a typed McpRequest for the capability broker.
// The parser does NOT rewrite payloads (mediator posture is deferred to v1.0+
// per ADR 0015). It extracts only the fields the broker needs for authorization.
//
// SHA-256 of the arguments is computed here for the audit trail; the actual
// argument bytes never leave this module.

use riftgate_core::capability::{LifecycleMethod, McpRequest, ResourceId, ToolId};
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// JSON-RPC 2.0 request envelope (only the fields we need).
#[derive(Debug, Deserialize)]
struct JsonRpcEnvelope {
    jsonrpc: String,
    method: String,
    #[serde(default)]
    params: Value,
}

/// Error returned when an MCP request body cannot be parsed.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    /// The request body is not valid JSON.
    #[error("JSON parse error: {0}")]
    Json(#[from] serde_json::Error),
    /// The JSON-RPC version field is not "2.0".
    #[error("unexpected JSON-RPC version: expected \"2.0\", got \"{0}\"")]
    BadVersion(String),
    /// The MCP method is not recognized or not supported in this version.
    #[error("unknown or unsupported MCP method: \"{0}\"")]
    UnknownMethod(String),
    /// A required field is missing from the params object.
    #[error("missing required field \"{field}\" in {method} params")]
    MissingField {
        /// The MCP method being parsed.
        method: &'static str,
        /// The missing field name.
        field: &'static str,
    },
}

/// Parse a JSON-encoded MCP request body into a typed [`McpRequest`].
///
/// The input is expected to be a UTF-8 JSON-RPC 2.0 request body, as
/// typically carried in the HTTP request body of an MCP `POST /mcp` endpoint.
///
/// # Errors
/// Returns `ParseError` if the body is malformed, the version is wrong, or
/// the method is unknown.
pub fn parse(body: &[u8]) -> Result<McpRequest, ParseError> {
    let env: JsonRpcEnvelope = serde_json::from_slice(body)?;
    if env.jsonrpc != "2.0" {
        return Err(ParseError::BadVersion(env.jsonrpc));
    }

    match env.method.as_str() {
        "tools/call" => {
            let name = string_field(&env.params, "tools/call", "name")?;
            let args = env.params.get("arguments").cloned().unwrap_or(Value::Null);
            let argument_hash = sha256_of_value(&args);
            Ok(McpRequest::ToolCall {
                tool: ToolId::from(name),
                argument_hash,
            })
        }
        "tools/list" => Ok(McpRequest::ToolList),
        "resources/read" => {
            let uri = string_field(&env.params, "resources/read", "uri")?;
            Ok(McpRequest::ResourceRead {
                resource: ResourceId::from(uri),
            })
        }
        "resources/list" => Ok(McpRequest::ResourceList),
        "prompts/get" => {
            let name = string_field(&env.params, "prompts/get", "name")?;
            Ok(McpRequest::PromptGet { name })
        }
        "prompts/list" => Ok(McpRequest::PromptList),
        "initialize" => Ok(McpRequest::Lifecycle {
            method: LifecycleMethod::Initialize,
        }),
        "ping" => Ok(McpRequest::Lifecycle {
            method: LifecycleMethod::Ping,
        }),
        "shutdown" => Ok(McpRequest::Lifecycle {
            method: LifecycleMethod::Shutdown,
        }),
        other => Err(ParseError::UnknownMethod(other.to_owned())),
    }
}

// Extract a required string field from a JSON params object.
fn string_field(
    params: &Value,
    method: &'static str,
    field: &'static str,
) -> Result<String, ParseError> {
    params
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(ParseError::MissingField { method, field })
}

// Compute SHA-256 of a JSON value's serialized bytes.
fn sha256_of_value(v: &Value) -> [u8; 32] {
    let bytes = serde_json::to_vec(v).unwrap_or_default();
    Sha256::digest(&bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_call_round_trip() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search-web","arguments":{"query":"rust"}}}"#;
        let req = parse(body).unwrap();
        match req {
            McpRequest::ToolCall { tool, argument_hash } => {
                assert_eq!(tool.as_str(), "search-web");
                assert_ne!(argument_hash, [0u8; 32]);
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
    }

    #[test]
    fn resources_read_round_trip() {
        let body = br#"{"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"s3://bucket/key"}}"#;
        let req = parse(body).unwrap();
        match req {
            McpRequest::ResourceRead { resource } => {
                assert_eq!(resource.as_str(), "s3://bucket/key");
            }
            other => panic!("expected ResourceRead, got {other:?}"),
        }
    }

    #[test]
    fn lifecycle_initialize() {
        let body = br#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}"#;
        let req = parse(body).unwrap();
        assert!(matches!(
            req,
            McpRequest::Lifecycle { method: LifecycleMethod::Initialize }
        ));
    }

    #[test]
    fn unknown_method_returns_error() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"unsupported/method","params":{}}"#;
        assert!(matches!(parse(body), Err(ParseError::UnknownMethod(_))));
    }

    #[test]
    fn bad_version_returns_error() {
        let body = br#"{"jsonrpc":"1.0","id":1,"method":"ping","params":{}}"#;
        assert!(matches!(parse(body), Err(ParseError::BadVersion(_))));
    }

    #[test]
    fn missing_tool_name_returns_error() {
        let body = br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{}}"#;
        assert!(matches!(parse(body), Err(ParseError::MissingField { .. })));
    }
}
