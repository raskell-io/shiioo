//! Organizations API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::types::{OrgId, Organization};

/// Organizations API for managing organizations.
pub struct OrganizationsApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> OrganizationsApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all organizations.
    pub async fn list(&self) -> ShiiooResult<Vec<Organization>> {
        let response: ListOrganizationsResponse =
            self.client.http.get("/api/organizations").await?;
        Ok(response.organizations)
    }

    /// Get a specific organization by ID.
    pub async fn get(&self, org_id: &OrgId) -> ShiiooResult<Organization> {
        self.client
            .http
            .get(&format!("/api/organizations/{}", org_id.0))
            .await
    }

    /// Create or update an organization.
    pub async fn create(&self, org: &Organization) -> ShiiooResult<CreateOrganizationResponse> {
        self.client.http.post("/api/organizations", org).await
    }

    /// Delete an organization.
    pub async fn delete(&self, org_id: &OrgId) -> ShiiooResult<DeleteOrganizationResponse> {
        self.client
            .http
            .delete(&format!("/api/organizations/{}", org_id.0))
            .await
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListOrganizationsResponse {
    organizations: Vec<Organization>,
}

/// Response from creating an organization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateOrganizationResponse {
    pub org_id: String,
    pub message: String,
}

/// Response from deleting an organization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteOrganizationResponse {
    pub message: String,
}
