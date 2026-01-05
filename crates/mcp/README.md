# Shiioo MCP Server

Model Context Protocol (MCP) server for Shiioo, providing enterprise tools via JSON-RPC 2.0 over stdio.

## Overview

The MCP server exposes Shiioo's workflow execution, event log, and context management capabilities as tools that can be called by LLM agents (Claude Code, etc.).

## Architecture

- **JSON-RPC 2.0 over stdio** - Standard protocol for tool communication
- **Tool Registry** - Manage and discover available tools
- **Tiered Security** - Tools are classified by risk level (Tier 0-2)
- **Event Logging** - All tool calls are logged for audit trails

## Available Tools

### Tier 0 (Read-Only)

**`context_get`** - Get details about a workflow run
```json
{
  "name": "context_get",
  "arguments": {
    "run_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

**`context_search`** - Search for workflow runs
```json
{
  "name": "context_search",
  "arguments": {
    "status": "completed",
    "limit": 10
  }
}
```

**`context_events`** - Get event log for a run
```json
{
  "name": "context_events",
  "arguments": {
    "run_id": "550e8400-e29b-41d4-a716-446655440000",
    "limit": 100
  }
}
```

## Running the Server

### Standalone Mode

```bash
# Build the MCP server
cargo build --bin shiioo-mcp --release

# Run with default data directory (./data)
./target/release/shiioo-mcp

# Run with custom data directory
SHIIOO_DATA_DIR=/path/to/data ./target/release/shiioo-mcp
```

### Testing with JSON-RPC

```bash
# Initialize
echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}' | ./target/release/shiioo-mcp

# List tools
echo '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | ./target/release/shiioo-mcp

# Call a tool
echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"context_search","arguments":{"limit":5}}}' | ./target/release/shiioo-mcp
```

## Protocol

The server implements the MCP protocol:

1. **Initialize**: Establish connection and exchange capabilities
2. **tools/list**: Discover available tools
3. **tools/call**: Execute a tool with arguments

All messages are JSON-RPC 2.0 format, one message per line over stdio.

## Tool Development

### Adding a New Tool

1. Create a struct that implements the `Tool` trait
2. Define the schema (name, description, input schema)
3. Implement the `execute` method
4. Register the tool in the registry

Example:

```rust
use shiioo_mcp::protocol::{CallToolResult, ToolContent, ToolSchema};
use shiioo_mcp::tools::{Tool, json_schema_object, json_schema_string};

pub struct MyTool;

#[async_trait::async_trait]
impl Tool for MyTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "my_tool".to_string(),
            description: "Does something useful".to_string(),
            input_schema: json_schema_object(
                serde_json::json!({
                    "arg1": json_schema_string("First argument")
                }),
                vec!["arg1"],
            ),
        }
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<CallToolResult> {
        // Tool implementation
        Ok(CallToolResult {
            content: vec![ToolContent::text("Success!")],
            is_error: None,
        })
    }

    fn tier(&self) -> ToolTier {
        ToolTier::Tier0  // Read-only
    }
}
```

## Security Tiers

- **Tier 0** (Read-only): Safe operations, no approval required
- **Tier 1** (Controlled writes): PR-only, reversible operations
- **Tier 2** (Dangerous): Requires manual approval (merge, deploy, etc.)

## Coming Soon

- Policy enforcement (per-role tool access control)
- Approval workflows for Tier 2 tools
- Additional tools:
  - `repo.read` - Read repository files
  - `repo.apply_patch` - Apply code changes (Tier 1)
  - `web.fetch` - Fetch web content
  - `deploy.run` - Deploy to environments (Tier 2)
