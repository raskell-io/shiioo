//! Tool definitions and executor for the Chief of Staff.

use serde_json::json;
use shiioo_core::agent::{
    AgentId, AgentOrganization, AgentStatus, AgentTask, ExecutionContext,
};
use shiioo_core::storage::AgentStore;
use shiioo_core::types::{TeamId, ToolDefinition};

use crate::config::AppState;

/// Result of executing a tool.
pub struct ToolExecResult {
    pub content: String,
    pub is_error: bool,
}

/// Return tool definitions available to the Chief of Staff.
pub fn chief_of_staff_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "list_employees".to_string(),
            description: "List employees in the company. Optionally filter by status or team."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "description": "Filter by status: active, paused, suspended, or archived",
                        "enum": ["active", "paused", "suspended", "archived"]
                    },
                    "team": {
                        "type": "string",
                        "description": "Filter by team ID"
                    }
                },
                "required": []
            }),
        },
        ToolDefinition {
            name: "get_employee".to_string(),
            description: "Get detailed information about a specific employee by their ID."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The employee ID"
                    }
                },
                "required": ["id"]
            }),
        },
        ToolDefinition {
            name: "hire_employee".to_string(),
            description: "Hire a new employee (create an agent). Returns the new employee's details."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "The employee's name"
                    },
                    "description": {
                        "type": "string",
                        "description": "What this employee does / their role description"
                    },
                    "team": {
                        "type": "string",
                        "description": "The team ID to assign the employee to"
                    },
                    "archetype": {
                        "type": "string",
                        "description": "Optional archetype/job title ID"
                    }
                },
                "required": ["name", "description", "team"]
            }),
        },
        ToolDefinition {
            name: "delegate_task".to_string(),
            description: "Delegate a task to an employee for execution.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "employee_id": {
                        "type": "string",
                        "description": "The employee ID to delegate to"
                    },
                    "task": {
                        "type": "string",
                        "description": "The task description/prompt to execute"
                    },
                    "reason": {
                        "type": "string",
                        "description": "Why this employee was chosen for this task"
                    }
                },
                "required": ["employee_id", "task"]
            }),
        },
        ToolDefinition {
            name: "company_status".to_string(),
            description: "Get an overview of the company: employee count, LLM capacity sources, and cost."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDefinition {
            name: "list_teams".to_string(),
            description: "List all teams in the company.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDefinition {
            name: "check_budgets".to_string(),
            description: "Check budget allocations. Optionally specify an employee ID to see their individual budgets."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "employee_id": {
                        "type": "string",
                        "description": "Optional employee ID to check specific budgets"
                    }
                },
                "required": []
            }),
        },
    ]
}

/// Execute a tool by name with the given input.
pub async fn execute_tool(
    name: &str,
    input: &serde_json::Value,
    state: &AppState,
) -> ToolExecResult {
    match name {
        "list_employees" => exec_list_employees(input, state).await,
        "get_employee" => exec_get_employee(input, state).await,
        "hire_employee" => exec_hire_employee(input, state).await,
        "delegate_task" => exec_delegate_task(input, state).await,
        "company_status" => exec_company_status(state).await,
        "list_teams" => exec_list_teams(state).await,
        "check_budgets" => exec_check_budgets(input, state).await,
        _ => ToolExecResult {
            content: format!("Unknown tool: {name}"),
            is_error: true,
        },
    }
}

