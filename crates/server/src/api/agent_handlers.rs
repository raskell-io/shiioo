//! Employee and Job Title API handlers.

use super::ApiResult;
use crate::config::AppState;
use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};
use shiioo_core::{
    agent::{
        Agent, AgentBudgets, AgentId, AgentOrganization, AgentPolicies, AgentSkills, AgentStatus,
        Archetype, ArchetypeId, McpBinding, SecretGrant,
    },
    storage::AgentStore,
    types::TeamId,
};
use std::sync::Arc;

// ============================================================================
// Agent Endpoints
// ============================================================================

/// List all employees
pub async fn list_agents(
    State(state): State<Arc<AppState>>,
    Query(params): Query<ListAgentsParams>,
) -> ApiResult<Json<ListAgentsResponse>> {
    let agents = if let Some(status) = params.status {
        let status = parse_agent_status(&status)?;
        state.agent_store.list_agents_by_status(status).await?
    } else if let Some(team_id) = params.team_id {
        state.agent_store.list_agents_by_team(&team_id).await?
    } else {
        state.agent_store.list_agents().await?
    };

    Ok(Json(ListAgentsResponse { agents }))
}

#[derive(Debug, Deserialize)]
pub struct ListAgentsParams {
    pub status: Option<String>,
    pub team_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ListAgentsResponse {
    pub agents: Vec<Agent>,
}

/// Get an employee by ID
pub async fn get_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> ApiResult<Json<Agent>> {
    let agent = state
        .agent_store
        .get_agent(&AgentId::new(&agent_id))
        .await?
        .ok_or_else(|| anyhow::anyhow!("Employee not found: {}", agent_id))?;

    Ok(Json(agent))
}

/// Hire a new employee
pub async fn create_agent(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateAgentRequest>,
) -> ApiResult<Json<Agent>> {
    // Check if agent already exists
    if state
        .agent_store
        .get_agent(&AgentId::new(&req.id))
        .await?
        .is_some()
    {
        return Err(anyhow::anyhow!("Employee already exists: {}", req.id).into());
    }

    let mut builder = Agent::builder(AgentId::new(&req.id), &req.name)
        .description(&req.description)
        .organization(AgentOrganization {
            team: TeamId::new(&req.team_id),
            reports_to: req.reports_to.map(AgentId::new),
            supervises: req
                .supervises
                .unwrap_or_default()
                .into_iter()
                .map(AgentId::new)
                .collect(),
            peers: req
                .peers
                .unwrap_or_default()
                .into_iter()
                .map(AgentId::new)
                .collect(),
        })
        .skills(req.skills.unwrap_or_default())
        .policies(req.policies.unwrap_or_default())
        .mcp_bindings(req.mcp_bindings.unwrap_or_default())
        .secret_grants(req.secret_grants.unwrap_or_default())
        .budgets(req.budgets.unwrap_or_default());

    if let Some(archetype_id) = req.archetype_id {
        builder = builder.archetype(ArchetypeId::new(archetype_id));
    }

    let agent = builder.build();

    state.agent_store.store_agent(&agent).await?;

    tracing::info!("Hired employee: {} ({})", agent.name, agent.id);

    Ok(Json(agent))
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub team_id: String,
    pub archetype_id: Option<String>,
    pub reports_to: Option<String>,
    pub supervises: Option<Vec<String>>,
    pub peers: Option<Vec<String>>,
    pub skills: Option<AgentSkills>,
    pub policies: Option<AgentPolicies>,
    pub mcp_bindings: Option<Vec<McpBinding>>,
    pub secret_grants: Option<Vec<SecretGrant>>,
    pub budgets: Option<AgentBudgets>,
}

/// Update an employee
pub async fn update_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
    Json(req): Json<UpdateAgentRequest>,
) -> ApiResult<Json<Agent>> {
    let mut agent = state
        .agent_store
        .get_agent(&AgentId::new(&agent_id))
        .await?
        .ok_or_else(|| anyhow::anyhow!("Employee not found: {}", agent_id))?;

    // Update fields if provided
    if let Some(name) = req.name {
        agent.name = name;
    }
    if let Some(description) = req.description {
        agent.description = description;
    }
    if let Some(archetype_id) = req.archetype_id {
        agent.archetype = Some(ArchetypeId::new(archetype_id));
    }
    if let Some(skills) = req.skills {
        agent.skills = skills;
    }
    if let Some(policies) = req.policies {
        agent.policies = policies;
    }
    if let Some(mcp_bindings) = req.mcp_bindings {
        agent.mcp_bindings = mcp_bindings;
    }
    if let Some(secret_grants) = req.secret_grants {
        agent.secret_grants = secret_grants;
    }
    if let Some(budgets) = req.budgets {
        agent.budgets = budgets;
    }

    agent.updated_at = chrono::Utc::now();

    state.agent_store.store_agent(&agent).await?;

    tracing::info!("Updated employee: {} ({})", agent.name, agent.id);

    Ok(Json(agent))
}

