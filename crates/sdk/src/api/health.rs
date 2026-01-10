//! Health API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};

/// Health API for checking server status.
pub struct HealthApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> HealthApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// Check basic health status.
    pub async fn check(&self) -> ShiiooResult<HealthCheck> {
        self.client.http.get("/api/health").await
    }

    /// Get comprehensive health status.
    pub async fn status(&self) -> ShiiooResult<HealthStatusResponse> {
        self.client.http.get("/api/health/status").await
    }
}

/// Basic health check response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub status: String,
}

/// Comprehensive health status response.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