async fn exec_list_employees(input: &serde_json::Value, state: &AppState) -> ToolExecResult {
    let result = if let Some(status_str) = input["status"].as_str() {
        let status = match status_str {
            "active" => AgentStatus::Active,
            "paused" => AgentStatus::Paused,
            "suspended" => AgentStatus::Suspended,
            "archived" => AgentStatus::Archived,
            other => {
                return ToolExecResult {
                    content: format!("Invalid status: {other}. Use active, paused, suspended, or archived."),
                    is_error: true,
                };
            }
        };
        state.agent_store.list_agents_by_status(status).await
    } else if let Some(team) = input["team"].as_str() {
        state.agent_store.list_agents_by_team(team).await
    } else {
        state.agent_store.list_agents().await
    };

    match result {
        Ok(agents) => {
            if agents.is_empty() {
                ToolExecResult {
                    content: "No employees found.".to_string(),
                    is_error: false,
                }
            } else {
                let mut lines = vec![format!("Found {} employee(s):", agents.len())];
                for agent in &agents {
                    lines.push(format!(
                        "- {} (id: {}) — {} [status: {}, team: {}]",
                        agent.name,
                        agent.id,
                        agent.description,
                        agent.status,
                        agent.organization.team.0,
                    ));
                }
                ToolExecResult {
                    content: lines.join("\n"),
                    is_error: false,
                }
            }
        }
        Err(e) => ToolExecResult {
            content: format!("Failed to list employees: {e}"),
            is_error: true,
        },
    }
}

async fn exec_get_employee(input: &serde_json::Value, state: &AppState) -> ToolExecResult {
    let id = match input["id"].as_str() {
        Some(id) => id,
        None => {
            return ToolExecResult {
                content: "Missing required parameter: id".to_string(),
                is_error: true,
            };
        }
    };

    match state.agent_store.get_agent(&AgentId::new(id)).await {
        Ok(Some(agent)) => {
            let mut info = vec![
                format!("Name: {}", agent.name),
                format!("ID: {}", agent.id),
                format!("Description: {}", agent.description),
                format!("Status: {}", agent.status),
                format!("Team: {}", agent.organization.team.0),
            ];
            if let Some(ref archetype) = agent.archetype {
                info.push(format!("Job title: {}", archetype.0));
            }
            if let Some(ref supervisor) = agent.organization.reports_to {
                info.push(format!("Reports to: {}", supervisor));
            }
            info.push(format!("Created: {}", agent.created_at.format("%Y-%m-%d %H:%M UTC")));
            ToolExecResult {
                content: info.join("\n"),
                is_error: false,
            }
        }
        Ok(None) => ToolExecResult {
            content: format!("Employee not found: {id}"),
            is_error: true,
        },
        Err(e) => ToolExecResult {
            content: format!("Failed to get employee: {e}"),
            is_error: true,
        },
    }
}

async fn exec_hire_employee(input: &serde_json::Value, state: &AppState) -> ToolExecResult {
    let name = match input["name"].as_str() {
        Some(n) => n,
        None => {
            return ToolExecResult {
                content: "Missing required parameter: name".to_string(),
                is_error: true,
            };
        }
    };
    let description = match input["description"].as_str() {
        Some(d) => d,
        None => {
            return ToolExecResult {
                content: "Missing required parameter: description".to_string(),
                is_error: true,
            };
        }
    };
    let team = match input["team"].as_str() {
        Some(t) => t,
        None => {
            return ToolExecResult {
                content: "Missing required parameter: team".to_string(),
                is_error: true,
            };
        }
    };

    let id = AgentId::generate();
    let mut builder = shiioo_core::agent::Agent::builder(id.clone(), name)
        .description(description)
        .organization(AgentOrganization::new(TeamId::new(team)))
        .created_by("chief-of-staff");

    if let Some(archetype) = input["archetype"].as_str() {
        builder = builder.archetype(shiioo_core::agent::ArchetypeId::new(archetype));
    }

    let agent = builder.build();

    match state.agent_store.store_agent(&agent).await {
        Ok(()) => ToolExecResult {
            content: format!(
                "Hired {} (id: {}) on team '{}'. Status: active.",
                agent.name, agent.id, team
            ),
            is_error: false,
        },
        Err(e) => ToolExecResult {
            content: format!("Failed to hire employee: {e}"),
            is_error: true,
        },
    }
}

