//! MCP server for envo — exposes environment lifecycle as structured
//! operations for AI agents.
//!
//! Implements the Model Context Protocol (MCP) over stdio using JSON-RPC 2.0
//! with Content-Length framing. The server is synchronous — it processes
//! one request at a time, which is acceptable since Nix operations are
//! inherently sequential.

pub mod protocol;
pub mod resources;
pub mod tools;

use protocol::{
    read_message, write_response, JsonRpcResponse, INTERNAL_ERROR, INVALID_PARAMS,
    METHOD_NOT_FOUND,
};
use serde_json::json;
use std::io::{self, BufReader, BufWriter};
use std::path::PathBuf;

/// Run the MCP server, reading from stdin and writing to stdout.
///
/// The server processes messages sequentially until EOF or an `exit`
/// notification is received. All logging goes to stderr.
pub fn run_server() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());

    // Track the working directory — set by initialize params or default to cwd
    let mut working_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    eprintln!("[envo-mcp] server starting");

    loop {
        let msg = match read_message(&mut reader) {
            Ok(Some(msg)) => msg,
            Ok(None) => {
                eprintln!("[envo-mcp] EOF, shutting down");
                break;
            }
            Err(e) => {
                eprintln!("[envo-mcp] read error: {e}");
                // Try to continue — might be a single malformed message
                continue;
            }
        };

        eprintln!("[envo-mcp] received: {} (id: {:?})", msg.method, msg.id);

        // Notifications (no id) don't get responses
        let is_notification = msg.id.is_none();

        let response = match msg.method.as_str() {
            "initialize" => {
                // Extract working directory from params if provided
                if let Some(roots) = msg.params.get("roots").and_then(|r| r.as_array()) {
                    if let Some(first) = roots.first() {
                        if let Some(uri) = first.get("uri").and_then(|u| u.as_str()) {
                            if let Some(path) = uri.strip_prefix("file://") {
                                working_dir = PathBuf::from(path);
                                eprintln!("[envo-mcp] working directory: {}", working_dir.display());
                            }
                        }
                    }
                }

                Some(JsonRpcResponse::success(
                    msg.id,
                    json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {
                            "tools": {},
                            "resources": {}
                        },
                        "serverInfo": {
                            "name": "envo-mcp",
                            "version": env!("CARGO_PKG_VERSION")
                        }
                    }),
                ))
            }

            "notifications/initialized" => {
                eprintln!("[envo-mcp] client initialized");
                None // No response for notifications
            }

            "tools/list" => Some(JsonRpcResponse::success(
                msg.id,
                json!({ "tools": tools::tool_definitions() }),
            )),

            "tools/call" => {
                let tool_name = msg
                    .params
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                let arguments = msg
                    .params
                    .get("arguments")
                    .cloned()
                    .unwrap_or(json!({}));

                // Inject working directory if not specified in arguments
                let mut args = arguments.clone();
                if args.get("directory").is_none() || args["directory"].as_str().unwrap_or("").is_empty() {
                    if let Some(obj) = args.as_object_mut() {
                        obj.insert(
                            "directory".to_string(),
                            json!(working_dir.to_string_lossy()),
                        );
                    }
                }

                let result = tools::call_tool(tool_name, &args);
                Some(JsonRpcResponse::success(msg.id, result))
            }

            "resources/list" => Some(JsonRpcResponse::success(
                msg.id,
                json!({ "resources": resources::resource_definitions() }),
            )),

            "resources/read" => {
                let uri = msg
                    .params
                    .get("uri")
                    .and_then(|u| u.as_str())
                    .unwrap_or("");

                if uri.is_empty() {
                    Some(JsonRpcResponse::error(
                        msg.id,
                        INVALID_PARAMS,
                        "missing uri parameter",
                    ))
                } else {
                    let result = resources::read_resource(uri, &working_dir);
                    Some(JsonRpcResponse::success(msg.id, result))
                }
            }

            "shutdown" => {
                eprintln!("[envo-mcp] shutdown requested");
                Some(JsonRpcResponse::success(msg.id, json!(null)))
            }

            "exit" => {
                eprintln!("[envo-mcp] exit");
                break;
            }

            _ => {
                if is_notification {
                    eprintln!("[envo-mcp] ignoring unknown notification: {}", msg.method);
                    None
                } else {
                    Some(JsonRpcResponse::error(
                        msg.id,
                        METHOD_NOT_FOUND,
                        format!("unknown method: {}", msg.method),
                    ))
                }
            }
        };

        if let Some(resp) = response {
            write_response(&mut writer, &resp)?;
        }
    }

    eprintln!("[envo-mcp] server stopped");
    Ok(())
}
