// MCP server implementation
// Will be implemented in Phase 2

use anyhow::Result;

pub struct McpServer {
    // Will contain tool registry, policy engine reference, etc.
}

impl McpServer {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn start(&self) -> Result<()> {
        // Will implement JSON-RPC server over stdio
        tracing::info!("MCP server started (stub)");
        Ok(())
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}
