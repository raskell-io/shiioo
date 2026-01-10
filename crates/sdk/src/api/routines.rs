//! Routines API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::types::{Routine, RoutineExecution, RoutineId, RoutineSchedule, WorkflowSpec};

/// Routines API for managing scheduled workflows.
pub struct RoutinesApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> RoutinesApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all routines.
    pub async fn list(&self) -> ShiiooResult<Vec<Routine>> {
        let response: ListRoutinesResponse = self.client.http.get("/api/routines").await?;
        Ok(response.routines)
    }

    /// Get a specific routine.
    pub async fn get(&self, routine_id: &RoutineId) -> ShiiooResult<Routine> {
        self.client
            .http
            .get(&format!("/api/routines/{}", routine_id.0))
            .await
    }

    /// Create a routine.
    pub async fn create(&self, request: CreateRoutineRequest) -> ShiiooResult<CreateRoutineResponse> {
        self.client.http.post("/api/routines", &request).await
    }

    /// Delete a routine.
    pub async fn delete(&self, routine_id: &RoutineId) -> ShiiooResult<DeleteRoutineResponse> {
        self.client
            .http
            .delete(&format!("/api/routines/{}", routine_id.0))
            .await
    }

    /// Enable a routine.
    pub async fn enable(&self, routine_id: &RoutineId) -> ShiiooResult<EnableRoutineResponse> {
        self.client
            .http
            .post(&format!("/api/routines/{}/enable", routine_id.0), &())
            .await
    }

    /// Disable a routine.
    pub async fn disable(&self, routine_id: &RoutineId) -> ShiiooResult<DisableRoutineResponse> {
        self.client
            .http
            .post(&format!("/api/routines/{}/disable", routine_id.0), &())
            .await
    }

    /// Get execution history for a routine.
    pub async fn executions(&self, routine_id: &RoutineId) -> ShiiooResult<Vec<RoutineExecution>> {
        let response: GetRoutineExecutionsResponse = self
            .client
            .http
            .get(&format!("/api/routines/{}/executions", routine_id.0))
            .await?;
        Ok(response.executions)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListRoutinesResponse {
    routines: Vec<Routine>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetRoutineExecutionsResponse {
    executions: Vec<RoutineExecution>,
}

/// Request to create a routine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoutineRequest {
    pub name: String,
    pub description: String,
    pub schedule: RoutineSchedule,
    pub workflow: WorkflowSpec,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_by: Option<String>,
}

/// Response from creating a routine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoutineResponse {
    pub routine_id: String,
    pub message: String,
}

/// Response from deleting a routine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteRoutineResponse {
    pub message: String,
}

/// Response from enabling a routine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnableRoutineResponse {
    pub message: String,
}

/// Response from disabling a routine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisableRoutineResponse {
    pub message: String,
}
