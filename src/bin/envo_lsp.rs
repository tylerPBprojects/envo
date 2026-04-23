//! Binary entry point for the envo LSP server.
//!
//! This is a standalone binary that communicates over stdio using the
//! Language Server Protocol. It is started as a child process by the
//! VS Code extension.

#[tokio::main]
async fn main() {
    envo::lsp::run_server().await;
}
