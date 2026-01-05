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
    storage::IndexStore,
    template::TemplateProcessor,
    types::{
        Job, OrgId, Organization, PolicyId, PolicySpec, ProcessTemplate, RoleId, RoleSpec, Run,
        RunId, TemplateId, TemplateInstance, WorkflowSpec,
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
