//! Roles API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::types::{RoleId, RoleSpec};

/// Roles API for managing roles.
pub struct RolesApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> RolesApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all roles.
    pub async fn list(&self) -> ShiiooResult<Vec<RoleSpec>> {
        let response: ListRolesResponse = self.client.http.get("/api/roles").await?;
        Ok(response.roles)
    }

    /// Get a specific role by ID.
    pub async fn get(&self, role_id: &RoleId) -> ShiiooResult<RoleSpec> {
        self.client.http.get(&format!("/api/roles/{}", role_id.0)).await
    }

    /// Create or update a role.
    pub async fn create(&self, role: &RoleSpec) -> ShiiooResult<CreateRoleResponse> {
        self.client.http.post("/api/roles", role).await
    }

    /// Delete a role.
    pub async fn delete(&self, role_id: &RoleId) -> ShiiooResult<DeleteRoleResponse> {
        self.client.http.delete(&format!("/api/roles/{}", role_id.0)).await
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListRolesResponse {
    roles: Vec<RoleSpec>,
}

/// Response from creating a role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoleResponse {
    pub role_id: String,
    pub message: String,
}

/// Response from deleting a role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteRoleResponse {
    pub message: String,
}
