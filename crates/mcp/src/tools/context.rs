// Context tools for accessing workflow runs and events

use crate::protocol::{CallToolResult, ToolContent, ToolSchema};
use crate::tools::{json_schema_object, json_schema_string, Tool};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use shiioo_core::events::EventLog;
use shiioo_core::storage::IndexStore;
use shiioo_core::types::RunId;
use std::sync::Arc;

/// Tool to get run details
pub struct ContextGetTool {
    index_store: Arc<dyn IndexStore>,
}

impl ContextGetTool {
    pub fn new(index_store: Arc<dyn IndexStore>) -> Self {
        Self { index_store }
    }
}

#[derive(Debug, Deserialize)]
struct ContextGetArgs {
    run_id: String,
}

#[async_trait::async_trait]
impl Tool for ContextGetTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "context_get".to_string(),
            description: "Get details about a workflow run by its ID".to_string(),
            input_schema: json_schema_object(
                serde_json::json!({
                    "run_id": json_schema_string("The run ID to retrieve")
                }),
                vec!["run_id"],
            ),
        }
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<CallToolResult> {
        let args: ContextGetArgs = serde_json::from_value(arguments)
            .context("Invalid arguments for context_get")?;

        let run_id = RunId(args.run_id.parse().context("Invalid run ID format")?);

        match self.index_store.get_run(&run_id)? {
            Some(run) => {
                let json = serde_json::to_string_pretty(&run)?;
                Ok(CallToolResult {
                    content: vec![ToolContent::text(json)],
                    is_error: None,
                })
            }
            None => Ok(CallToolResult {
                content: vec![ToolContent::error(format!("Run {} not found", run_id))],
                is_error: Some(true),
            }),
        }
    }
}

/// Tool to search for runs
pub struct ContextSearchTool {
    index_store: Arc<dyn IndexStore>,
}

impl ContextSearchTool {
    pub fn new(index_store: Arc<dyn IndexStore>) -> Self {
        Self { index_store }
    }
}

#[derive(Debug, Deserialize)]
struct ContextSearchArgs {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait::async_trait]
impl Tool for ContextSearchTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "context_search".to_string(),
            description: "Search for workflow runs with optional filters".to_string(),
            input_schema: json_schema_object(
                serde_json::json!({
                    "status": {
                        "type": "string",
                        "description": "Filter by run status (pending, running, completed, failed, cancelled)",
                        "enum": ["pending", "running", "completed", "failed", "cancelled"]
                    },
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of results to return (default: 10)"
                    }
                }),
                vec![],
            ),
        }
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<CallToolResult> {
        let args: ContextSearchArgs = serde_json::from_value(arguments)
            .context("Invalid arguments for context_search")?;

        let mut runs = self.index_store.list_runs()?;

        // Filter by status if provided
        if let Some(status_str) = args.status {
            let status = match status_str.as_str() {
                "pending" => shiioo_core::types::RunStatus::Pending,
                "running" => shiioo_core::types::RunStatus::Running,
                "completed" => shiioo_core::types::RunStatus::Completed,
                "failed" => shiioo_core::types::RunStatus::Failed,
                "cancelled" => shiioo_core::types::RunStatus::Cancelled,
                _ => {
                    return Ok(CallToolResult {
                        content: vec![ToolContent::error(format!(
                            "Invalid status: {}",
                            status_str
                        ))],
                        is_error: Some(true),
                    })
                }
            };
            runs.retain(|r| r.status == status);
        }

        // Apply limit
        let limit = args.limit.unwrap_or(10);
        runs.truncate(limit);

        let json = serde_json::to_string_pretty(&runs)?;
        Ok(CallToolResult {
            content: vec![ToolContent::text(format!(
                "Found {} run(s):\n\n{}",
                runs.len(),
                json
            ))],
            is_error: None,
        })
    }
}

/// Tool to get events for a run
pub struct ContextEventsTool {
    event_log: Arc<dyn EventLog>,
}

impl ContextEventsTool {
    pub fn new(event_log: Arc<dyn EventLog>) -> Self {
        Self { event_log }
    }
}

#[derive(Debug, Deserialize)]
struct ContextEventsArgs {
    run_id: String,
    #[serde(default)]
    limit: Option<usize>,
}

#[async_trait::async_trait]
impl Tool for ContextEventsTool {
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "context_events".to_string(),
            description: "Get the event log for a workflow run".to_string(),
            input_schema: json_schema_object(
                serde_json::json!({
                    "run_id": json_schema_string("The run ID to get events for"),
                    "limit": {
                        "type": "number",
                        "description": "Maximum number of events to return (default: 100)"
                    }
                }),
                vec!["run_id"],
            ),
        }
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<CallToolResult> {
        let args: ContextEventsArgs = serde_json::from_value(arguments)
            .context("Invalid arguments for context_events")?;

        let run_id = RunId(args.run_id.parse().context("Invalid run ID format")?);

        let mut events = self.event_log.get_run_events(run_id).await?;

        // Apply limit
        let limit = args.limit.unwrap_or(100);
        events.truncate(limit);

        let json = serde_json::to_string_pretty(&events)?;
        Ok(CallToolResult {
            content: vec![ToolContent::text(format!(
                "Retrieved {} event(s):\n\n{}",
                events.len(),
                json
            ))],
            is_error: None,
        })
    }
}