#[derive(Debug, Deserialize)]
pub struct UpdateAgentRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub archetype_id: Option<String>,
    pub skills: Option<AgentSkills>,
    pub policies: Option<AgentPolicies>,
    pub mcp_bindings: Option<Vec<McpBinding>>,
    pub secret_grants: Option<Vec<SecretGrant>>,
    pub budgets: Option<AgentBudgets>,
}

/// Offboard an employee
pub async fn delete_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> ApiResult<Json<DeleteAgentResponse>> {
    let deleted = state
        .agent_store
        .delete_agent(&AgentId::new(&agent_id))
        .await?;

    if deleted {
        tracing::info!("Offboarded employee: {}", agent_id);
    }

    Ok(Json(DeleteAgentResponse { deleted }))
}

#[derive(Debug, Serialize)]
pub struct DeleteAgentResponse {
    pub deleted: bool,
}

/// Activate an employee
pub async fn activate_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> ApiResult<Json<Agent>> {
    state
        .agent_store
        .update_agent_status(&AgentId::new(&agent_id), AgentStatus::Active)
        .await?;

    let agent = state
        .agent_store
        .get_agent(&AgentId::new(&agent_id))
        .await?
        .ok_or_else(|| anyhow::anyhow!("Employee not found: {}", agent_id))?;

    tracing::info!("Activated employee: {}", agent_id);

    Ok(Json(agent))
}

/// Place an employee on leave
pub async fn suspend_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> ApiResult<Json<Agent>> {
    state
        .agent_store
        .update_agent_status(&AgentId::new(&agent_id), AgentStatus::Suspended)
        .await?;

    let agent = state
        .agent_store
        .get_agent(&AgentId::new(&agent_id))
        .await?
        .ok_or_else(|| anyhow::anyhow!("Employee not found: {}", agent_id))?;

    tracing::info!("Placed employee on leave: {}", agent_id);

    Ok(Json(agent))
}

/// Pause an employee
pub async fn pause_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> ApiResult<Json<Agent>> {
    state
        .agent_store
        .update_agent_status(&AgentId::new(&agent_id), AgentStatus::Paused)
        .await?;

    let agent = state
        .agent_store
        .get_agent(&AgentId::new(&agent_id))
        .await?
        .ok_or_else(|| anyhow::anyhow!("Employee not found: {}", agent_id))?;

    tracing::info!("Paused employee: {}", agent_id);

    Ok(Json(agent))
}

/// Offboard an employee (soft delete)
pub async fn archive_agent(
    State(state): State<Arc<AppState>>,
    Path(agent_id): Path<String>,
) -> ApiResult<Json<Agent>> {
    state
        .agent_store
        .update_agent_status(&AgentId::new(&agent_id), AgentStatus::Archived)
        .await?;

    let agent = state
        .agent_store
        .get_agent(&AgentId::new(&agent_id))
        .await?
        .ok_or_else(|| anyhow::anyhow!("Employee not found: {}", agent_id))?;

    tracing::info!("Offboarded employee: {}", agent_id);

    Ok(Json(agent))
}

// ============================================================================
// Job Title Endpoints
// ============================================================================

