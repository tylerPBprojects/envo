//! Binary entry point for the envo MCP server.
//!
//! This is a standalone binary that communicates over stdio using the
//! MCP protocol (JSON-RPC 2.0 with Content-Length framing). AI agents
//! connect to it as an MCP server.

fn main() {
    if let Err(e) = envo::mcp::run_server() {
        eprintln!("[envo-mcp] fatal error: {e}");
        std::process::exit(1);
    }
}
