//! Agent Runtime - execution engine for agents.
//!
//! The runtime manages:
//! - Task execution with policy checking
//! - Budget tracking and enforcement
//! - Tool invocation through MCP bindings

use crate::agent::{
    policy_engine::{ActionContext, ActionType, AgentAction, AgentPolicyEngine, PolicyDecision},
    Agent, AgentId, AgentStatus, BudgetUsageStats, McpBinding,
};
use crate::events::EventLog;
use crate::types::RunId;
use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for the agent runtime.
#[derive(Debug, Clone)]
pub struct RuntimeConfig {
    /// Maximum execution time for a single task (seconds).
    pub max_task_duration_secs: u64,
    /// Whether to enforce budgets strictly.
    pub enforce_budgets: bool,
    /// Whether to require policy approval before execution.
    pub require_policy_check: bool,
    /// Default timeout for tool calls (seconds).
    pub default_tool_timeout_secs: u64,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            max_task_duration_secs: 3600,
            enforce_budgets: true,
            require_policy_check: true,
            default_tool_timeout_secs: 30,
        }
    }
}

/// Result of an agent task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    /// Unique execution ID.
    pub execution_id: String,
    /// Agent that executed the task.
    pub agent_id: AgentId,
    /// Whether execution succeeded.
    pub success: bool,
    /// Output produced by the agent.
    pub output: Option<String>,
    /// Error message if failed.
    pub error: Option<String>,
    /// Tool calls made during execution.
    pub tool_calls: Vec<ToolCallRecord>,
    /// Budget consumed.
    pub budget_consumed: BudgetConsumed,
    /// Execution start time.
    pub started_at: DateTime<Utc>,
    /// Execution end time.
    pub ended_at: DateTime<Utc>,
}

/// Record of a tool call made during execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub server_id: String,
    pub arguments: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub duration_ms: u64,
    pub timestamp: DateTime<Utc>,
}

/// Budget consumed during execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetConsumed {
    pub tokens_input: u64,
    pub tokens_output: u64,
    pub cost_cents: u64,
    pub requests: u64,
    pub tool_calls: u64,
}

/// A task to be executed by an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    /// Task identifier.
    pub id: String,
    /// The prompt/instruction for the agent.
    pub prompt: String,
    /// Optional skill to use (if not specified, agent decides).
    pub skill: Option<String>,
    /// Context from previous execution steps.
    pub context: HashMap<String, serde_json::Value>,
    /// Run ID this task belongs to.
    pub run_id: Option<RunId>,
    /// Step ID within the run.
    pub step_id: Option<String>,
}

/// Execution context for an agent.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub run_id: Option<RunId>,
    pub step_id: Option<String>,
    pub environment: String,
    pub parent_agent: Option<AgentId>,
    pub delegation_chain: Vec<AgentId>,
}

impl Default for ExecutionContext {
    fn default() -> Self {
        Self {
            run_id: None,
            step_id: None,
            environment: "development".to_string(),
            parent_agent: None,
            delegation_chain: Vec::new(),
        }
    }
}

/// Interface for resolving MCP bindings.
#[async_trait]
pub trait McpResolver: Send + Sync {
    /// Get available tools for an agent based on their bindings.
    async fn get_available_tools(&self, bindings: &[McpBinding]) -> Result<Vec<RuntimeToolInfo>>;
}

/// Information about an available tool in the runtime.
#[derive(Debug, Clone)]
pub struct RuntimeToolInfo {
    pub server_id: String,
    pub name: String,
    pub description: String,
    pub tier: u8,
}

/// Interface for tool execution.
#[async_trait]
pub trait ToolExecutor: Send + Sync {
    /// Execute a tool with the given arguments.
    async fn execute_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value>;
}

/// In-memory MCP resolver for testing.
pub struct InMemoryMcpResolver {
    tools: RwLock<Vec<RuntimeToolInfo>>,
}

impl InMemoryMcpResolver {
    pub fn new() -> Self {
        Self {
            tools: RwLock::new(Vec::new()),
        }
    }

    pub async fn register_tool(&self, tool: RuntimeToolInfo) {
        self.tools.write().await.push(tool);
    }
}

