// Standalone MCP server binary

use anyhow::Result;
use shiioo_core::storage::{FilesystemBlobStore, JsonlEventLog, RedbIndexStore};
use shiioo_mcp::server::McpServer;
use shiioo_mcp::tools::*;
use std::path::PathBuf;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    tracing::info!("Shiioo MCP Server starting...");

    // Set up storage (use ./data by default)
    let data_dir = std::env::var("SHIIOO_DATA_DIR")
        .unwrap_or_else(|_| "./data".to_string());
    let data_path = PathBuf::from(data_dir);

    let _blob_store = Arc::new(FilesystemBlobStore::new(data_path.join("blobs"))?);
    let event_log = Arc::new(JsonlEventLog::new(data_path.join("events"))?);
    let index_store = Arc::new(RedbIndexStore::new(data_path.join("index.redb"))?);

    // Get repository root (current directory by default)
    let repo_root = std::env::current_dir()?;

    // Create tool registry
    let mut registry = ToolRegistry::new();

    // Register Tier 0 tools (read-only)

    // Context tools
    registry.register(Arc::new(ContextGetTool::new(index_store.clone())));
    registry.register(Arc::new(ContextSearchTool::new(index_store.clone())));
    registry.register(Arc::new(ContextEventsTool::new(event_log.clone())));

    // Repository tools
    registry.register(Arc::new(RepoReadTool::new(repo_root)));

    // Web tools
    registry.register(Arc::new(WebFetchTool::new()));

    tracing::info!("Registered {} tools", registry.list_schemas().len());

    // Start MCP server
    let server = McpServer::new(registry);
    server.start().await?;

    Ok(())
}
