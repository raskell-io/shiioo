//! Config Changes API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::types::{ApprovalBoardId, ConfigChange, ConfigChangeId, ConfigChangeType};

/// Config Changes API for managing configuration changes.
pub struct ConfigChangesApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> ConfigChangesApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all config changes.
    pub async fn list(&self) -> ShiiooResult<Vec<ConfigChange>> {
        let response: ListConfigChangesResponse =
            self.client.http.get("/api/config-changes").await?;
        Ok(response.changes)
    }

    /// Get a specific config change.
    pub async fn get(&self, change_id: &ConfigChangeId) -> ShiiooResult<ConfigChange> {
        self.client
            .http
            .get(&format!("/api/config-changes/{}", change_id.0))
            .await
    }

    /// Propose a config change.
    pub async fn propose(
        &self,
        request: ProposeConfigChangeRequest,
    ) -> ShiiooResult<ProposeConfigChangeResponse> {
        self.client.http.post("/api/config-changes", &request).await
    }

    /// Apply a config change.
    pub async fn apply(&self, change_id: &ConfigChangeId) -> ShiiooResult<ApplyConfigChangeResponse> {
        self.client
            .http
            .post(&format!("/api/config-changes/{}/apply", change_id.0), &())
            .await
    }

    /// Reject a config change.
    pub async fn reject(
        &self,
        change_id: &ConfigChangeId,
        request: RejectConfigChangeRequest,
    ) -> ShiiooResult<RejectConfigChangeResponse> {
        self.client
            .http
            .post(&format!("/api/config-changes/{}/reject", change_id.0), &request)
            .await
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListConfigChangesResponse {
    changes: Vec<ConfigChange>,
}

/// Request to propose a config change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposeConfigChangeRequest {
    pub change_type: ConfigChangeType,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    pub after: String,
    pub proposed_by: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_board: Option<ApprovalBoardId>,
}

/// Response from proposing a config change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposeConfigChangeResponse {
    pub change_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    pub message: String,
}

/// Response from applying a config change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplyConfigChangeResponse {
    pub message: String,
}

/// Request to reject a config change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectConfigChangeRequest {
    pub reason: String,
}

/// Response from rejecting a config change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RejectConfigChangeResponse {
    pub message: String,
}
