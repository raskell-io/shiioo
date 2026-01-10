//! RBAC API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::rbac::{Action, RbacRole, Resource};

/// RBAC API for role-based access control.
pub struct RbacApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> RbacApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all RBAC roles.
    pub async fn roles(&self) -> ShiiooResult<Vec<RbacRole>> {
        self.client.http.get("/api/rbac/roles").await
    }

    /// Get a specific RBAC role.
    pub async fn get_role(&self, role_id: &str) -> ShiiooResult<RbacRole> {
        self.client
            .http
            .get(&format!("/api/rbac/roles/{}", role_id))
            .await
    }

    /// Create an RBAC role.
    pub async fn create_role(&self, request: CreateRbacRoleRequest) -> ShiiooResult<RbacRole> {
        self.client.http.post("/api/rbac/roles", &request).await
    }

    /// Assign a role to a user.
    pub async fn assign_role(&self, request: AssignRoleRequest) -> ShiiooResult<SuccessResponse> {
        self.client.http.post("/api/rbac/assign-role", &request).await
    }

    /// Check if a user has a permission.
    pub async fn check_permission(
        &self,
        request: CheckPermissionRequest,
    ) -> ShiiooResult<PermissionCheckResponse> {
        self.client
            .http
            .post("/api/rbac/check-permission", &request)
            .await
    }
}

/// Request to create an RBAC role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRbacRoleRequest {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// Request to assign a role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssignRoleRequest {
    pub user_id: String,
    pub role_id: String,
}

/// Request to check a permission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckPermissionRequest {
    pub user_id: String,
    pub resource: Resource,
    pub action: Action,
}

/// Generic success response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessResponse {
    pub success: bool,
    pub message: String,
}

/// Response from permission check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionCheckResponse {
    pub has_permission: bool,
    pub user_id: String,
    pub resource: Resource,
    pub action: Action,
}
