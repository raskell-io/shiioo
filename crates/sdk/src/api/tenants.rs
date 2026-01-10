//! Tenants API endpoints.

use crate::client::ShiiooClient;
use crate::error::ShiiooResult;
use serde::{Deserialize, Serialize};
use shiioo_core::storage::TenantStorageStats;
use shiioo_core::tenant::{Tenant, TenantId, TenantQuota, TenantSettings};

/// Tenants API for multi-tenant operations.
pub struct TenantsApi<'a> {
    client: &'a ShiiooClient,
}

impl<'a> TenantsApi<'a> {
    pub(crate) fn new(client: &'a ShiiooClient) -> Self {
        Self { client }
    }

    /// List all tenants.
    pub async fn list(&self) -> ShiiooResult<Vec<Tenant>> {
        let response: ListTenantsResponse = self.client.http.get("/api/tenants").await?;
        Ok(response.tenants)
    }

    /// Get a specific tenant.
    pub async fn get(&self, tenant_id: &TenantId) -> ShiiooResult<Tenant> {
        self.client
            .http
            .get(&format!("/api/tenants/{}", tenant_id.0))
            .await
    }

    /// Register a new tenant.
    pub async fn register(&self, request: RegisterTenantRequest) -> ShiiooResult<Tenant> {
        self.client.http.post("/api/tenants", &request).await
    }

    /// Update a tenant.
    pub async fn update(
        &self,
        tenant_id: &TenantId,
        request: UpdateTenantRequest,
    ) -> ShiiooResult<Tenant> {
        self.client
            .http
            .put(&format!("/api/tenants/{}", tenant_id.0), &request)
            .await
    }

    /// Delete a tenant.
    pub async fn delete(&self, tenant_id: &TenantId) -> ShiiooResult<DeleteTenantResponse> {
        self.client
            .http
            .delete(&format!("/api/tenants/{}", tenant_id.0))
            .await
    }

    /// Suspend a tenant.
    pub async fn suspend(&self, tenant_id: &TenantId) -> ShiiooResult<Tenant> {
        self.client
            .http
            .post(&format!("/api/tenants/{}/suspend", tenant_id.0), &())
            .await
    }

    /// Activate a tenant.
    pub async fn activate(&self, tenant_id: &TenantId) -> ShiiooResult<Tenant> {
        self.client
            .http
            .post(&format!("/api/tenants/{}/activate", tenant_id.0), &())
            .await
    }

    /// Get storage statistics for a tenant.
    pub async fn storage_stats(&self, tenant_id: &TenantId) -> ShiiooResult<TenantStorageStats> {
        self.client
            .http
            .get(&format!("/api/tenants/{}/storage-stats", tenant_id.0))
            .await
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ListTenantsResponse {
    tenants: Vec<Tenant>,
}

/// Request to register a new tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterTenantRequest {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota: Option<TenantQuota>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<TenantSettings>,
}

/// Request to update a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateTenantRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quota: Option<TenantQuota>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings: Option<TenantSettings>,
}

/// Response from deleting a tenant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteTenantResponse {
    pub message: String,
}
