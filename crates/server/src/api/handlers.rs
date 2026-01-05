use super::{ApiResult, ErrorResponse};
use crate::config::AppState;
use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use shiioo_core::{
    events::EventLog,
    storage::IndexStore,
    types::{Job, Run, RunId, WorkflowSpec},
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