/// List all job titles
pub async fn list_archetypes(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListArchetypesResponse>> {
    let archetypes = state.agent_store.list_archetypes().await?;
    Ok(Json(ListArchetypesResponse { archetypes }))
}

#[derive(Debug, Serialize)]
pub struct ListArchetypesResponse {
    pub archetypes: Vec<Archetype>,
}

/// Get a job title by ID
pub async fn get_archetype(
    State(state): State<Arc<AppState>>,
    Path(archetype_id): Path<String>,
) -> ApiResult<Json<Archetype>> {
    let archetype = state
        .agent_store
        .get_archetype(&ArchetypeId::new(&archetype_id))
        .await?
        .ok_or_else(|| anyhow::anyhow!("Job title not found: {}", archetype_id))?;

    Ok(Json(archetype))
}

/// Create a new job title
pub async fn create_archetype(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateArchetypeRequest>,
) -> ApiResult<Json<Archetype>> {
    // Check if archetype already exists
    if state
        .agent_store
        .get_archetype(&ArchetypeId::new(&req.id))
        .await?
        .is_some()
    {
        return Err(anyhow::anyhow!("Job title already exists: {}", req.id).into());
    }

    let mut builder = Archetype::builder(req.id.clone(), req.name.clone())
        .description(req.description.clone())
        .created_by(req.created_by.as_deref().unwrap_or("api"));

    if let Some(parent_id) = req.parent_id {
        builder = builder.parent(ArchetypeId::new(parent_id));
    }
    if let Some(skills) = req.skills {
        builder = builder.skills(skills);
    }
    if let Some(policies) = req.policies {
        builder = builder.policies(policies);
    }
    if let Some(mcp_bindings) = req.mcp_bindings {
        builder = builder.mcp_bindings(mcp_bindings);
    }
    if let Some(budgets) = req.budgets {
        builder = builder.budgets(budgets);
    }

    let archetype = builder.build();

    state.agent_store.store_archetype(&archetype).await?;

    tracing::info!("Created job title: {} ({})", archetype.name, archetype.id);

    Ok(Json(archetype))
}

#[derive(Debug, Deserialize)]
pub struct CreateArchetypeRequest {
    pub id: String,
    pub name: String,
    pub description: String,
    pub parent_id: Option<String>,
    pub skills: Option<AgentSkills>,
    pub policies: Option<AgentPolicies>,
    pub mcp_bindings: Option<Vec<McpBinding>>,
    pub budgets: Option<AgentBudgets>,
    pub created_by: Option<String>,
}

/// Update a job title
pub async fn update_archetype(
    State(state): State<Arc<AppState>>,
    Path(archetype_id): Path<String>,
    Json(req): Json<UpdateArchetypeRequest>,
) -> ApiResult<Json<Archetype>> {
    let existing = state
        .agent_store
        .get_archetype(&ArchetypeId::new(&archetype_id))
        .await?
        .ok_or_else(|| anyhow::anyhow!("Job title not found: {}", archetype_id))?;

    // Rebuild archetype with updated values
    let name = req.name.unwrap_or_else(|| existing.name.clone());
    let description = req.description.unwrap_or_else(|| existing.description.clone());
    let mut builder = Archetype::builder(existing.id.0.clone(), name)
        .description(description)
        .created_by(&existing.created_by);

    // Use new parent if provided, else keep existing
    if let Some(parent_id) = req.parent_id {
        builder = builder.parent(ArchetypeId::new(parent_id));
    } else if let Some(ref parent) = existing.parent {
        builder = builder.parent(parent.clone());
    }

    builder = builder
        .skills(req.skills.unwrap_or(existing.skills))
        .policies(req.policies.unwrap_or(existing.policies))
        .mcp_bindings(req.mcp_bindings.unwrap_or(existing.mcp_bindings))
        .budgets(req.budgets.unwrap_or(existing.budgets));

    let mut archetype = builder.build();
    archetype.created_at = existing.created_at; // Preserve original creation time

    state.agent_store.store_archetype(&archetype).await?;

    tracing::info!("Updated job title: {} ({})", archetype.name, archetype.id);

    Ok(Json(archetype))
}

#[derive(Debug, Deserialize)]
pub struct UpdateArchetypeRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub parent_id: Option<String>,
    pub skills: Option<AgentSkills>,
    pub policies: Option<AgentPolicies>,
    pub mcp_bindings: Option<Vec<McpBinding>>,
    pub budgets: Option<AgentBudgets>,
}

/// Delete a job title
pub async fn delete_archetype(
    State(state): State<Arc<AppState>>,
    Path(archetype_id): Path<String>,
) -> ApiResult<Json<DeleteArchetypeResponse>> {
    let deleted = state
        .agent_store
        .delete_archetype(&ArchetypeId::new(&archetype_id))
        .await?;

    if deleted {
        tracing::info!("Deleted job title: {}", archetype_id);
    }

    Ok(Json(DeleteArchetypeResponse { deleted }))
}

#[derive(Debug, Serialize)]
pub struct DeleteArchetypeResponse {
    pub deleted: bool,
}

// ============================================================================
// Helper Functions
// ============================================================================

fn parse_agent_status(status: &str) -> Result<AgentStatus, anyhow::Error> {
    match status.to_lowercase().as_str() {
        "active" => Ok(AgentStatus::Active),
        "paused" => Ok(AgentStatus::Paused),
        "suspended" => Ok(AgentStatus::Suspended),
        "archived" => Ok(AgentStatus::Archived),
        _ => Err(anyhow::anyhow!("Invalid employee status: {}", status)),
    }
}