async fn exec_delegate_task(input: &serde_json::Value, state: &AppState) -> ToolExecResult {
    let employee_id = match input["employee_id"].as_str() {
        Some(id) => id,
        None => {
            return ToolExecResult {
                content: "Missing required parameter: employee_id".to_string(),
                is_error: true,
            };
        }
    };
    let task_prompt = match input["task"].as_str() {
        Some(t) => t,
        None => {
            return ToolExecResult {
                content: "Missing required parameter: task".to_string(),
                is_error: true,
            };
        }
    };

    let agent = match state.agent_store.get_agent(&AgentId::new(employee_id)).await {
        Ok(Some(agent)) => agent,
        Ok(None) => {
            return ToolExecResult {
                content: format!("Employee not found: {employee_id}"),
                is_error: true,
            };
        }
        Err(e) => {
            return ToolExecResult {
                content: format!("Failed to look up employee: {e}"),
                is_error: true,
            };
        }
    };

    if !agent.can_accept_work() {
        return ToolExecResult {
            content: format!(
                "Employee {} ({}) cannot accept work — current status: {}",
                agent.name, agent.id, agent.status
            ),
            is_error: true,
        };
    }

    let task = AgentTask {
        id: uuid::Uuid::new_v4().to_string(),
        prompt: task_prompt.to_string(),
        skill: None,
        context: std::collections::HashMap::new(),
        run_id: None,
        step_id: None,
    };

    let context = ExecutionContext::default();

    match state.agent_runtime.execute_task(&agent, task, context).await {
        Ok(result) => {
            let status = if result.success { "completed" } else { "failed" };
            let mut lines = vec![format!(
                "Task delegated to {} ({}) — {}.",
                agent.name, agent.id, status
            )];
            if let Some(ref output) = result.output {
                lines.push(format!("Output: {output}"));
            }
            if let Some(ref error) = result.error {
                lines.push(format!("Error: {error}"));
            }
            ToolExecResult {
                content: lines.join("\n"),
                is_error: !result.success,
            }
        }
        Err(e) => ToolExecResult {
            content: format!("Task execution failed: {e}"),
            is_error: true,
        },
    }
}

async fn exec_company_status(state: &AppState) -> ToolExecResult {
    let agents = state.agent_store.list_agents().await.unwrap_or_default();
    let active = agents
        .iter()
        .filter(|a| a.status == AgentStatus::Active)
        .count();
    let total = agents.len();

    let sources = state.capacity_broker.list_sources();

    let cost = state.capacity_broker.get_total_cost(chrono::Utc::now() - chrono::Duration::days(1));

    let teams = state.agent_orchestrator.list_teams().await;

    let mut lines = vec![
        format!("Employees: {active} active / {total} total"),
        format!("Teams: {}", teams.len()),
        format!("LLM capacity sources: {}", sources.len()),
        format!("Cost (last 24h): ${cost:.4}"),
    ];

    if !sources.is_empty() {
        lines.push("Sources:".to_string());
        for source in &sources {
            lines.push(format!(
                "  - {} ({:?}, model: {}, priority: {})",
                source.name, source.provider, source.model, source.priority
            ));
        }
    }

    ToolExecResult {
        content: lines.join("\n"),
        is_error: false,
    }
}

async fn exec_list_teams(state: &AppState) -> ToolExecResult {
    let teams = state.agent_orchestrator.list_teams().await;

    if teams.is_empty() {
        return ToolExecResult {
            content: "No teams have been created yet.".to_string(),
            is_error: false,
        };
    }

    let mut lines = vec![format!("Found {} team(s):", teams.len())];
    for team in &teams {
        lines.push(format!(
            "- {} (id: {}) — lead: {}, {} member(s)",
            team.name,
            team.id,
            team.lead,
            team.members.len(),
        ));
    }
    ToolExecResult {
        content: lines.join("\n"),
        is_error: false,
    }
}

