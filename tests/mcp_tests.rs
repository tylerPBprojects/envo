//! External integration tests for the MCP server modules.

use envo::mcp::protocol::{
    format_message, read_message, write_response, JsonRpcResponse, METHOD_NOT_FOUND,
};
use envo::mcp::resources::{read_resource, resource_definitions};
use envo::mcp::tools::{call_tool, tool_definitions, tool_env_info};
use std::io::{BufReader, Cursor};

// ── Protocol tests ────────────────────────────────────────────────

#[test]
fn test_protocol_roundtrip() {
    let body = r#"{"jsonrpc":"2.0","id":42,"method":"tools/list","params":{}}"#;
    let input = format_message(body);
    let mut reader = BufReader::new(Cursor::new(input));

    let msg = read_message(&mut reader).unwrap().unwrap();
    assert_eq!(msg.method, "tools/list");
    assert_eq!(msg.id.unwrap(), 42);
}

#[test]
fn test_response_serialization() {
    let resp = JsonRpcResponse::success(
        Some(serde_json::json!(1)),
        serde_json::json!({"tools": []}),
    );
    let mut buf = Vec::new();
    write_response(&mut buf, &resp).unwrap();
    let output = String::from_utf8(buf).unwrap();
    assert!(output.contains("Content-Length:"));
    assert!(output.contains("\"tools\""));
}

#[test]
fn test_error_response_format() {
    let resp = JsonRpcResponse::error(
        Some(serde_json::json!(1)),
        METHOD_NOT_FOUND,
        "not found",
    );
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("-32601"));
    assert!(!json.contains("\"result\""));
}

// ── Tool tests ────────────────────────────────────────────────────

#[test]
fn test_tool_definitions_complete() {
    let defs = tool_definitions();
    let arr = defs.as_array().unwrap();
    assert_eq!(arr.len(), 6);
}

#[test]
fn test_tool_init_and_env_info() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().to_str().unwrap();

    // Init
    let result = call_tool("envo_init", &serde_json::json!({"directory": dir}));
    assert!(!result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false));

    // Env info
    let result = call_tool("envo_env_info", &serde_json::json!({"directory": dir}));
    let text = result["content"][0]["text"].as_str().unwrap();
    let data: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(data["initialized"], true);
}

#[test]
fn test_tool_error_on_bad_input() {
    let result = call_tool("envo_install", &serde_json::json!({}));
    assert!(result["isError"].as_bool().unwrap_or(false));

    let result = call_tool("envo_uninstall", &serde_json::json!({}));
    assert!(result["isError"].as_bool().unwrap_or(false));

    let result = call_tool("envo_search", &serde_json::json!({}));
    assert!(result["isError"].as_bool().unwrap_or(false));
}

#[test]
fn test_tool_unknown_name() {
    let result = call_tool("does_not_exist", &serde_json::json!({}));
    assert!(result["isError"].as_bool().unwrap_or(false));
}

// ── Resource tests ────────────────────────────────────────────────

#[test]
fn test_resource_definitions_complete() {
    let defs = resource_definitions();
    let arr = defs.as_array().unwrap();
    assert_eq!(arr.len(), 3);
}

#[test]
fn test_resource_manifest_with_file() {
    let tmp = tempfile::tempdir().unwrap();
    let envo_dir = tmp.path().join(".envo");
    std::fs::create_dir_all(&envo_dir).unwrap();
    std::fs::write(
        envo_dir.join("manifest.toml"),
        "[project]\nname = \"res-test\"\n",
    )
    .unwrap();

    let result = read_resource("envo://manifest", tmp.path());
    assert!(!result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false));
    let text = result["contents"][0]["text"].as_str().unwrap();
    assert!(text.contains("res-test"));
    assert_eq!(result["contents"][0]["mimeType"], "application/toml");
}

#[test]
fn test_resource_status() {
    let tmp = tempfile::tempdir().unwrap();
    let result = read_resource("envo://status", tmp.path());
    assert!(!result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false));
    assert_eq!(result["contents"][0]["mimeType"], "application/json");
}

#[test]
fn test_resource_unknown_uri() {
    let tmp = tempfile::tempdir().unwrap();
    let result = read_resource("envo://bogus", tmp.path());
    assert!(result.get("isError").and_then(|v| v.as_bool()).unwrap_or(false));
}
