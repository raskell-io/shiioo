//! Jobs API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::types::{RunId, WorkflowSpec};

/// Jobs API for creating and managing jobs.
pub struct JobsApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> JobsApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// Create a new job.
    pub async fn create(&self, request: CreateJobRequest) -> ShiiooResult<CreateJobResponse> {
        self.client.http.post("/api/jobs", &request).await
    }
}

/// Request to create a new job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateJobRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub workflow: WorkflowSpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub execute: Option<bool>,
}

/// Response from creating a job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateJobResponse {
    pub job_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<RunId>,
    pub message: String,
}
