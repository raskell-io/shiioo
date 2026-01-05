// MCP tool definitions and implementations

use crate::protocol::{CallToolResult, ToolContent, ToolSchema};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;

/// Tool executor trait
#[async_trait::async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool schema for MCP
    fn schema(&self) -> ToolSchema;

    /// Execute the tool with given arguments
    async fn execute(&self, arguments: serde_json::Value) -> Result<CallToolResult>;

    /// Get the tool's tier (for policy enforcement)
    fn tier(&self) -> ToolTier {
        ToolTier::Tier0
    }
}

/// Tool security tier
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolTier {
    /// Read-only operations
    Tier0,
    /// Controlled writes (PR-only, reversible)
    Tier1,
    /// Dangerous operations (always require approval)
    Tier2,
}

/// Tool registry for managing available tools
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool
    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let schema = tool.schema();
        self.tools.insert(schema.name.clone(), tool);
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// List all tool schemas
    pub fn list_schemas(&self) -> Vec<ToolSchema> {
        self.tools.values().map(|t| t.schema()).collect()
    }

    /// Check if a tool exists
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// Helper functions for creating tool schemas

pub fn json_schema_object(properties: serde_json::Value, required: Vec<&str>) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required
    })
}

pub fn json_schema_string(description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "string",
        "description": description
    })
}

pub fn json_schema_number(description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "number",
        "description": description
    })
}

pub fn json_schema_boolean(description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "boolean",
        "description": description
    })
}

pub fn json_schema_array(items: serde_json::Value, description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "array",
        "items": items,
        "description": description
    })
}