async fn exec_check_budgets(input: &serde_json::Value, state: &AppState) -> ToolExecResult {
    if let Some(employee_id) = input["employee_id"].as_str() {
        match state
            .agent_store
            .get_agent(&AgentId::new(employee_id))
            .await
        {
            Ok(Some(agent)) => {
                let budgets = &agent.budgets;
                let lines = vec![
                    format!("Budgets for {} ({}):", agent.name, agent.id),
                    format!(
                        "  Tokens — per request: {}, per hour: {}, per day: {}, per month: {}",
                        fmt_opt_u64(budgets.tokens.per_request),
                        fmt_opt_u64(budgets.tokens.per_hour),
                        fmt_opt_u64(budgets.tokens.per_day),
                        fmt_opt_u64(budgets.tokens.per_month),
                    ),
                    format!(
                        "  Cost — per request: {}, per hour: {}, per day: {}, per month: {}",
                        fmt_opt_cents(budgets.cost.per_request_cents),
                        fmt_opt_cents(budgets.cost.per_hour_cents),
                        fmt_opt_cents(budgets.cost.per_day_cents),
                        fmt_opt_cents(budgets.cost.per_month_cents),
                    ),
                    format!(
                        "  Requests — per minute: {}, per hour: {}, per day: {}",
                        fmt_opt_u32(budgets.requests.per_minute),
                        fmt_opt_u32(budgets.requests.per_hour),
                        fmt_opt_u32(budgets.requests.per_day),
                    ),
                ];
                ToolExecResult {
                    content: lines.join("\n"),
                    is_error: false,
                }
            }
            Ok(None) => ToolExecResult {
                content: format!("Employee not found: {employee_id}"),
                is_error: true,
            },
            Err(e) => ToolExecResult {
                content: format!("Failed to get employee: {e}"),
                is_error: true,
            },
        }
    } else {
        // Show all employees' budget summaries
        let agents = state.agent_store.list_agents().await.unwrap_or_default();
        if agents.is_empty() {
            return ToolExecResult {
                content: "No employees hired yet — no budgets to show.".to_string(),
                is_error: false,
            };
        }

        let mut lines = vec![format!("Budget summary for {} employee(s):", agents.len())];
        for agent in &agents {
            let b = &agent.budgets;
            let daily_tokens = fmt_opt_u64(b.tokens.per_day);
            let daily_cost = fmt_opt_cents(b.cost.per_day_cents);
            lines.push(format!(
                "- {} ({}): daily tokens={}, daily cost={}",
                agent.name, agent.id, daily_tokens, daily_cost,
            ));
        }
        ToolExecResult {
            content: lines.join("\n"),
            is_error: false,
        }
    }
}

fn fmt_opt_u64(v: Option<u64>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "unlimited".to_string())
}

fn fmt_opt_u32(v: Option<u32>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "unlimited".to_string())
}

fn fmt_opt_cents(v: Option<u64>) -> String {
    v.map(|c| format!("${:.2}", c as f64 / 100.0))
        .unwrap_or_else(|| "unlimited".to_string())
}

// === Tests ===

#[cfg(test)]
mod tests {
    use super::*;
    use shiioo_core::storage::InMemoryAgentStore;
    use std::sync::Arc;

