// MCP server implementation (JSON-RPC 2.0 over stdio)

use crate::protocol::*;
use crate::tools::ToolRegistry;
use anyhow::Result;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::RwLock;

pub struct McpServer {
    registry: Arc<RwLock<ToolRegistry>>,
    initialized: Arc<RwLock<bool>>,
}

impl McpServer {
    pub fn new(registry: ToolRegistry) -> Self {
        Self {
            registry: Arc::new(RwLock::new(registry)),
            initialized: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the MCP server (JSON-RPC over stdio)
    pub async fn start(&self) -> Result<()> {
        tracing::info!("Starting MCP server");

        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        while let Some(line) = lines.next_line().await? {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            tracing::debug!("Received: {}", line);

            let response = self.handle_request(line).await;
            let response_json = serde_json::to_string(&response)?;

            tracing::debug!("Sending: {}", response_json);

            stdout.write_all(response_json.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }

        Ok(())
    }

    async fn handle_request(&self, line: &str) -> JsonRpcResponse {
        // Parse the request
        let request: JsonRpcRequest = match serde_json::from_str(line) {
            Ok(req) => req,
            Err(e) => {
                tracing::error!("Failed to parse request: {}", e);
                return JsonRpcResponse::error(
                    serde_json::Value::Null,
                    JsonRpcError::parse_error(),
                );
            }
        };

        // Handle the request
        let id = request.id.clone().unwrap_or(serde_json::Value::Null);

        match request.method.as_str() {
            "initialize" => self.handle_initialize(id, request.params).await,
            "tools/list" => self.handle_tools_list(id).await,
            "tools/call" => self.handle_tools_call(id, request.params).await,
            method => {
                tracing::warn!("Unknown method: {}", method);
                JsonRpcResponse::error(id, JsonRpcError::method_not_found(method))
            }
        }
    }

    async fn handle_initialize(
        &self,
        id: serde_json::Value,
        params: Option<serde_json::Value>,
    ) -> JsonRpcResponse {
        let _params: InitializeParams = match params {
            Some(p) => match serde_json::from_value(p) {
                Ok(params) => params,
                Err(e) => {
                    return JsonRpcResponse::error(
                        id,
                        JsonRpcError::invalid_params(format!("Invalid initialize params: {}", e)),
                    )
                }
            },
            None => {
                return JsonRpcResponse::error(
                    id,
                    JsonRpcError::invalid_params("Missing initialize params"),
                )
            }
        };

        *self.initialized.write().await = true;

        let result = InitializeResult {
            protocol_version: "2024-11-05".to_string(),
            capabilities: ServerCapabilities {
                tools: Some(ToolsCapability {
                    list_changed: false,
                }),
                experimental: serde_json::Value::Null,
            },
            server_info: ServerInfo {
                name: "shiioo-mcp".to_string(),
                version: env!("CARGO_PKG_VERSION").to_string(),
            },
        };

        JsonRpcResponse::success(id, result)
    }

    async fn handle_tools_list(&self, id: serde_json::Value) -> JsonRpcResponse {
        if !*self.initialized.read().await {
            return JsonRpcResponse::error(
                id,
                JsonRpcError::custom(-32002, "Server not initialized"),
            );
        }

        let registry = self.registry.read().await;
        let tools = registry.list_schemas();

        let result = ListToolsResult { tools };
        JsonRpcResponse::success(id, result)
    }

    async fn handle_tools_call(
        &self,
        id: serde_json::Value,
        params: Option<serde_json::Value>,
    ) -> JsonRpcResponse {
        if !*self.initialized.read().await {
            return JsonRpcResponse::error(
                id,
                JsonRpcError::custom(-32002, "Server not initialized"),
            );
        }

        let params: CallToolParams = match params {
            Some(p) => match serde_json::from_value(p) {
                Ok(params) => params,
                Err(e) => {
                    return JsonRpcResponse::error(
                        id,
                        JsonRpcError::invalid_params(format!("Invalid tool call params: {}", e)),
                    )
                }
            },
            None => {
                return JsonRpcResponse::error(
                    id,
                    JsonRpcError::invalid_params("Missing tool call params"),
                )
            }
        };

        let registry = self.registry.read().await;

        let tool = match registry.get(&params.name) {
            Some(tool) => tool,
            None => {
                return JsonRpcResponse::error(
                    id,
                    JsonRpcError::custom(-32001, format!("Tool not found: {}", params.name)),
                )
            }
        };

        // Execute the tool
        match tool.execute(params.arguments).await {
            Ok(result) => JsonRpcResponse::success(id, result),
            Err(e) => {
                let error_result = CallToolResult {
                    content: vec![ToolContent::error(format!("Tool execution failed: {}", e))],
                    is_error: Some(true),
                };
                JsonRpcResponse::success(id, error_result)
            }
        }
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new(ToolRegistry::new())
    }
}
