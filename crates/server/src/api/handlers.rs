use super::{ApiResult, ErrorResponse};
use crate::config::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use shiioo_core::{
    claude_compiler::ClaudeCompiler,
    events::EventLog,
    organization::OrganizationManager,
    template::TemplateProcessor,
    types::{
        ApprovalBoard, ApprovalBoardId, ApprovalId, CapacitySource, CapacitySourceId,
        ConfigChange, ConfigChangeId, ConfigChangeType, Job, OrgId, Organization, PersonId,
        PolicyId, PolicySpec, ProcessTemplate, Routine, RoutineId, RoutineSchedule, RoleId,
        RoleSpec, Run, RunId, TemplateId, TemplateInstance, VoteDecision, WorkflowSpec,
    },
};
use std::sync::Arc;

/// List all runs
pub async fn list_runs(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListRunsResponse>> {
    let runs = state.index_store.list_runs()?;
    Ok(Json(ListRunsResponse { runs }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListRunsResponse {
    pub runs: Vec<Run>,
}

/// Get a specific run
pub async fn get_run(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> ApiResult<Json<Run>> {
    let run_id = RunId(
        run_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid run ID"))?,
    );

    let run = state
        .index_store
        .get_run(&run_id)?
        .ok_or_else(|| anyhow::anyhow!("Run not found"))?;

    Ok(Json(run))
}

/// Get events for a run
pub async fn get_run_events(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> ApiResult<Json<GetRunEventsResponse>> {
    let run_id = RunId(
        run_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid run ID"))?,
    );

    let events = state.event_log.get_run_events(run_id).await?;

    Ok(Json(GetRunEventsResponse { events }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetRunEventsResponse {
    pub events: Vec<shiioo_core::events::Event>,
}

/// Create a new job
pub async fn create_job(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateJobRequest>,
) -> ApiResult<Json<CreateJobResponse>> {
    let job = Job {
        id: uuid::Uuid::new_v4().to_string(),
        name: req.name.clone(),
        description: req.description.clone(),
        workflow: req.workflow.clone(),
        created_at: chrono::Utc::now(),
        created_by: req.created_by.clone().unwrap_or_else(|| "system".to_string()),
    };

    tracing::info!("Created job: {} ({})", job.name, job.id);

    // Execute the workflow asynchronously if requested
    let run_id = if req.execute.unwrap_or(true) {
        let executor = state.workflow_executor.clone();
        let work_item_id = job.id.clone();
        let workflow = req.workflow;

        // Spawn execution in background
        let run = executor.execute(work_item_id, workflow).await?;

        tracing::info!("Started workflow execution: run_id={}", run.id);
        Some(run.id)
    } else {
        None
    };

    Ok(Json(CreateJobResponse {
        job_id: job.id,
        run_id,
        message: if run_id.is_some() {
            "Job created and execution started".to_string()
        } else {
            "Job created".to_string()
        },
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateJobRequest {
    pub name: String,
    pub description: Option<String>,
    pub workflow: WorkflowSpec,
    pub created_by: Option<String>,
    /// Whether to execute the job immediately (default: true)
    pub execute: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateJobResponse {
    pub job_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    pub message: String,
}

// === Role Management Endpoints ===

/// List all roles
pub async fn list_roles(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListRolesResponse>> {
    let roles = state.index_store.list_roles()?;
    Ok(Json(ListRolesResponse { roles }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListRolesResponse {
    pub roles: Vec<RoleSpec>,
}

/// Get a specific role
pub async fn get_role(
    State(state): State<Arc<AppState>>,
    Path(role_id): Path<String>,
) -> ApiResult<Json<RoleSpec>> {
    let role_id = RoleId::new(role_id);

    let role = state
        .index_store
        .get_role(&role_id)?
        .ok_or_else(|| anyhow::anyhow!("Role not found"))?;

    Ok(Json(role))
}

/// Create or update a role
pub async fn create_role(
    State(state): State<Arc<AppState>>,
    Json(role): Json<RoleSpec>,
) -> ApiResult<Json<CreateRoleResponse>> {
    state.index_store.store_role(&role)?;

    tracing::info!("Created/updated role: {} ({})", role.name, role.id.0);

    Ok(Json(CreateRoleResponse {
        role_id: role.id.0.clone(),
        message: "Role created/updated successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateRoleResponse {
    pub role_id: String,
    pub message: String,
}

/// Delete a role
pub async fn delete_role(
    State(state): State<Arc<AppState>>,
    Path(role_id): Path<String>,
) -> ApiResult<Json<DeleteRoleResponse>> {
    let role_id = RoleId::new(role_id);

    state.index_store.delete_role(&role_id)?;

    tracing::info!("Deleted role: {}", role_id.0);

    Ok(Json(DeleteRoleResponse {
        message: "Role deleted successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteRoleResponse {
    pub message: String,
}

// === Policy Management Endpoints ===

/// List all policies
pub async fn list_policies(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListPoliciesResponse>> {
    let policies = state.index_store.list_policies()?;
    Ok(Json(ListPoliciesResponse { policies }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListPoliciesResponse {
    pub policies: Vec<PolicySpec>,
}

/// Get a specific policy
pub async fn get_policy(
    State(state): State<Arc<AppState>>,
    Path(policy_id): Path<String>,
) -> ApiResult<Json<PolicySpec>> {
    let policy_id = PolicyId(policy_id);

    let policy = state
        .index_store
        .get_policy(&policy_id)?
        .ok_or_else(|| anyhow::anyhow!("Policy not found"))?;

    Ok(Json(policy))
}

/// Create or update a policy
pub async fn create_policy(
    State(state): State<Arc<AppState>>,
    Json(policy): Json<PolicySpec>,
) -> ApiResult<Json<CreatePolicyResponse>> {
    state.index_store.store_policy(&policy)?;

    tracing::info!("Created/updated policy: {} ({})", policy.name, policy.id.0);

    Ok(Json(CreatePolicyResponse {
        policy_id: policy.id.0.clone(),
        message: "Policy created/updated successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreatePolicyResponse {
    pub policy_id: String,
    pub message: String,
}

/// Delete a policy
pub async fn delete_policy(
    State(state): State<Arc<AppState>>,
    Path(policy_id): Path<String>,
) -> ApiResult<Json<DeletePolicyResponse>> {
    let policy_id = PolicyId(policy_id);

    state.index_store.delete_policy(&policy_id)?;

    tracing::info!("Deleted policy: {}", policy_id.0);

    Ok(Json(DeletePolicyResponse {
        message: "Policy deleted successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeletePolicyResponse {
    pub message: String,
}

// === Organization Management Endpoints ===

/// List all organizations
pub async fn list_organizations(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListOrganizationsResponse>> {
    let orgs = state.index_store.list_organizations()?;
    Ok(Json(ListOrganizationsResponse { organizations: orgs }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListOrganizationsResponse {
    pub organizations: Vec<Organization>,
}

/// Get a specific organization
pub async fn get_organization(
    State(state): State<Arc<AppState>>,
    Path(org_id): Path<String>,
) -> ApiResult<Json<Organization>> {
    let org_id = OrgId::new(org_id);

    let org = state
        .index_store
        .get_organization(&org_id)?
        .ok_or_else(|| anyhow::anyhow!("Organization not found"))?;

    Ok(Json(org))
}

/// Create or update an organization
pub async fn create_organization(
    State(state): State<Arc<AppState>>,
    Json(org): Json<Organization>,
) -> ApiResult<Json<CreateOrganizationResponse>> {
    // Validate organization structure
    OrganizationManager::new(org.clone())?;

    state.index_store.store_organization(&org)?;

    tracing::info!("Created/updated organization: {} ({})", org.name, org.id.0);

    Ok(Json(CreateOrganizationResponse {
        org_id: org.id.0.clone(),
        message: "Organization created/updated successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateOrganizationResponse {
    pub org_id: String,
    pub message: String,
}

/// Delete an organization
pub async fn delete_organization(
    State(state): State<Arc<AppState>>,
    Path(org_id): Path<String>,
) -> ApiResult<Json<DeleteOrganizationResponse>> {
    let org_id = OrgId::new(org_id);

    state.index_store.delete_organization(&org_id)?;

    tracing::info!("Deleted organization: {}", org_id.0);

    Ok(Json(DeleteOrganizationResponse {
        message: "Organization deleted successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteOrganizationResponse {
    pub message: String,
}

// === Template Management Endpoints ===

/// List all templates
pub async fn list_templates(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListTemplatesResponse>> {
    let templates = state.index_store.list_templates()?;
    Ok(Json(ListTemplatesResponse { templates }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListTemplatesResponse {
    pub templates: Vec<ProcessTemplate>,
}

/// Get a specific template
pub async fn get_template(
    State(state): State<Arc<AppState>>,
    Path(template_id): Path<String>,
) -> ApiResult<Json<ProcessTemplate>> {
    let template_id = TemplateId::new(template_id);

    let template = state
        .index_store
        .get_template(&template_id)?
        .ok_or_else(|| anyhow::anyhow!("Template not found"))?;

    Ok(Json(template))
}

/// Create or update a template
pub async fn create_template(
    State(state): State<Arc<AppState>>,
    Json(template): Json<ProcessTemplate>,
) -> ApiResult<Json<CreateTemplateResponse>> {
    state.index_store.store_template(&template)?;

    tracing::info!(
        "Created/updated template: {} ({})",
        template.name,
        template.id.0
    );

    Ok(Json(CreateTemplateResponse {
        template_id: template.id.0.clone(),
        message: "Template created/updated successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateTemplateResponse {
    pub template_id: String,
    pub message: String,
}

/// Delete a template
pub async fn delete_template(
    State(state): State<Arc<AppState>>,
    Path(template_id): Path<String>,
) -> ApiResult<Json<DeleteTemplateResponse>> {
    let template_id = TemplateId::new(template_id);

    state.index_store.delete_template(&template_id)?;

    tracing::info!("Deleted template: {}", template_id.0);

    Ok(Json(DeleteTemplateResponse {
        message: "Template deleted successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteTemplateResponse {
    pub message: String,
}

/// Instantiate a template
pub async fn instantiate_template(
    State(state): State<Arc<AppState>>,
    Path(template_id): Path<String>,
    Json(instance): Json<TemplateInstance>,
) -> ApiResult<Json<InstantiateTemplateResponse>> {
    let template_id = TemplateId::new(template_id);

    let template = state
        .index_store
        .get_template(&template_id)?
        .ok_or_else(|| anyhow::anyhow!("Template not found"))?;

    let workflow = TemplateProcessor::instantiate(&template, &instance)?;

    tracing::info!("Instantiated template: {}", template.name);

    Ok(Json(InstantiateTemplateResponse {
        workflow,
        message: "Template instantiated successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct InstantiateTemplateResponse {
    pub workflow: WorkflowSpec,
    pub message: String,
}

// === Claude Config Compiler Endpoint ===

/// Generate Claude configuration for a role
pub async fn compile_claude_config(
    State(state): State<Arc<AppState>>,
    Path(role_id): Path<String>,
) -> ApiResult<Json<CompileClaudeConfigResponse>> {
    let role_id = RoleId::new(role_id);

    // Get organization (assume first one for MVP)
    let orgs = state.index_store.list_organizations()?;
    let org = orgs
        .first()
        .ok_or_else(|| anyhow::anyhow!("No organization configured"))?;

    // Get all roles and policies
    let roles = state.index_store.list_roles()?;
    let policies = state.index_store.list_policies()?;

    // Compile configuration
    let compiler = ClaudeCompiler::new(org.clone(), roles, policies);
    let config = compiler.compile_for_role(&role_id)?;
    let readme = compiler.generate_readme(&role_id)?;

    Ok(Json(CompileClaudeConfigResponse {
        config,
        readme,
        message: "Claude configuration compiled successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CompileClaudeConfigResponse {
    pub config: shiioo_core::types::ClaudeConfig,
    pub readme: String,
    pub message: String,
}

// === Capacity Management Endpoints ===

/// List all capacity sources
pub async fn list_capacity_sources(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListCapacitySourcesResponse>> {
    let sources = state.index_store.list_capacity_sources()?;
    Ok(Json(ListCapacitySourcesResponse { sources }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListCapacitySourcesResponse {
    pub sources: Vec<CapacitySource>,
}

/// Get a specific capacity source
pub async fn get_capacity_source(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
) -> ApiResult<Json<CapacitySource>> {
    let source_id = CapacitySourceId::new(source_id);

    let source = state
        .index_store
        .get_capacity_source(&source_id)?
        .ok_or_else(|| anyhow::anyhow!("Capacity source not found"))?;

    Ok(Json(source))
}

/// Create or update a capacity source
pub async fn create_capacity_source(
    State(state): State<Arc<AppState>>,
    Json(source): Json<CapacitySource>,
) -> ApiResult<Json<CreateCapacitySourceResponse>> {
    state.index_store.store_capacity_source(&source)?;

    tracing::info!(
        "Created/updated capacity source: {} ({})",
        source.name,
        source.id.0
    );

    Ok(Json(CreateCapacitySourceResponse {
        source_id: source.id.0.clone(),
        message: "Capacity source created/updated successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateCapacitySourceResponse {
    pub source_id: String,
    pub message: String,
}

/// Delete a capacity source
pub async fn delete_capacity_source(
    State(state): State<Arc<AppState>>,
    Path(source_id): Path<String>,
) -> ApiResult<Json<DeleteCapacitySourceResponse>> {
    let source_id = CapacitySourceId::new(source_id);

    state.index_store.delete_capacity_source(&source_id)?;

    tracing::info!("Deleted capacity source: {}", source_id.0);

    Ok(Json(DeleteCapacitySourceResponse {
        message: "Capacity source deleted successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteCapacitySourceResponse {
    pub message: String,
}

/// List capacity usage records
pub async fn list_capacity_usage(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListCapacityUsageResponse>> {
    let usage = state.index_store.list_capacity_usage()?;
    Ok(Json(ListCapacityUsageResponse { usage }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListCapacityUsageResponse {
    pub usage: Vec<shiioo_core::types::CapacityUsage>,
}

/// Get capacity cost summary
pub async fn get_capacity_cost(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<CapacityCostResponse>> {
    let usage = state.index_store.list_capacity_usage()?;

    // Calculate total cost and usage by source
    let total_cost: f64 = usage.iter().map(|u| u.cost).sum();
    let total_tokens: u32 = usage.iter().map(|u| u.total_tokens).sum();
    let total_requests: u32 = usage.iter().map(|u| u.request_count).sum();

    Ok(Json(CapacityCostResponse {
        total_cost,
        total_tokens,
        total_requests,
        record_count: usage.len(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CapacityCostResponse {
    pub total_cost: f64,
    pub total_tokens: u32,
    pub total_requests: u32,
    pub record_count: usize,
}

// === Routine Management Endpoints (Phase 5) ===

/// List all routines
pub async fn list_routines(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListRoutinesResponse>> {
    let routines = state.routine_scheduler.list_routines();
    Ok(Json(ListRoutinesResponse { routines }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListRoutinesResponse {
    pub routines: Vec<Routine>,
}

/// Get a specific routine
pub async fn get_routine(
    State(state): State<Arc<AppState>>,
    Path(routine_id): Path<String>,
) -> ApiResult<Json<Routine>> {
    let routine_id = RoutineId::new(routine_id);

    let routine = state
        .routine_scheduler
        .get_routine(&routine_id)
        .ok_or_else(|| anyhow::anyhow!("Routine not found"))?;

    Ok(Json(routine))
}

/// Create a routine
pub async fn create_routine(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CreateRoutineRequest>,
) -> ApiResult<Json<CreateRoutineResponse>> {
    let routine = Routine {
        id: RoutineId::new(uuid::Uuid::new_v4().to_string()),
        name: req.name,
        description: req.description,
        schedule: req.schedule,
        workflow: req.workflow,
        enabled: req.enabled.unwrap_or(false),
        last_run: None,
        next_run: chrono::Utc::now(),
        created_at: chrono::Utc::now(),
        created_by: req.created_by.unwrap_or_else(|| "system".to_string()),
        updated_at: chrono::Utc::now(),
    };

    state.routine_scheduler.register_routine(routine.clone())?;

    tracing::info!("Created routine: {} ({})", routine.name, routine.id.0);

    Ok(Json(CreateRoutineResponse {
        routine_id: routine.id.0.clone(),
        message: "Routine created successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateRoutineRequest {
    pub name: String,
    pub description: String,
    pub schedule: RoutineSchedule,
    pub workflow: WorkflowSpec,
    pub enabled: Option<bool>,
    pub created_by: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateRoutineResponse {
    pub routine_id: String,
    pub message: String,
}

/// Delete a routine
pub async fn delete_routine(
    State(state): State<Arc<AppState>>,
    Path(routine_id): Path<String>,
) -> ApiResult<Json<DeleteRoutineResponse>> {
    let routine_id = RoutineId::new(routine_id);

    state.routine_scheduler.unregister_routine(&routine_id)?;

    tracing::info!("Deleted routine: {}", routine_id.0);

    Ok(Json(DeleteRoutineResponse {
        message: "Routine deleted successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteRoutineResponse {
    pub message: String,
}

/// Enable a routine
pub async fn enable_routine(
    State(state): State<Arc<AppState>>,
    Path(routine_id): Path<String>,
) -> ApiResult<Json<EnableRoutineResponse>> {
    let routine_id = RoutineId::new(routine_id);

    state.routine_scheduler.enable_routine(&routine_id)?;

    tracing::info!("Enabled routine: {}", routine_id.0);

    Ok(Json(EnableRoutineResponse {
        message: "Routine enabled successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct EnableRoutineResponse {
    pub message: String,
}

/// Disable a routine
pub async fn disable_routine(
    State(state): State<Arc<AppState>>,
    Path(routine_id): Path<String>,
) -> ApiResult<Json<DisableRoutineResponse>> {
    let routine_id = RoutineId::new(routine_id);

    state.routine_scheduler.disable_routine(&routine_id)?;

    tracing::info!("Disabled routine: {}", routine_id.0);

    Ok(Json(DisableRoutineResponse {
        message: "Routine disabled successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisableRoutineResponse {
    pub message: String,
}

/// Get execution history for a routine
pub async fn get_routine_executions(
    State(state): State<Arc<AppState>>,
    Path(routine_id): Path<String>,
) -> ApiResult<Json<GetRoutineExecutionsResponse>> {
    let routine_id = RoutineId::new(routine_id);

    let executions = state.routine_scheduler.get_executions(&routine_id);

    Ok(Json(GetRoutineExecutionsResponse { executions }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GetRoutineExecutionsResponse {
    pub executions: Vec<shiioo_core::types::RoutineExecution>,
}

// === Approval Board Management Endpoints (Phase 5) ===

/// List all approval boards
pub async fn list_approval_boards(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListApprovalBoardsResponse>> {
    let boards = state.approval_manager.list_boards();
    Ok(Json(ListApprovalBoardsResponse { boards }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListApprovalBoardsResponse {
    pub boards: Vec<ApprovalBoard>,
}

/// Get a specific approval board
pub async fn get_approval_board(
    State(state): State<Arc<AppState>>,
    Path(board_id): Path<String>,
) -> ApiResult<Json<ApprovalBoard>> {
    let board_id = ApprovalBoardId::new(board_id);

    let board = state
        .approval_manager
        .get_board(&board_id)
        .ok_or_else(|| anyhow::anyhow!("Approval board not found"))?;

    Ok(Json(board))
}

/// Create an approval board
pub async fn create_approval_board(
    State(state): State<Arc<AppState>>,
    Json(board): Json<ApprovalBoard>,
) -> ApiResult<Json<CreateApprovalBoardResponse>> {
    state.approval_manager.register_board(board.clone())?;

    tracing::info!("Created approval board: {} ({})", board.name, board.id.0);

    Ok(Json(CreateApprovalBoardResponse {
        board_id: board.id.0.clone(),
        message: "Approval board created successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateApprovalBoardResponse {
    pub board_id: String,
    pub message: String,
}

/// Delete an approval board
pub async fn delete_approval_board(
    State(state): State<Arc<AppState>>,
    Path(board_id): Path<String>,
) -> ApiResult<Json<DeleteApprovalBoardResponse>> {
    let board_id = ApprovalBoardId::new(board_id);

    state.approval_manager.delete_board(&board_id)?;

    tracing::info!("Deleted approval board: {}", board_id.0);

    Ok(Json(DeleteApprovalBoardResponse {
        message: "Approval board deleted successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteApprovalBoardResponse {
    pub message: String,
}

// === Approval Management Endpoints (Phase 5) ===

/// List all approvals
pub async fn list_approvals(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListApprovalsResponse>> {
    let approvals = state.approval_manager.list_approvals();
    Ok(Json(ListApprovalsResponse { approvals }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListApprovalsResponse {
    pub approvals: Vec<shiioo_core::types::Approval>,
}

/// Get a specific approval
pub async fn get_approval(
    State(state): State<Arc<AppState>>,
    Path(approval_id): Path<String>,
) -> ApiResult<Json<shiioo_core::types::Approval>> {
    let approval_id = ApprovalId::new(approval_id);

    let approval = state
        .approval_manager
        .get_approval(&approval_id)
        .ok_or_else(|| anyhow::anyhow!("Approval not found"))?;

    Ok(Json(approval))
}

/// Cast a vote on an approval
pub async fn cast_vote(
    State(state): State<Arc<AppState>>,
    Path(approval_id): Path<String>,
    Json(req): Json<CastVoteRequest>,
) -> ApiResult<Json<CastVoteResponse>> {
    let approval_id = ApprovalId::new(approval_id);
    let voter_id = req.voter_id.clone();

    state
        .approval_manager
        .cast_vote(&approval_id, req.voter_id, req.decision, req.comment)?;

    tracing::info!(
        "Vote cast on approval {}: {:?} by {}",
        approval_id.0,
        req.decision,
        voter_id.0
    );

    Ok(Json(CastVoteResponse {
        message: "Vote cast successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CastVoteRequest {
    pub voter_id: PersonId,
    pub decision: VoteDecision,
    pub comment: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CastVoteResponse {
    pub message: String,
}

// === Config Change Management Endpoints (Phase 5) ===

/// List all config changes
pub async fn list_config_changes(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListConfigChangesResponse>> {
    let changes = state.config_change_manager.list_changes();
    Ok(Json(ListConfigChangesResponse { changes }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListConfigChangesResponse {
    pub changes: Vec<ConfigChange>,
}

/// Get a specific config change
pub async fn get_config_change(
    State(state): State<Arc<AppState>>,
    Path(change_id): Path<String>,
) -> ApiResult<Json<ConfigChange>> {
    let change_id = ConfigChangeId::new(change_id);

    let change = state
        .config_change_manager
        .get_change(&change_id)
        .ok_or_else(|| anyhow::anyhow!("Config change not found"))?;

    Ok(Json(change))
}

/// Propose a config change
pub async fn propose_config_change(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ProposeConfigChangeRequest>,
) -> ApiResult<Json<ProposeConfigChangeResponse>> {
    let change = state.config_change_manager.propose_change(
        req.change_type,
        req.description,
        req.before,
        req.after,
        req.proposed_by,
        req.approval_board,
    )?;

    tracing::info!(
        "Proposed config change: {} ({})",
        change.description,
        change.id.0
    );

    Ok(Json(ProposeConfigChangeResponse {
        change_id: change.id.0.clone(),
        approval_id: change.approval_id.as_ref().map(|id| id.0.clone()),
        message: "Config change proposed successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProposeConfigChangeRequest {
    pub change_type: ConfigChangeType,
    pub description: String,
    pub before: Option<String>,
    pub after: String,
    pub proposed_by: String,
    pub approval_board: Option<ApprovalBoardId>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProposeConfigChangeResponse {
    pub change_id: String,
    pub approval_id: Option<String>,
    pub message: String,
}

/// Apply a config change
pub async fn apply_config_change(
    State(state): State<Arc<AppState>>,
    Path(change_id): Path<String>,
) -> ApiResult<Json<ApplyConfigChangeResponse>> {
    let change_id = ConfigChangeId::new(change_id);

    state.config_change_manager.apply_change(&change_id)?;

    tracing::info!("Applied config change: {}", change_id.0);

    Ok(Json(ApplyConfigChangeResponse {
        message: "Config change applied successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ApplyConfigChangeResponse {
    pub message: String,
}

/// Reject a config change
pub async fn reject_config_change(
    State(state): State<Arc<AppState>>,
    Path(change_id): Path<String>,
    Json(req): Json<RejectConfigChangeRequest>,
) -> ApiResult<Json<RejectConfigChangeResponse>> {
    let change_id = ConfigChangeId::new(change_id);

    state
        .config_change_manager
        .reject_change(&change_id, req.reason)?;

    tracing::info!("Rejected config change: {}", change_id.0);

    Ok(Json(RejectConfigChangeResponse {
        message: "Config change rejected successfully".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RejectConfigChangeRequest {
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RejectConfigChangeResponse {
    pub message: String,
}

// === Observability Endpoints (Phase 6) ===

/// Get all metrics
pub async fn get_metrics(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<MetricsResponse>> {
    let counters = state.metrics.get_counters();
    let gauges = state.metrics.get_gauges();
    let histograms = state.metrics.get_histograms();

    Ok(Json(MetricsResponse {
        counters,
        gauges,
        histograms,
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MetricsResponse {
    pub counters: Vec<shiioo_core::metrics::Counter>,
    pub gauges: Vec<shiioo_core::metrics::Gauge>,
    pub histograms: Vec<shiioo_core::metrics::Histogram>,
}

/// Get workflow analytics
pub async fn get_workflow_analytics(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<WorkflowAnalyticsResponse>> {
    let stats = state.analytics.get_all_workflow_stats();
    Ok(Json(WorkflowAnalyticsResponse { workflows: stats }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorkflowAnalyticsResponse {
    pub workflows: Vec<shiioo_core::analytics::WorkflowStats>,
}

/// Get specific workflow analytics
pub async fn get_workflow_analytics_by_id(
    State(state): State<Arc<AppState>>,
    Path(workflow_id): Path<String>,
) -> ApiResult<Json<shiioo_core::analytics::WorkflowStats>> {
    let stats = state
        .analytics
        .get_workflow_stats(&workflow_id)
        .ok_or_else(|| anyhow::anyhow!("Workflow stats not found"))?;

    Ok(Json(stats))
}

/// Get step analytics
pub async fn get_step_analytics(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<StepAnalyticsResponse>> {
    let stats = state.analytics.get_all_step_stats();
    Ok(Json(StepAnalyticsResponse { steps: stats }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StepAnalyticsResponse {
    pub steps: Vec<shiioo_core::analytics::StepStats>,
}

/// Get execution traces
pub async fn get_execution_traces(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ExecutionTracesResponse>> {
    let traces = state.analytics.get_recent_traces(50);
    Ok(Json(ExecutionTracesResponse { traces }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ExecutionTracesResponse {
    pub traces: Vec<shiioo_core::analytics::ExecutionTrace>,
}

/// Get specific execution trace
pub async fn get_execution_trace(
    State(state): State<Arc<AppState>>,
    Path(run_id): Path<String>,
) -> ApiResult<Json<shiioo_core::analytics::ExecutionTrace>> {
    let run_id = RunId(
        run_id
            .parse()
            .map_err(|_| anyhow::anyhow!("Invalid run ID"))?,
    );

    let trace = state
        .analytics
        .get_trace(&run_id)
        .ok_or_else(|| anyhow::anyhow!("Execution trace not found"))?;

    Ok(Json(trace))
}

/// Get bottleneck analysis for a workflow
pub async fn get_bottleneck_analysis(
    State(state): State<Arc<AppState>>,
    Path(workflow_id): Path<String>,
) -> ApiResult<Json<shiioo_core::analytics::BottleneckReport>> {
    let report = state
        .analytics
        .detect_bottlenecks(&workflow_id)
        .ok_or_else(|| anyhow::anyhow!("Bottleneck analysis not available for this workflow"))?;

    Ok(Json(report))
}

/// Get system health status
pub async fn get_health_status(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<HealthStatusResponse>> {
    let routines = state.routine_scheduler.list_routines();
    let approvals = state.approval_manager.list_approvals();
    let workflow_stats = state.analytics.get_all_workflow_stats();

    let active_routines = routines.iter().filter(|r| r.enabled).count();
    let pending_approvals = approvals
        .iter()
        .filter(|a| matches!(a.status, shiioo_core::types::ApprovalStatus::Pending))
        .count();

    let total_workflow_executions: u64 = workflow_stats.iter().map(|w| w.execution_count).sum();
    let successful_executions: u64 = workflow_stats.iter().map(|w| w.success_count).sum();
    let failed_executions: u64 = workflow_stats.iter().map(|w| w.failure_count).sum();

    let success_rate = if total_workflow_executions > 0 {
        (successful_executions as f64 / total_workflow_executions as f64) * 100.0
    } else {
        100.0
    };

    Ok(Json(HealthStatusResponse {
        status: "healthy".to_string(),
        uptime_secs: 0, // TODO: Track actual uptime
        active_routines,
        total_routines: routines.len(),
        pending_approvals,
        total_workflow_executions,
        successful_executions,
        failed_executions,
        success_rate,
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HealthStatusResponse {
    pub status: String,
    pub uptime_secs: u64,
    pub active_routines: usize,
    pub total_routines: usize,
    pub pending_approvals: usize,
    pub total_workflow_executions: u64,
    pub successful_executions: u64,
    pub failed_executions: u64,
    pub success_rate: f64,
}

// ============================================================================
// Phase 7: Multi-tenancy & High Availability
// ============================================================================

use shiioo_core::{
    tenant::{Tenant, TenantId, TenantQuota, TenantSettings, TenantStatus, QuotaResource},
    cluster::{ClusterNode, NodeId, NodeStatus, NodeRole},
};

/// Register a new tenant
pub async fn register_tenant(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterTenantRequest>,
) -> ApiResult<Json<Tenant>> {
    let tenant = Tenant {
        id: TenantId::generate(),
        name: req.name.clone(),
        description: req.description.clone(),
        status: TenantStatus::Active,
        quota: req.quota.unwrap_or_default(),
        settings: req.settings.unwrap_or_default(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };

    state.tenant_manager.register_tenant(tenant.clone())?;

    // Initialize tenant storage
    state.tenant_storage.initialize_tenant(&tenant.id)?;

    tracing::info!("Registered tenant: {} ({})", tenant.name, tenant.id.0);

    Ok(Json(tenant))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterTenantRequest {
    pub name: String,
    pub description: String,
    pub quota: Option<TenantQuota>,
    pub settings: Option<TenantSettings>,
}

/// List all tenants
pub async fn list_tenants(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListTenantsResponse>> {
    let tenants = state.tenant_manager.list_tenants();
    Ok(Json(ListTenantsResponse { tenants }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListTenantsResponse {
    pub tenants: Vec<Tenant>,
}

/// Get a specific tenant
pub async fn get_tenant(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
) -> ApiResult<Json<Tenant>> {
    let tenant_id = TenantId::new(tenant_id);

    let tenant = state
        .tenant_manager
        .get_tenant(&tenant_id)
        .ok_or_else(|| anyhow::anyhow!("Tenant not found"))?;

    Ok(Json(tenant))
}

/// Update tenant
pub async fn update_tenant(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
    Json(req): Json<UpdateTenantRequest>,
) -> ApiResult<Json<Tenant>> {
    let tenant_id = TenantId::new(tenant_id);

    let mut tenant = state
        .tenant_manager
        .get_tenant(&tenant_id)
        .ok_or_else(|| anyhow::anyhow!("Tenant not found"))?;

    if let Some(name) = req.name {
        tenant.name = name;
    }
    if let Some(description) = req.description {
        tenant.description = description;
    }
    if let Some(quota) = req.quota {
        tenant.quota = quota;
    }
    if let Some(settings) = req.settings {
        tenant.settings = settings;
    }

    tenant.updated_at = chrono::Utc::now();

    state.tenant_manager.update_tenant(tenant.clone())?;

    tracing::info!("Updated tenant: {}", tenant_id.0);

    Ok(Json(tenant))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateTenantRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub quota: Option<TenantQuota>,
    pub settings: Option<TenantSettings>,
}

/// Delete tenant
pub async fn delete_tenant(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
) -> ApiResult<Json<DeleteTenantResponse>> {
    let tenant_id = TenantId::new(tenant_id);

    // Delete tenant data
    state.tenant_storage.delete_tenant_data(&tenant_id)?;

    // Remove tenant from manager
    state.tenant_manager.delete_tenant(&tenant_id)?;

    tracing::info!("Deleted tenant: {}", tenant_id.0);

    Ok(Json(DeleteTenantResponse {
        message: format!("Tenant {} deleted successfully", tenant_id.0),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DeleteTenantResponse {
    pub message: String,
}

/// Suspend tenant
pub async fn suspend_tenant(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
) -> ApiResult<Json<Tenant>> {
    let tenant_id = TenantId::new(tenant_id);

    state.tenant_manager.suspend_tenant(&tenant_id)?;

    let tenant = state
        .tenant_manager
        .get_tenant(&tenant_id)
        .ok_or_else(|| anyhow::anyhow!("Tenant not found"))?;

    tracing::info!("Suspended tenant: {}", tenant_id.0);

    Ok(Json(tenant))
}

/// Activate tenant
pub async fn activate_tenant(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
) -> ApiResult<Json<Tenant>> {
    let tenant_id = TenantId::new(tenant_id);

    state.tenant_manager.activate_tenant(&tenant_id)?;

    let tenant = state
        .tenant_manager
        .get_tenant(&tenant_id)
        .ok_or_else(|| anyhow::anyhow!("Tenant not found"))?;

    tracing::info!("Activated tenant: {}", tenant_id.0);

    Ok(Json(tenant))
}

/// Get tenant storage statistics
pub async fn get_tenant_storage_stats(
    State(state): State<Arc<AppState>>,
    Path(tenant_id): Path<String>,
) -> ApiResult<Json<shiioo_core::storage::TenantStorageStats>> {
    let tenant_id = TenantId::new(tenant_id);

    let stats = state.tenant_storage.tenant_stats(&tenant_id)?;

    Ok(Json(stats))
}

/// Register cluster node
pub async fn register_cluster_node(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RegisterNodeRequest>,
) -> ApiResult<Json<ClusterNode>> {
    let node = ClusterNode {
        id: NodeId::generate(),
        address: req.address.clone(),
        region: req.region.clone(),
        status: NodeStatus::Healthy,
        role: NodeRole::Follower,
        last_heartbeat: chrono::Utc::now(),
        started_at: chrono::Utc::now(),
        metadata: req.metadata.unwrap_or_default(),
    };

    state.cluster_manager.register_node(node.clone())?;

    tracing::info!("Registered cluster node: {} at {}", node.id.0, node.address);

    Ok(Json(node))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterNodeRequest {
    pub address: String,
    pub region: Option<String>,
    pub metadata: Option<std::collections::HashMap<String, String>>,
}

/// List cluster nodes
pub async fn list_cluster_nodes(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ListNodesResponse>> {
    let nodes = state.cluster_manager.list_nodes();
    Ok(Json(ListNodesResponse { nodes }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ListNodesResponse {
    pub nodes: Vec<ClusterNode>,
}

/// Get cluster node
pub async fn get_cluster_node(
    State(state): State<Arc<AppState>>,
    Path(node_id): Path<String>,
) -> ApiResult<Json<ClusterNode>> {
    let node_id = NodeId::new(node_id);

    let node = state
        .cluster_manager
        .get_node(&node_id)
        .ok_or_else(|| anyhow::anyhow!("Node not found"))?;

    Ok(Json(node))
}

/// Send heartbeat for a node
pub async fn node_heartbeat(
    State(state): State<Arc<AppState>>,
    Path(node_id): Path<String>,
) -> ApiResult<Json<HeartbeatResponse>> {
    let node_id = NodeId::new(node_id);

    state.cluster_manager.heartbeat(&node_id)?;

    tracing::debug!("Heartbeat received from node: {}", node_id.0);

    Ok(Json(HeartbeatResponse {
        message: "Heartbeat acknowledged".to_string(),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct HeartbeatResponse {
    pub message: String,
}

/// Remove cluster node
pub async fn remove_cluster_node(
    State(state): State<Arc<AppState>>,
    Path(node_id): Path<String>,
) -> ApiResult<Json<RemoveNodeResponse>> {
    let node_id = NodeId::new(node_id);

    state.cluster_manager.remove_node(&node_id)?;

    tracing::info!("Removed cluster node: {}", node_id.0);

    Ok(Json(RemoveNodeResponse {
        message: format!("Node {} removed successfully", node_id.0),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RemoveNodeResponse {
    pub message: String,
}

/// Get current leader node
pub async fn get_cluster_leader(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<LeaderResponse>> {
    let leader = state.cluster_manager.get_leader();

    Ok(Json(LeaderResponse { leader }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LeaderResponse {
    pub leader: Option<ClusterNode>,
}

/// Get cluster health
pub async fn get_cluster_health(
    State(state): State<Arc<AppState>>,
) -> ApiResult<Json<ClusterHealthResponse>> {
    let total_nodes = state.cluster_manager.cluster_size();
    let healthy_nodes = state.cluster_manager.healthy_node_count();
    let leader = state.cluster_manager.get_leader();

    Ok(Json(ClusterHealthResponse {
        total_nodes,
        healthy_nodes,
        has_leader: leader.is_some(),
        leader_id: leader.map(|l| l.id.0),
    }))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ClusterHealthResponse {
    pub total_nodes: usize,
    pub healthy_nodes: usize,
    pub has_leader: bool,
    pub leader_id: Option<String>,
}
