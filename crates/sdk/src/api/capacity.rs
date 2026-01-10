//! Capacity API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::types::{CapacitySource, CapacitySourceId, CapacityUsage};

/// Capacity API for managing LLM capacity sources.
pub struct CapacityApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> CapacityApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all capacity sources.
    pub async fn sources(&self) -> ShiiooResult<Vec<CapacitySource>> {
        let response: ListCapacitySourcesResponse =
            self.client.http.get("/api/capacity/sources").await?;
        Ok(response.sources)
    }

    /// Get a specific capacity source.
    pub async fn get_source(&self, source_id: &CapacitySourceId) -> ShiiooResult<CapacitySource> {
        self.client
            .http
            .get(&format!("/api/capacity/sources/{}", source_id.0))
            .await
    }

    /// Create or update a capacity source.
    pub async fn create_source(
        &self,
        source: &CapacitySource,
    ) -> ShiiooResult<CreateCapacitySourceResponse> {
        self.client.http.post("/api/capacity/sources", source).await
    }

    /// Delete a capacity source.
    pub async fn delete_source(
        &self,
        source_id: &CapacitySourceId,
    ) -> ShiiooResult<DeleteCapacitySourceResponse> {
        self.client
            .http
            .delete(&format!("/api/capacity/sources/{}", source_id.0))
            .await
    }

    /// List capacity usage records.
    pub async fn usage(&self) -> ShiiooResult<Vec<CapacityUsage>> {
        let response: ListCapacityUsageResponse =
            self.client.http.get("/api/capacity/usage").await?;
        Ok(response.usage)
    }

    /// Get capacity cost summary.
    pub async fn cost(&self) -> ShiiooResult<CapacityCostResponse> {
        self.client.http.get("/api/capacity/cost").await
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListCapacitySourcesResponse {
    sources: Vec<CapacitySource>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ListCapacityUsageResponse {
    usage: Vec<CapacityUsage>,
}

/// Response from creating a capacity source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateCapacitySourceResponse {
    pub source_id: String,
    pub message: String,
}

/// Response from deleting a capacity source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteCapacitySourceResponse {
    pub message: String,
}

/// Response with capacity cost summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityCostResponse {
    pub total_cost: f64,
    pub total_tokens: u32,
    pub total_requests: u32,
    pub record_count: usize,
}
