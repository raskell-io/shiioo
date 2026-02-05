//! Agent Runtime and Orchestration API handlers.

use super::ApiResult;
use crate::config::AppState;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use chrono::Utc;
use shiioo_core::agent::{
    AgentId, AgentPolicies, AgentTask, BudgetConsumed, DelegationRequest,
    ExecutionContext, SupervisionEvent, SupervisionStrategy,
};
use shiioo_core::storage::AgentStore;
use shiioo_core::types::{RunId, StepId};
use std::collections::HashMap;
use std::sync::Arc;

// ============================================================================
// Task Execution Endpoints
// ============================================================================

/// Execute a task for an agent
pub async fn execute_task(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(req): Json<ExecuteTaskRequest>,
) -> ApiResult<Json<ExecuteTaskResponse>> {
    let agent_id = AgentId::new(&agent_id);

    // Get the agent
    let agent = state
        .agent_store
        .get_agent(&agent_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Agent not found: {}", agent_id.0))?;

    // Create the task
    let run_id = if let Some(id) = &req.run_id {
        Some(RunId(uuid::Uuid::parse_str(id).map_err(|e| anyhow::anyhow!("Invalid run_id UUID: {}", e))?))
    } else {
        None
    };

    let task = AgentTask {
        id: uuid::Uuid::new_v4().to_string(),
        prompt: req.prompt.clone(),
        skill: req.skill.clone(),
        context: req.context.unwrap_or_default(),
        run_id,
        step_id: req.step_id.clone(),
    };

    // Create execution context
    let context = ExecutionContext {
        run_id: task.run_id.clone(),
        step_id: task.step_id.clone(),
        environment: "production".to_string(),
        parent_agent: None,
        delegation_chain: Vec::new(),
    };

    // Execute the task
    let result = state.agent_runtime.execute_task(&agent, task, context).await?;

    tracing::info!(
        "Executed task for agent {} - success: {}, tool_calls: {}",
        agent_id.0,
        result.success,
        result.tool_calls.len()
    );

    Ok(Json(ExecuteTaskResponse {
        execution_id: result.execution_id,
        success: result.success,
        output: result.output,
        error: result.error,
        tool_calls: result.tool_calls.len(),
        budget_consumed: BudgetConsumedDto::from(result.budget_consumed),
    }))
}

#[derive(Debug, Deserialize)]
pub struct ExecuteTaskRequest {
    pub prompt: String,
    pub skill: Option<String>,
    pub context: Option<HashMap<String, serde_json::Value>>,
    pub run_id: Option<String>,
    pub step_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ExecuteTaskResponse {
    pub execution_id: String,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
    pub tool_calls: usize,
    pub budget_consumed: BudgetConsumedDto,
}

#[derive(Debug, Serialize, Default)]
pub struct BudgetConsumedDto {
    pub tokens_input: u64,
    pub tokens_output: u64,
    pub cost_cents: u64,
    pub requests: u64,
    pub tool_calls: u64,
}

impl From<BudgetConsumed> for BudgetConsumedDto {
    fn from(b: BudgetConsumed) -> Self {
        Self {
            tokens_input: b.tokens_input,
            tokens_output: b.tokens_output,
            cost_cents: b.cost_cents,
            requests: b.requests,
            tool_calls: b.tool_calls,
        }
    }
}

/// Get agent's current budget status
pub async fn get_agent_budget(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> ApiResult<Json<AgentBudgetResponse>> {
    let agent_id = AgentId::new(&agent_id);

    let agent = state
        .agent_store
        .get_agent(&agent_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Agent not found: {}", agent_id.0))?;

    Ok(Json(AgentBudgetResponse {
        agent_id: agent_id.0.clone(),
        budgets: agent.budgets,
    }))
}

#[derive(Debug, Serialize)]
pub struct AgentBudgetResponse {
    pub agent_id: String,
    pub budgets: shiioo_core::agent::AgentBudgets,
}

// ============================================================================
// Delegation Endpoints
// ============================================================================

/// Delegate a task from one agent to another
pub async fn delegate_task(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(req): Json<DelegateTaskRequest>,
) -> ApiResult<Json<DelegateTaskResponse>> {
    let from_agent_id = AgentId::new(&agent_id);
    let to_agent_id = AgentId::new(&req.to_agent_id);

    let request = DelegationRequest {
        id: uuid::Uuid::new_v4().to_string(),
        from_agent: from_agent_id.clone(),
        to_agent: to_agent_id.clone(),
        task: AgentTask {
            id: uuid::Uuid::new_v4().to_string(),
            prompt: req.prompt.clone(),
            skill: req.skill.clone(),
            context: req.context.unwrap_or_default(),
            run_id: None,
            step_id: None,
        },
        reason: req.reason.clone(),
        propagated_policies: None,
        budget_allocation: None,
        depth: 0,
        chain: vec![from_agent_id.clone()],
        created_at: Utc::now(),
    };

    let result = state.agent_orchestrator.delegate(request).await?;

    tracing::info!(
        "Delegated task from {} to {} - success: {}",
        from_agent_id.0,
        to_agent_id.0,
        result.success
    );

    Ok(Json(DelegateTaskResponse {
        delegation_id: result.delegation_id,
        success: result.success,
        task_result: result.task_result.map(|r| TaskResultDto {
            execution_id: r.execution_id,
            success: r.success,
            output: r.output,
            error: r.error,
        }),
        error: result.error,
    }))
}

#[derive(Debug, Deserialize)]
pub struct DelegateTaskRequest {
    pub to_agent_id: String,
    pub prompt: String,
    pub skill: Option<String>,
    pub context: Option<HashMap<String, serde_json::Value>>,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct DelegateTaskResponse {
    pub delegation_id: String,
    pub success: bool,
    pub task_result: Option<TaskResultDto>,
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TaskResultDto {
    pub execution_id: String,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
}

// ============================================================================
// Team Management Endpoints
// ============================================================================

/// Create a new agent team
pub async fn create_team(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateTeamRequest>,
) -> ApiResult<Json<TeamResponse>> {
    let lead_id = AgentId::new(&req.lead_id);
    let member_ids: Vec<AgentId> = req
        .member_ids
        .iter()
        .map(|id| AgentId::new(id))
        .collect();

    let team = shiioo_core::agent::AgentTeam {
        id: req.id.clone(),
        name: req.name.clone(),
        lead: lead_id.clone(),
        members: member_ids.clone(),
        policies: AgentPolicies::default(),
        channel: shiioo_core::agent::TeamChannel::default(),
    };

    state.agent_orchestrator.create_team(team).await?;

    tracing::info!(
        "Created team {} with {} members",
        req.id,
        member_ids.len()
    );

    Ok(Json(TeamResponse {
        id: req.id,
        name: req.name,
        lead_id: lead_id.0.clone(),
        member_ids: member_ids.iter().map(|id| id.0.clone()).collect(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct CreateTeamRequest {
    pub id: String,
    pub name: String,
    pub lead_id: String,
    pub member_ids: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TeamResponse {
    pub id: String,
    pub name: String,
    pub lead_id: String,
    pub member_ids: Vec<String>,
}

/// Get a team by ID
pub async fn get_team(
    State(state): State<Arc<AppState>>,
    Path(team_id): Path<String>,
) -> ApiResult<Json<TeamResponse>> {
    let team = state
        .agent_orchestrator
        .get_team(&team_id)
        .await
        .ok_or_else(|| anyhow::anyhow!("Team not found: {}", team_id))?;

    Ok(Json(TeamResponse {
        id: team.id.clone(),
        name: team.name.clone(),
        lead_id: team.lead.0.clone(),
        member_ids: team.members.iter().map(|id| id.0.clone()).collect(),
    }))
}

/// List all teams
pub async fn list_teams(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListTeamsResponse>> {
    let teams = state.agent_orchestrator.list_teams().await;

    let team_responses: Vec<TeamResponse> = teams
        .iter()
        .map(|t| TeamResponse {
            id: t.id.clone(),
            name: t.name.clone(),
            lead_id: t.lead.0.clone(),
            member_ids: t.members.iter().map(|id| id.0.clone()).collect(),
        })
        .collect();

    Ok(Json(ListTeamsResponse { teams: team_responses }))
}

#[derive(Debug, Serialize)]
pub struct ListTeamsResponse {
    pub teams: Vec<TeamResponse>,
}

/// Broadcast a task to all team members
pub async fn broadcast_to_team(
    State(state): State<Arc<AppState>>,
    Path(team_id): Path<String>,
    Json(req): Json<BroadcastRequest>,
) -> ApiResult<Json<BroadcastResponse>> {
    let task = AgentTask {
        id: uuid::Uuid::new_v4().to_string(),
        prompt: req.prompt.clone(),
        skill: req.skill.clone(),
        context: req.context.unwrap_or_default(),
        run_id: None,
        step_id: None,
    };

    let results = state
        .agent_orchestrator
        .broadcast_to_team(&team_id, task)
        .await?;

    tracing::info!(
        "Broadcast task to team {} - {} agents executed",
        team_id,
        results.len()
    );

    Ok(Json(BroadcastResponse {
        team_id,
        results: results
            .into_iter()
            .map(|r| TaskResultDto {
                execution_id: r.execution_id,
                success: r.success,
                output: r.output,
                error: r.error,
            })
            .collect(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct BroadcastRequest {
    pub prompt: String,
    pub skill: Option<String>,
    pub context: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Serialize)]
pub struct BroadcastResponse {
    pub team_id: String,
    pub results: Vec<TaskResultDto>,
}

// ============================================================================
// Supervision Endpoints
// ============================================================================

/// Create a supervisor for an agent
pub async fn create_supervisor(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(req): Json<CreateSupervisorRequest>,
) -> ApiResult<Json<SupervisorResponse>> {
    let agent_id = AgentId::new(&agent_id);

    let strategy = match req.strategy.to_lowercase().as_str() {
        "restart" => SupervisionStrategy::Restart,
        "stop" => SupervisionStrategy::Stop,
        "escalate" => SupervisionStrategy::Escalate,
        "resume" => SupervisionStrategy::Resume,
        "delegate" => SupervisionStrategy::Delegate,
        _ => return Err(anyhow::anyhow!("Invalid supervision strategy: {}", req.strategy).into()),
    };

    state
        .agent_orchestrator
        .supervisor()
        .supervise(agent_id.clone(), strategy.clone())
        .await;

    tracing::info!("Created supervisor for agent {} with strategy {:?}", agent_id.0, strategy);

    Ok(Json(SupervisorResponse {
        agent_id: agent_id.0.clone(),
        strategy: req.strategy,
        created: true,
    }))
}

#[derive(Debug, Deserialize)]
pub struct CreateSupervisorRequest {
    pub strategy: String,
}

#[derive(Debug, Serialize)]
pub struct SupervisorResponse {
    pub agent_id: String,
    pub strategy: String,
    pub created: bool,
}

/// Get supervision events for an agent
pub async fn get_supervision_events(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Query(_params): Query<SupervisionEventsParams>,
) -> ApiResult<Json<SupervisionEventsResponse>> {
    let agent_id = AgentId::new(&agent_id);

    let events: Vec<SupervisionEvent> = state
        .agent_orchestrator
        .supervisor()
        .get_events(&agent_id)
        .await;

    let event_dtos: Vec<SupervisionEventDto> = events
        .iter()
        .map(|e| SupervisionEventDto {
            timestamp: e.timestamp.to_rfc3339(),
            agent_id: e.agent_id.0.clone(),
            event_type: format!("{:?}", e.event_type),
            details: e.details.clone(),
        })
        .collect();

    Ok(Json(SupervisionEventsResponse { events: event_dtos }))
}

#[derive(Debug, Deserialize)]
pub struct SupervisionEventsParams {
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SupervisionEventsResponse {
    pub events: Vec<SupervisionEventDto>,
}

#[derive(Debug, Serialize)]
pub struct SupervisionEventDto {
    pub timestamp: String,
    pub agent_id: String,
    pub event_type: String,
    pub details: Option<String>,
}

// ============================================================================
// Workflow Adapter Endpoints
// ============================================================================

/// Execute a workflow step as an agent task
pub async fn execute_workflow_step(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ExecuteWorkflowStepRequest>,
) -> ApiResult<Json<ExecuteWorkflowStepResponse>> {
    let agent_id = AgentId::new(&req.agent_id);
    let run_uuid = uuid::Uuid::parse_str(&req.run_id)
        .map_err(|e| anyhow::anyhow!("Invalid run_id UUID: {}", e))?;
    let run_id = RunId(run_uuid);
    let step_id = StepId::new(&req.step_id);

    // Execute through the orchestrator's workflow step adapter
    let result = state
        .agent_orchestrator
        .execute_workflow_step(&agent_id, &step_id, &run_id, req.prompt.clone())
        .await?;

    tracing::info!(
        "Executed workflow step {}/{} via agent {} - success: {}",
        req.run_id,
        req.step_id,
        agent_id.0,
        result.success
    );

    Ok(Json(ExecuteWorkflowStepResponse {
        run_id: req.run_id,
        step_id: req.step_id,
        success: result.success,
        output: result.output,
        error: result.error,
    }))
}

#[derive(Debug, Deserialize)]
pub struct ExecuteWorkflowStepRequest {
    pub run_id: String,
    pub step_id: String,
    pub agent_id: String,
    pub prompt: String,
}

#[derive(Debug, Serialize)]
pub struct ExecuteWorkflowStepResponse {
    pub run_id: String,
    pub step_id: String,
    pub success: bool,
    pub output: Option<String>,
    pub error: Option<String>,
}
