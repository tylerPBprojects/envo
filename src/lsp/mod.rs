//! LSP server for envo manifest.toml files.
//!
//! Implements the Language Server Protocol using `tower-lsp` to provide
//! diagnostics (inline errors/warnings), autocompletion, and hover
//! documentation for envo manifest files.
//!
//! The server communicates over stdio and is started as a child process
//! by the VS Code extension.

pub mod completion;
pub mod diagnostics;
pub mod hover;

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

/// The envo LSP server backend.
///
/// Holds the LSP client handle (for sending notifications like diagnostics)
/// and a cache of open document contents.
pub struct EnvoLspBackend {
    /// LSP client handle for sending notifications.
    client: Client,

    /// Cache of open document contents, keyed by URI.
    documents: Arc<RwLock<HashMap<String, String>>>,
}

impl EnvoLspBackend {
    /// Create a new LSP backend.
    pub fn new(client: Client) -> Self {
        Self {
            client,
            documents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Publish diagnostics for a document.
    async fn publish_diagnostics(&self, uri: Url, source: &str) {
        let diags = diagnostics::diagnose(source);
        self.client
            .publish_diagnostics(uri, diags, None)
            .await;
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for EnvoLspBackend {
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                completion_provider: Some(CompletionOptions {
                    trigger_characters: Some(vec![
                        "[".to_string(),
                        "=".to_string(),
                        "\"".to_string(),
                        ".".to_string(),
                    ]),
                    resolve_provider: Some(false),
                    ..Default::default()
                }),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: "envo-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "envo-lsp initialized")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        let text = params.text_document.text.clone();

        // Cache the document content
        self.documents
            .write()
            .await
            .insert(uri.to_string(), text.clone());

        // Publish diagnostics on open
        self.publish_diagnostics(uri, &text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();

        // We use FULL sync, so the first change contains the entire document
        if let Some(change) = params.content_changes.into_iter().next() {
            self.documents
                .write()
                .await
                .insert(uri.to_string(), change.text.clone());

            // Publish diagnostics on change
            self.publish_diagnostics(uri, &change.text).await;
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let uri = params.text_document.uri;

        // Re-publish diagnostics on save
        let docs = self.documents.read().await;
        if let Some(text) = docs.get(&uri.to_string()) {
            let text = text.clone();
            drop(docs);
            self.publish_diagnostics(uri, &text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;

        // Remove from cache
        self.documents.write().await.remove(&uri.to_string());

        // Clear diagnostics on close
        self.client
            .publish_diagnostics(uri, vec![], None)
            .await;
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;

        let docs = self.documents.read().await;
        let items = if let Some(source) = docs.get(&uri.to_string()) {
            completion::get_completions(source, position.line, position.character)
        } else {
            Vec::new()
        };

        if items.is_empty() {
            Ok(None)
        } else {
            Ok(Some(CompletionResponse::Array(items)))
        }
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;

        let docs = self.documents.read().await;
        let result = if let Some(source) = docs.get(&uri.to_string()) {
            hover::get_hover(source, position.line, position.character)
        } else {
            None
        };

        Ok(result.map(|content| Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: content,
            }),
            range: None,
        }))
    }
}

/// Run the LSP server on stdio.
///
/// This is called from the `envo-lsp` binary entry point.
pub async fn run_server() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = tower_lsp::LspService::new(EnvoLspBackend::new);
    tower_lsp::Server::new(stdin, stdout, socket)
        .serve(service)
        .await;
}
