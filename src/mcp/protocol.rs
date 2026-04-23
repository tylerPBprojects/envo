//! JSON-RPC 2.0 types and Content-Length framing for the MCP protocol.
//!
//! The MCP protocol uses JSON-RPC 2.0 over stdio with HTTP-style
//! Content-Length headers for message framing (identical to LSP).

use serde::{Deserialize, Serialize};
use std::io::{self, BufRead, Write};

/// A JSON-RPC 2.0 request or notification.
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcMessage {
    pub jsonrpc: String,
    pub id: Option<serde_json::Value>,
    pub method: String,
    #[serde(default)]
    pub params: serde_json::Value,
}

/// A JSON-RPC 2.0 response.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// A JSON-RPC 2.0 error object.
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// Standard JSON-RPC error codes
pub const PARSE_ERROR: i64 = -32700;
pub const INVALID_REQUEST: i64 = -32600;
pub const METHOD_NOT_FOUND: i64 = -32601;
pub const INVALID_PARAMS: i64 = -32602;
pub const INTERNAL_ERROR: i64 = -32603;

impl JsonRpcResponse {
    /// Create a success response.
    pub fn success(id: Option<serde_json::Value>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Create an error response.
    pub fn error(id: Option<serde_json::Value>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// Read a single JSON-RPC message from a buffered reader.
///
/// Parses the `Content-Length` header, reads the body, and deserializes
/// the JSON-RPC message. Returns `None` on EOF.
pub fn read_message(reader: &mut impl BufRead) -> io::Result<Option<JsonRpcMessage>> {
    // Read Content-Length header
    let content_length = match read_content_length(reader)? {
        Some(len) => len,
        None => return Ok(None), // EOF
    };

    // Read the body
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body)?;

    let body_str = String::from_utf8_lossy(&body);

    // Parse JSON-RPC message
    let msg: JsonRpcMessage = serde_json::from_str(&body_str).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid JSON-RPC message: {e}"),
        )
    })?;

    Ok(Some(msg))
}

/// Write a JSON-RPC response with Content-Length framing.
pub fn write_response(writer: &mut impl Write, response: &JsonRpcResponse) -> io::Result<()> {
    let body = serde_json::to_string(response).map_err(|e| {
        io::Error::new(io::ErrorKind::Other, format!("JSON serialization error: {e}"))
    })?;

    write!(writer, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    writer.flush()
}

/// Parse the Content-Length header from the input stream.
///
/// Reads lines until it finds `Content-Length: N`, then reads the
/// blank line separator. Returns `None` on EOF.
fn read_content_length(reader: &mut impl BufRead) -> io::Result<Option<usize>> {
    let mut content_length: Option<usize> = None;

    loop {
        let mut line = String::new();
        let bytes_read = reader.read_line(&mut line)?;

        if bytes_read == 0 {
            return Ok(None); // EOF
        }

        let trimmed = line.trim();

        if trimmed.is_empty() {
            // Blank line = end of headers
            break;
        }

        if let Some(value) = trimmed.strip_prefix("Content-Length:") {
            content_length = Some(value.trim().parse::<usize>().map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid Content-Length: {e}"),
                )
            })?);
        }
        // Ignore other headers (Content-Type, etc.)
    }

    match content_length {
        Some(len) => Ok(Some(len)),
        None => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "missing Content-Length header",
        )),
    }
}

/// Format a JSON-RPC message with Content-Length header (for testing).
pub fn format_message(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_read_message_valid() {
        let body = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let input = format_message(body);
        let mut reader = io::BufReader::new(Cursor::new(input));

        let msg = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(msg.method, "initialize");
        assert_eq!(msg.id, Some(serde_json::json!(1)));
    }

    #[test]
    fn test_read_message_notification() {
        let body = r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}"#;
        let input = format_message(body);
        let mut reader = io::BufReader::new(Cursor::new(input));

        let msg = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(msg.method, "notifications/initialized");
        assert!(msg.id.is_none());
    }

    #[test]
    fn test_read_message_eof() {
        let mut reader = io::BufReader::new(Cursor::new(""));
        let msg = read_message(&mut reader).unwrap();
        assert!(msg.is_none());
    }

    #[test]
    fn test_write_response() {
        let resp = JsonRpcResponse::success(Some(serde_json::json!(1)), serde_json::json!({"ok": true}));
        let mut output = Vec::new();
        write_response(&mut output, &resp).unwrap();

        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.starts_with("Content-Length:"));
        assert!(output_str.contains("\"jsonrpc\":\"2.0\""));
        assert!(output_str.contains("\"ok\":true"));
    }

    #[test]
    fn test_error_response() {
        let resp = JsonRpcResponse::error(Some(serde_json::json!(1)), METHOD_NOT_FOUND, "not found");
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("-32601"));
        assert!(json.contains("not found"));
        assert!(!json.contains("\"result\""));
    }

    #[test]
    fn test_success_response_no_error() {
        let resp = JsonRpcResponse::success(Some(serde_json::json!(1)), serde_json::json!("ok"));
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn test_format_message() {
        let body = r#"{"test":true}"#;
        let msg = format_message(body);
        assert_eq!(msg, "Content-Length: 13\r\n\r\n{\"test\":true}");
    }

    #[test]
    fn test_read_multiple_messages() {
        let body1 = r#"{"jsonrpc":"2.0","id":1,"method":"first","params":{}}"#;
        let body2 = r#"{"jsonrpc":"2.0","id":2,"method":"second","params":{}}"#;
        let input = format!("{}{}", format_message(body1), format_message(body2));
        let mut reader = io::BufReader::new(Cursor::new(input));

        let msg1 = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(msg1.method, "first");

        let msg2 = read_message(&mut reader).unwrap().unwrap();
        assert_eq!(msg2.method, "second");
    }
}