    #[test]
    fn test_tool_definitions_valid() {
        let tools = chief_of_staff_tools();
        assert_eq!(tools.len(), 7);

        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"list_employees"));
        assert!(names.contains(&"get_employee"));
        assert!(names.contains(&"hire_employee"));
        assert!(names.contains(&"delegate_task"));
        assert!(names.contains(&"company_status"));
        assert!(names.contains(&"list_teams"));
        assert!(names.contains(&"check_budgets"));

        // All schemas should be valid JSON objects
        for tool in &tools {
            assert!(tool.input_schema.is_object(), "Schema for {} is not an object", tool.name);
            assert_eq!(
                tool.input_schema["type"].as_str(),
                Some("object"),
                "Schema for {} missing type:object",
                tool.name,
            );
            assert!(
                tool.input_schema["properties"].is_object(),
                "Schema for {} missing properties",
                tool.name,
            );
        }
    }

    #[test]
    fn test_tool_names_unique() {
        let tools = chief_of_staff_tools();
        let mut names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        let original_len = names.len();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), original_len, "Duplicate tool names found");
    }

    #[tokio::test]
    async fn test_exec_list_employees_empty() {
        let store = Arc::new(InMemoryAgentStore::new());
        let result = exec_list_employees_with_store(&json!({}), store.as_ref()).await;
        assert!(!result.is_error);
        assert!(result.content.contains("No employees found"));
    }

    #[tokio::test]
    async fn test_exec_list_employees_with_data() {
        let store = Arc::new(InMemoryAgentStore::new());

        let agent = shiioo_core::agent::Agent::builder("eng-alice", "Alice")
            .description("Software Engineer")
            .organization(AgentOrganization::new(TeamId::new("engineering")))
            .build();
        store.store_agent(&agent).await.unwrap();

        let result = exec_list_employees_with_store(&json!({}), store.as_ref()).await;
        assert!(!result.is_error);
        assert!(result.content.contains("Alice"));
        assert!(result.content.contains("1 employee(s)"));
    }

    #[tokio::test]
    async fn test_exec_list_employees_by_team() {
        let store = Arc::new(InMemoryAgentStore::new());

        let alice = shiioo_core::agent::Agent::builder("eng-alice", "Alice")
            .description("Software Engineer")
            .organization(AgentOrganization::new(TeamId::new("engineering")))
            .build();
        let bob = shiioo_core::agent::Agent::builder("mkt-bob", "Bob")
            .description("Marketer")
            .organization(AgentOrganization::new(TeamId::new("marketing")))
            .build();
        store.store_agent(&alice).await.unwrap();
        store.store_agent(&bob).await.unwrap();

        let result =
            exec_list_employees_with_store(&json!({"team": "engineering"}), store.as_ref()).await;
        assert!(!result.is_error);
        assert!(result.content.contains("Alice"));
        assert!(!result.content.contains("Bob"));
    }

    #[tokio::test]
    async fn test_exec_get_employee_found() {
        let store = Arc::new(InMemoryAgentStore::new());

        let agent = shiioo_core::agent::Agent::builder("eng-alice", "Alice")
            .description("Software Engineer")
            .organization(AgentOrganization::new(TeamId::new("engineering")))
            .build();
        store.store_agent(&agent).await.unwrap();

        let result =
            exec_get_employee_with_store(&json!({"id": "eng-alice"}), store.as_ref()).await;
        assert!(!result.is_error);
        assert!(result.content.contains("Alice"));
        assert!(result.content.contains("engineering"));
    }

    #[tokio::test]
    async fn test_exec_get_employee_not_found() {
        let store = Arc::new(InMemoryAgentStore::new());

        let result =
            exec_get_employee_with_store(&json!({"id": "nonexistent"}), store.as_ref()).await;
        assert!(result.is_error);
        assert!(result.content.contains("not found"));
    }

    #[tokio::test]
    async fn test_exec_hire_employee() {
        let store = Arc::new(InMemoryAgentStore::new());

        let input = json!({
            "name": "Charlie",
            "description": "QA Engineer",
            "team": "quality"
        });
        let result = exec_hire_employee_with_store(&input, store.as_ref()).await;
        assert!(!result.is_error);
        assert!(result.content.contains("Hired Charlie"));
        assert!(result.content.contains("quality"));

        // Verify the employee was actually stored
        let agents = store.list_agents().await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "Charlie");
    }

    #[tokio::test]
    async fn test_exec_hire_employee_missing_name() {
        let store = Arc::new(InMemoryAgentStore::new());

        let input = json!({"description": "test", "team": "test"});
        let result = exec_hire_employee_with_store(&input, store.as_ref()).await;
        assert!(result.is_error);
        assert!(result.content.contains("name"));
    }

    #[tokio::test]
    async fn test_unknown_tool() {
        // Use a dummy state-less check
        let tools = chief_of_staff_tools();
        let unknown = tools.iter().find(|t| t.name == "nonexistent_tool");
        assert!(unknown.is_none());
    }

    // Helper functions that work with just AgentStore for testing without full AppState

    async fn exec_list_employees_with_store(
        input: &serde_json::Value,
        store: &dyn AgentStore,
    ) -> ToolExecResult {
        let result = if let Some(status_str) = input["status"].as_str() {
            let status = match status_str {
                "active" => AgentStatus::Active,
                "paused" => AgentStatus::Paused,
                "suspended" => AgentStatus::Suspended,
                "archived" => AgentStatus::Archived,
                other => {
                    return ToolExecResult {
                        content: format!("Invalid status: {other}"),
                        is_error: true,
                    };
                }
            };
            store.list_agents_by_status(status).await
        } else if let Some(team) = input["team"].as_str() {
            store.list_agents_by_team(team).await
        } else {
            store.list_agents().await
        };

        match result {
            Ok(agents) => {
                if agents.is_empty() {
                    ToolExecResult {
                        content: "No employees found.".to_string(),
                        is_error: false,
                    }
                } else {
                    let mut lines = vec![format!("Found {} employee(s):", agents.len())];
                    for agent in &agents {
                        lines.push(format!(
                            "- {} (id: {}) — {} [status: {}, team: {}]",
                            agent.name,
                            agent.id,
                            agent.description,
                            agent.status,
                            agent.organization.team.0,
                        ));
                    }
                    ToolExecResult {
                        content: lines.join("\n"),
                        is_error: false,
                    }
                }
            }
            Err(e) => ToolExecResult {
                content: format!("Failed: {e}"),
                is_error: true,
            },
        }
    }

    async fn exec_get_employee_with_store(
        input: &serde_json::Value,
        store: &dyn AgentStore,
    ) -> ToolExecResult {
        let id = match input["id"].as_str() {
            Some(id) => id,
            None => {
                return ToolExecResult {
                    content: "Missing: id".to_string(),
                    is_error: true,
                };
            }
        };

        match store.get_agent(&AgentId::new(id)).await {
            Ok(Some(agent)) => {
                let info = vec![
                    format!("Name: {}", agent.name),
                    format!("ID: {}", agent.id),
                    format!("Description: {}", agent.description),
                    format!("Team: {}", agent.organization.team.0),
                ];
                ToolExecResult {
                    content: info.join("\n"),
                    is_error: false,
                }
            }
            Ok(None) => ToolExecResult {
                content: format!("Employee not found: {id}"),
                is_error: true,
            },
            Err(e) => ToolExecResult {
                content: format!("Failed: {e}"),
                is_error: true,
            },
        }
    }

    async fn exec_hire_employee_with_store(
        input: &serde_json::Value,
        store: &dyn AgentStore,
    ) -> ToolExecResult {
        let name = match input["name"].as_str() {
            Some(n) => n,
            None => {
                return ToolExecResult {
                    content: "Missing required parameter: name".to_string(),
                    is_error: true,
                };
            }
        };
        let description = match input["description"].as_str() {
            Some(d) => d,
            None => {
                return ToolExecResult {
                    content: "Missing required parameter: description".to_string(),
                    is_error: true,
                };
            }
        };
        let team = match input["team"].as_str() {
            Some(t) => t,
            None => {
                return ToolExecResult {
                    content: "Missing required parameter: team".to_string(),
                    is_error: true,
                };
            }
        };

        let id = AgentId::generate();
        let agent = shiioo_core::agent::Agent::builder(id.clone(), name)
            .description(description)
            .organization(AgentOrganization::new(TeamId::new(team)))
            .created_by("chief-of-staff")
            .build();

        match store.store_agent(&agent).await {
            Ok(()) => ToolExecResult {
                content: format!("Hired {} (id: {}) on team '{}'.", agent.name, agent.id, team),
                is_error: false,
            },
            Err(e) => ToolExecResult {
                content: format!("Failed: {e}"),
                is_error: true,
            },
        }
    }
}
