//! Runs API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::events::Event;
use shiioo_core::types::{Run, RunId};

/// Runs API for managing workflow runs.
pub struct RunsApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> RunsApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all runs.
    pub async fn list(&self) -> ShiiooResult<Vec<Run>> {
        let response: ListRunsResponse = self.client.http.get("/api/runs").await?;
        Ok(response.runs)
    }

    /// Get a specific run by ID.
    pub async fn get(&self, run_id: &RunId) -> ShiiooResult<Run> {
        self.client.http.get(&format!("/api/runs/{}", run_id.0)).await
    }

    /// Get events for a run.
    pub async fn events(&self, run_id: &RunId) -> ShiiooResult<Vec<Event>> {
        let response: GetRunEventsResponse = self
            .client
            .http
            .get(&format!("/api/runs/{}/events", run_id.0))
            .await?;
        Ok(response.events)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListRunsResponse {
    runs: Vec<Run>,
}

#[derive(Debug, Serialize, Deserialize)]
struct GetRunEventsResponse {
    events: Vec<Event>,
}
