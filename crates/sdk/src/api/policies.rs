//! Policies API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::types::{PolicyId, PolicySpec};

/// Policies API for managing policies.
pub struct PoliciesApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> PoliciesApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all policies.
    pub async fn list(&self) -> ShiiooResult<Vec<PolicySpec>> {
        let response: ListPoliciesResponse = self.client.http.get("/api/policies").await?;
        Ok(response.policies)
    }

    /// Get a specific policy by ID.
    pub async fn get(&self, policy_id: &PolicyId) -> ShiiooResult<PolicySpec> {
        self.client
            .http
            .get(&format!("/api/policies/{}", policy_id.0))
            .await
    }

    /// Create or update a policy.
    pub async fn create(&self, policy: &PolicySpec) -> ShiiooResult<CreatePolicyResponse> {
        self.client.http.post("/api/policies", policy).await
    }

    /// Delete a policy.
    pub async fn delete(&self, policy_id: &PolicyId) -> ShiiooResult<DeletePolicyResponse> {
        self.client
            .http
            .delete(&format!("/api/policies/{}", policy_id.0))
            .await
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListPoliciesResponse {
    policies: Vec<PolicySpec>,
}

/// Response from creating a policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreatePolicyResponse {
    pub policy_id: String,
    pub message: String,
}

/// Response from deleting a policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeletePolicyResponse {
    pub message: String,
}
