//! Analytics API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::analytics::{BottleneckReport, ExecutionTrace, StepStats, WorkflowStats};
use shiioo_core::types::RunId;

/// Analytics API for workflow analytics and traces.
pub struct AnalyticsApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> AnalyticsApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// Get workflow analytics for all workflows.
    pub async fn workflows(&self) -> ShiiooResult<Vec<WorkflowStats>> {
        let response: WorkflowAnalyticsResponse =
            self.client.http.get("/api/analytics/workflows").await?;
        Ok(response.workflows)
    }

    /// Get analytics for a specific workflow.
    pub async fn workflow(&self, workflow_id: &str) -> ShiiooResult<WorkflowStats> {
        self.client
            .http
            .get(&format!("/api/analytics/workflows/{}", workflow_id))
            .await
    }

    /// Get step analytics for all steps.
    pub async fn steps(&self) -> ShiiooResult<Vec<StepStats>> {
        let response: StepAnalyticsResponse =
            self.client.http.get("/api/analytics/steps").await?;
        Ok(response.steps)
    }

    /// Get recent execution traces.
    pub async fn traces(&self) -> ShiiooResult<Vec<ExecutionTrace>> {
        let response: ExecutionTracesResponse =
            self.client.http.get("/api/analytics/traces").await?;
        Ok(response.traces)
    }

    /// Get execution trace for a specific run.
    pub async fn trace(&self, run_id: &RunId) -> ShiiooResult<ExecutionTrace> {
        self.client
            .http
            .get(&format!("/api/analytics/traces/{}", run_id.0))
            .await
    }

    /// Get bottleneck analysis for a workflow.
    pub async fn bottlenecks(&self, workflow_id: &str) -> ShiiooResult<BottleneckReport> {
        self.client
            .http
            .get(&format!("/api/analytics/bottlenecks/{}", workflow_id))
            .await
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct WorkflowAnalyticsResponse {
    workflows: Vec<WorkflowStats>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StepAnalyticsResponse {
    steps: Vec<StepStats>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExecutionTracesResponse {
    traces: Vec<ExecutionTrace>,
}