impl Default for InMemoryMcpResolver {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl McpResolver for InMemoryMcpResolver {
    async fn get_available_tools(&self, _bindings: &[McpBinding]) -> Result<Vec<RuntimeToolInfo>> {
        Ok(self.tools.read().await.clone())
    }
}

/// In-memory tool executor for testing.
pub struct InMemoryToolExecutor {
    handlers: RwLock<HashMap<String, Box<dyn Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync>>>,
}

impl InMemoryToolExecutor {
    pub fn new() -> Self {
        Self {
            handlers: RwLock::new(HashMap::new()),
        }
    }

    pub async fn register_handler<F>(&self, server_id: &str, tool_name: &str, handler: F)
    where
        F: Fn(serde_json::Value) -> Result<serde_json::Value> + Send + Sync + 'static,
    {
        let key = format!("{}:{}", server_id, tool_name);
        self.handlers.write().await.insert(key, Box::new(handler));
    }
}

impl Default for InMemoryToolExecutor {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ToolExecutor for InMemoryToolExecutor {
    async fn execute_tool(
        &self,
        server_id: &str,
        tool_name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let key = format!("{}:{}", server_id, tool_name);
        let handlers = self.handlers.read().await;
        if let Some(handler) = handlers.get(&key) {
            handler(arguments)
        } else {
            // Return a mock result for testing
            Ok(serde_json::json!({
                "status": "success",
                "tool": tool_name,
                "server": server_id,
                "message": "Tool executed (mock)"
            }))
        }
    }
}

/// The agent runtime manages execution of agent tasks.
pub struct AgentRuntime<P, M, T, E>
where
    P: AgentPolicyEngine,
    M: McpResolver,
    T: ToolExecutor,
    E: EventLog,
{
    config: RuntimeConfig,
    policy_engine: Arc<P>,
    mcp_resolver: Arc<M>,
    tool_executor: Arc<T>,
    #[allow(dead_code)]
    event_log: Arc<E>,
    /// Active budget usage per agent.
    budget_usage: RwLock<HashMap<AgentId, BudgetUsageStats>>,
}

impl<P, M, T, E> AgentRuntime<P, M, T, E>
where
    P: AgentPolicyEngine,
    M: McpResolver,
    T: ToolExecutor,
    E: EventLog,
{
    pub fn new(
        config: RuntimeConfig,
        policy_engine: Arc<P>,
        mcp_resolver: Arc<M>,
        tool_executor: Arc<T>,
        event_log: Arc<E>,
    ) -> Self {
        Self {
            config,
            policy_engine,
            mcp_resolver,
            tool_executor,
            event_log,
            budget_usage: RwLock::new(HashMap::new()),
        }
    }

    /// Execute a task with the given agent.
    pub async fn execute_task(
        &self,
        agent: &Agent,
        task: AgentTask,
        context: ExecutionContext,
    ) -> Result<TaskResult> {
        let execution_id = uuid::Uuid::new_v4().to_string();
        let started_at = Utc::now();

        // Check agent status
        if agent.status != AgentStatus::Active {
            return Ok(TaskResult {
                execution_id,
                agent_id: agent.id.clone(),
                success: false,
                output: None,
                error: Some(format!("Agent is not active (status: {:?})", agent.status)),
                tool_calls: Vec::new(),
                budget_consumed: BudgetConsumed::default(),
                started_at,
                ended_at: Utc::now(),
            });
        }

        // Initialize budget tracking for this agent if needed
        {
            let mut usage = self.budget_usage.write().await;
            usage.entry(agent.id.clone()).or_insert_with(BudgetUsageStats::default);
        }

        // Check policy before execution
        if self.config.require_policy_check {
            let decision = self.check_task_policy(agent, &task, &context).await?;
            match decision {
                PolicyDecision::Allow => {}
                PolicyDecision::Deny { reason, .. } => {
                    return Ok(TaskResult {
                        execution_id,
                        agent_id: agent.id.clone(),
                        success: false,
                        output: None,
                        error: Some(format!("Policy denied: {}", reason)),
                        tool_calls: Vec::new(),
                        budget_consumed: BudgetConsumed::default(),
                        started_at,
                        ended_at: Utc::now(),
                    });
                }
                PolicyDecision::RequiresApproval { rule_id, .. } => {
                    return Ok(TaskResult {
                        execution_id,
                        agent_id: agent.id.clone(),
                        success: false,
                        output: None,
                        error: Some(format!("Requires approval (rule: {})", rule_id)),
                        tool_calls: Vec::new(),
                        budget_consumed: BudgetConsumed::default(),
                        started_at,
                        ended_at: Utc::now(),
                    });
                }
                PolicyDecision::BudgetExceeded { budget_type, limit, current } => {
                    return Ok(TaskResult {
                        execution_id,
                        agent_id: agent.id.clone(),
                        success: false,
                        output: None,
                        error: Some(format!(
                            "Budget exceeded: {} (limit: {}, current: {})",
                            budget_type, limit, current
                        )),
                        tool_calls: Vec::new(),
                        budget_consumed: BudgetConsumed::default(),
                        started_at,
                        ended_at: Utc::now(),
                    });
                }
                PolicyDecision::Delegate { target_agent, reason } => {
                    return Ok(TaskResult {
                        execution_id,
                        agent_id: agent.id.clone(),
                        success: false,
                        output: None,
                        error: Some(format!("Delegated to {}: {}", target_agent.0, reason)),
                        tool_calls: Vec::new(),
                        budget_consumed: BudgetConsumed::default(),
                        started_at,
                        ended_at: Utc::now(),
                    });
                }
                PolicyDecision::Escalate { target, reason, .. } => {
                    return Ok(TaskResult {
                        execution_id,
                        agent_id: agent.id.clone(),
                        success: false,
                        output: None,
                        error: Some(format!("Escalated to {:?}: {}", target, reason)),
                        tool_calls: Vec::new(),
                        budget_consumed: BudgetConsumed::default(),
                        started_at,
                        ended_at: Utc::now(),
                    });
                }
            }
        }

        // Check budget before execution
        if self.config.enforce_budgets {
            if let Some(exceeded) = self.check_budget_exceeded(agent).await {
                return Ok(TaskResult {
                    execution_id,
                    agent_id: agent.id.clone(),
                    success: false,
                    output: None,
                    error: Some(exceeded),
                    tool_calls: Vec::new(),
                    budget_consumed: BudgetConsumed::default(),
                    started_at,
                    ended_at: Utc::now(),
                });
            }
        }

        // Execute the task
        let result = self.run_task_execution(agent, &task, &context).await;

        let ended_at = Utc::now();
        let (success, output, error, tool_calls, budget_consumed) = match result {
            Ok((out, calls, budget)) => (true, Some(out), None, calls, budget),
            Err(e) => (
                false,
                None,
                Some(e.to_string()),
                Vec::new(),
                BudgetConsumed::default(),
            ),
        };

        // Update budget usage
        self.record_budget_usage(agent, &budget_consumed).await;

        Ok(TaskResult {
            execution_id,
            agent_id: agent.id.clone(),
            success,
            output,
            error,
            tool_calls,
            budget_consumed,
            started_at,
            ended_at,
        })
    }

    /// Run the actual task execution.
    async fn run_task_execution(
        &self,
        agent: &Agent,
        task: &AgentTask,
        _context: &ExecutionContext,
    ) -> Result<(String, Vec<ToolCallRecord>, BudgetConsumed)> {
        let tool_calls = Vec::new();
        let mut budget = BudgetConsumed::default();

        // Get available tools for this agent
        let _tools = self.mcp_resolver.get_available_tools(&agent.mcp_bindings).await?;

        // In a full implementation, this would:
        // 1. Parse the task prompt to determine what tools to call
        // 2. Execute the appropriate tools
        // 3. Build a response from the tool results
        //
        // For now, we return a simple acknowledgment
        let output = format!(
            "Agent {} processed task: {}",
            agent.name,
            if task.prompt.len() > 100 {
                format!("{}...", &task.prompt[..100])
            } else {
                task.prompt.clone()
            }
        );

        // Track request
        budget.requests = 1;

        Ok((output, tool_calls, budget))
    }

    /// Check if the task is allowed by policy.
    async fn check_task_policy(
        &self,
        agent: &Agent,
        task: &AgentTask,
        context: &ExecutionContext,
    ) -> Result<PolicyDecision> {
        let action = AgentAction {
            action_type: ActionType::WorkflowStep,
            tool_id: task.skill.clone(),
            tool_tier: None,
            resource: None,
            parameters: serde_json::json!({
                "prompt": task.prompt,
                "context_keys": task.context.keys().collect::<Vec<_>>()
            }),
            estimated_tokens: None,
            estimated_cost_cents: None,
        };

        let action_context = ActionContext {
            timestamp: Utc::now(),
            environment: context.environment.clone(),
            budget_usage: self.get_budget_usage(agent).await,
        };

        self.policy_engine
            .evaluate(agent, &action, &action_context)
            .await
    }

    /// Check if budget limits are exceeded.
    async fn check_budget_exceeded(&self, agent: &Agent) -> Option<String> {
        let usage = self.budget_usage.read().await;
        let stats = usage.get(&agent.id)?;

        // Check token budgets
        let token_budget = &agent.budgets.tokens;
        if let Some(limit) = token_budget.per_hour {
            if stats.tokens_this_hour > limit {
                return Some(format!(
                    "Hourly token budget exceeded: {} > {}",
                    stats.tokens_this_hour, limit
                ));
            }
        }
        if let Some(limit) = token_budget.per_day {
            if stats.tokens_today > limit {
                return Some(format!(
                    "Daily token budget exceeded: {} > {}",
                    stats.tokens_today, limit
                ));
            }
        }

        // Check cost budgets
        let cost_budget = &agent.budgets.cost;
        if let Some(limit) = cost_budget.per_hour_cents {
            if stats.cost_this_hour_cents > limit {
                return Some(format!(
                    "Hourly cost budget exceeded: {} > {}",
                    stats.cost_this_hour_cents, limit
                ));
            }
        }
        if let Some(limit) = cost_budget.per_day_cents {
            if stats.cost_today_cents > limit {
                return Some(format!(
                    "Daily cost budget exceeded: {} > {}",
                    stats.cost_today_cents, limit
                ));
            }
        }

        // Check request budgets
        let req_budget = &agent.budgets.requests;
        if let Some(limit) = req_budget.per_hour {
            if stats.requests_this_hour > limit {
                return Some(format!(
                    "Hourly request budget exceeded: {} > {}",
                    stats.requests_this_hour, limit
                ));
            }
        }

        None
    }

    /// Get current budget usage for an agent.
    async fn get_budget_usage(&self, agent: &Agent) -> BudgetUsageStats {
        let usage = self.budget_usage.read().await;
        usage.get(&agent.id).cloned().unwrap_or_default()
    }

    /// Record budget consumption.
    async fn record_budget_usage(&self, agent: &Agent, consumed: &BudgetConsumed) {
        let mut usage = self.budget_usage.write().await;
        let stats = usage.entry(agent.id.clone()).or_insert_with(BudgetUsageStats::default);

        stats.tokens_this_hour += consumed.tokens_input + consumed.tokens_output;
        stats.tokens_today += consumed.tokens_input + consumed.tokens_output;
        stats.cost_this_hour_cents += consumed.cost_cents;
        stats.cost_today_cents += consumed.cost_cents;
        stats.requests_this_hour += consumed.requests as u32;
        stats.requests_today += consumed.requests as u32;
    }

    /// Reset budget usage for an agent (for testing or period reset).
    pub async fn reset_budget_usage(&self, agent_id: &AgentId) {
        let mut usage = self.budget_usage.write().await;
        usage.remove(agent_id);
    }

    /// Get current budget stats for an agent.
    pub async fn get_budget_stats(&self, agent_id: &AgentId) -> Option<BudgetUsageStats> {
        let usage = self.budget_usage.read().await;
        usage.get(agent_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{
        policy_engine::{DefaultPolicyEngine, InMemoryPolicyStorage},
        AgentBudgets, AgentOrganization, TokenBudget,
    };
    use crate::events::InMemoryEventLog;
    use crate::types::TeamId;

    fn test_agent(id: &str, name: &str) -> Agent {
        Agent::builder(AgentId::new(id), name)
            .organization(AgentOrganization::new(TeamId::new("test-team")))
            .build()
    }

    fn test_agent_with_status(id: &str, name: &str, status: AgentStatus) -> Agent {
        Agent::builder(AgentId::new(id), name)
            .organization(AgentOrganization::new(TeamId::new("test-team")))
            .status(status)
            .build()
    }

    async fn create_test_runtime() -> AgentRuntime<
        DefaultPolicyEngine<InMemoryPolicyStorage>,
        InMemoryMcpResolver,
        InMemoryToolExecutor,
        InMemoryEventLog,
    > {
        let policy_storage = InMemoryPolicyStorage::new();
        let policy_engine = DefaultPolicyEngine::new(policy_storage);

        let mcp_resolver = InMemoryMcpResolver::new();
        let tool_executor = InMemoryToolExecutor::new();
        let event_log = InMemoryEventLog::new();

        AgentRuntime::new(
            RuntimeConfig::default(),
            Arc::new(policy_engine),
            Arc::new(mcp_resolver),
            Arc::new(tool_executor),
            Arc::new(event_log),
        )
    }

    #[tokio::test]
    async fn test_execute_task_active_agent() {
        let runtime = create_test_runtime().await;

        let agent = test_agent("test-agent", "Test Agent");

        let task = AgentTask {
            id: "task-1".to_string(),
            prompt: "Do something".to_string(),
            skill: None,
            context: HashMap::new(),
            run_id: None,
            step_id: None,
        };

        let result = runtime
            .execute_task(&agent, task, ExecutionContext::default())
            .await
            .unwrap();

        assert!(result.success);
        assert!(result.output.is_some());
    }

    #[tokio::test]
    async fn test_execute_task_paused_agent() {
        let runtime = create_test_runtime().await;

        let agent = test_agent_with_status("test-agent", "Test Agent", AgentStatus::Paused);

        let task = AgentTask {
            id: "task-1".to_string(),
            prompt: "Do something".to_string(),
            skill: None,
            context: HashMap::new(),
            run_id: None,
            step_id: None,
        };

        let result = runtime
            .execute_task(&agent, task, ExecutionContext::default())
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.unwrap().contains("not active"));
    }

    #[tokio::test]
    async fn test_budget_tracking() {
        let runtime = create_test_runtime().await;

        let agent = test_agent("test-agent", "Test Agent");

        // Execute a task
        let task = AgentTask {
            id: "task-1".to_string(),
            prompt: "Do something".to_string(),
            skill: None,
            context: HashMap::new(),
            run_id: None,
            step_id: None,
        };

        runtime
            .execute_task(&agent, task, ExecutionContext::default())
            .await
            .unwrap();

        // Check budget was tracked
        let stats = runtime.get_budget_stats(&agent.id).await;
        assert!(stats.is_some());
        assert!(stats.unwrap().requests_this_hour >= 1);
    }

    #[tokio::test]
    async fn test_budget_exceeded() {
        let mut config = RuntimeConfig::default();
        config.enforce_budgets = true;

        let policy_storage = InMemoryPolicyStorage::new();
        let policy_engine = DefaultPolicyEngine::new(policy_storage);
        let mcp_resolver = InMemoryMcpResolver::new();
        let tool_executor = InMemoryToolExecutor::new();
        let event_log = InMemoryEventLog::new();

        let runtime = AgentRuntime::new(
            config,
            Arc::new(policy_engine),
            Arc::new(mcp_resolver),
            Arc::new(tool_executor),
            Arc::new(event_log),
        );

        // Create agent with very restrictive budget
        let agent = Agent::builder(AgentId::new("test-agent"), "Test Agent")
            .organization(AgentOrganization::new(TeamId::new("test-team")))
            .budgets(AgentBudgets::new().with_tokens(
                TokenBudget::new()
                    .with_per_request(10)
                    .with_per_hour(1) // Very low limit
                    .with_per_day(10),
            ))
            .build();

        // Manually set high usage to trigger budget exceeded
        {
            let mut usage = runtime.budget_usage.write().await;
            usage.insert(
                agent.id.clone(),
                BudgetUsageStats {
                    tokens_this_hour: 100,
                    ..Default::default()
                },
            );
        }

        let task = AgentTask {
            id: "task-1".to_string(),
            prompt: "Do something".to_string(),
            skill: None,
            context: HashMap::new(),
            run_id: None,
            step_id: None,
        };

        let result = runtime
            .execute_task(&agent, task, ExecutionContext::default())
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("Budget exceeded"));
    }
}
