use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Unique identifier for a tenant
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TenantId(pub String);

impl TenantId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Create a new random tenant ID
    pub fn generate() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

/// Tenant configuration and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tenant {
    pub id: TenantId,
    pub name: String,
    pub description: String,
    pub status: TenantStatus,
    pub quota: TenantQuota,
    pub settings: TenantSettings,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Tenant status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TenantStatus {
    Active,
    Suspended,
    Disabled,
}

/// Resource quota limits for a tenant
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantQuota {
    /// Maximum concurrent workflows
    pub max_concurrent_workflows: Option<u32>,
    /// Maximum workflows per day
    pub max_workflows_per_day: Option<u32>,
    /// Maximum routines
    pub max_routines: Option<u32>,
    /// Maximum storage size in bytes
    pub max_storage_bytes: Option<u64>,
    /// Maximum API requests per minute
    pub max_api_requests_per_minute: Option<u32>,
}

impl Default for TenantQuota {
    fn default() -> Self {
        Self {
            max_concurrent_workflows: Some(10),
            max_workflows_per_day: Some(1000),
            max_routines: Some(50),
            max_storage_bytes: Some(10_000_000_000), // 10GB
            max_api_requests_per_minute: Some(1000),
        }
    }
}

/// Tenant-specific settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantSettings {
    /// Data retention period in days
    pub data_retention_days: u32,
    /// Enable audit logging
    pub enable_audit_logging: bool,
    /// Custom metadata
    pub metadata: HashMap<String, String>,
}

impl Default for TenantSettings {
    fn default() -> Self {
        Self {
            data_retention_days: 90,
            enable_audit_logging: true,
            metadata: HashMap::new(),
        }
    }
}

/// Tenant manager for multi-tenancy support
pub struct TenantManager {
    tenants: Arc<Mutex<HashMap<TenantId, Tenant>>>,
}

impl TenantManager {
    /// Create a new tenant manager
    pub fn new() -> Self {
        Self {
            tenants: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a new tenant
    pub fn register_tenant(&self, tenant: Tenant) -> anyhow::Result<()> {
        let mut tenants = self.tenants.lock().unwrap();

        if tenants.contains_key(&tenant.id) {
            return Err(anyhow::anyhow!("Tenant already exists: {}", tenant.id.0));
        }

        tenants.insert(tenant.id.clone(), tenant);
        Ok(())
    }

    /// Get a tenant by ID
    pub fn get_tenant(&self, tenant_id: &TenantId) -> Option<Tenant> {
        self.tenants.lock().unwrap().get(tenant_id).cloned()
    }

    /// Update a tenant
    pub fn update_tenant(&self, tenant: Tenant) -> anyhow::Result<()> {
        let mut tenants = self.tenants.lock().unwrap();

        if !tenants.contains_key(&tenant.id) {
            return Err(anyhow::anyhow!("Tenant not found: {}", tenant.id.0));
        }

        tenants.insert(tenant.id.clone(), tenant);
        Ok(())
    }

    /// Delete a tenant
    pub fn delete_tenant(&self, tenant_id: &TenantId) -> anyhow::Result<()> {
        let mut tenants = self.tenants.lock().unwrap();

        if tenants.remove(tenant_id).is_none() {
            return Err(anyhow::anyhow!("Tenant not found: {}", tenant_id.0));
        }

        Ok(())
    }

    /// List all tenants
    pub fn list_tenants(&self) -> Vec<Tenant> {
        self.tenants.lock().unwrap().values().cloned().collect()
    }

    /// List active tenants
    pub fn list_active_tenants(&self) -> Vec<Tenant> {
        self.tenants
            .lock()
            .unwrap()
            .values()
            .filter(|t| t.status == TenantStatus::Active)
            .cloned()
            .collect()
    }

    /// Suspend a tenant
    pub fn suspend_tenant(&self, tenant_id: &TenantId) -> anyhow::Result<()> {
        let mut tenants = self.tenants.lock().unwrap();

        let tenant = tenants
            .get_mut(tenant_id)
            .ok_or_else(|| anyhow::anyhow!("Tenant not found: {}", tenant_id.0))?;

        tenant.status = TenantStatus::Suspended;
        tenant.updated_at = Utc::now();

        Ok(())
    }

    /// Activate a tenant
    pub fn activate_tenant(&self, tenant_id: &TenantId) -> anyhow::Result<()> {
        let mut tenants = self.tenants.lock().unwrap();

        let tenant = tenants
            .get_mut(tenant_id)
            .ok_or_else(|| anyhow::anyhow!("Tenant not found: {}", tenant_id.0))?;

        tenant.status = TenantStatus::Active;
        tenant.updated_at = Utc::now();

        Ok(())
    }

    /// Check if a tenant is active
    pub fn is_active(&self, tenant_id: &TenantId) -> bool {
        self.tenants
            .lock()
            .unwrap()
            .get(tenant_id)
            .map(|t| t.status == TenantStatus::Active)
            .unwrap_or(false)
    }

    /// Validate tenant quota
    pub fn check_quota(&self, tenant_id: &TenantId, resource: QuotaResource) -> anyhow::Result<()> {
        let tenants = self.tenants.lock().unwrap();
        let tenant = tenants
            .get(tenant_id)
            .ok_or_else(|| anyhow::anyhow!("Tenant not found: {}", tenant_id.0))?;

        match resource {
            QuotaResource::ConcurrentWorkflows(count) => {
                if let Some(max) = tenant.quota.max_concurrent_workflows {
                    if count > max {
                        return Err(anyhow::anyhow!(
                            "Quota exceeded: max concurrent workflows is {}",
                            max
                        ));
                    }
                }
            }
            QuotaResource::WorkflowsPerDay(count) => {
                if let Some(max) = tenant.quota.max_workflows_per_day {
                    if count > max {
                        return Err(anyhow::anyhow!(
                            "Quota exceeded: max workflows per day is {}",
                            max
                        ));
                    }
                }
            }
            QuotaResource::Routines(count) => {
                if let Some(max) = tenant.quota.max_routines {
                    if count > max {
                        return Err(anyhow::anyhow!("Quota exceeded: max routines is {}", max));
                    }
                }
            }
            QuotaResource::Storage(bytes) => {
                if let Some(max) = tenant.quota.max_storage_bytes {
                    if bytes > max {
                        return Err(anyhow::anyhow!(
                            "Quota exceeded: max storage is {} bytes",
                            max
                        ));
                    }
                }
            }
            QuotaResource::ApiRequests(count) => {
                if let Some(max) = tenant.quota.max_api_requests_per_minute {
                    if count > max {
                        return Err(anyhow::anyhow!(
                            "Quota exceeded: max API requests per minute is {}",
                            max
                        ));
                    }
                }
            }
        }

        Ok(())
    }
}

impl Default for TenantManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Resource types for quota checking
pub enum QuotaResource {
    ConcurrentWorkflows(u32),
    WorkflowsPerDay(u32),
    Routines(u32),
    Storage(u64),
    ApiRequests(u32),
}

/// Tenant context for request isolation
#[derive(Debug, Clone)]
pub struct TenantContext {
    pub tenant_id: TenantId,
    pub request_id: String,
}

impl TenantContext {
    pub fn new(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            request_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    pub fn with_request_id(tenant_id: TenantId, request_id: String) -> Self {
        Self {
            tenant_id,
            request_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_tenant() -> Tenant {
        Tenant {
            id: TenantId::new("test_tenant"),
            name: "Test Tenant".to_string(),
            description: "A test tenant".to_string(),
            status: TenantStatus::Active,
            quota: TenantQuota::default(),
            settings: TenantSettings::default(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_register_tenant() {
        let manager = TenantManager::new();
        let tenant = create_test_tenant();

        manager.register_tenant(tenant.clone()).unwrap();

        let retrieved = manager.get_tenant(&tenant.id).unwrap();
        assert_eq!(retrieved.id, tenant.id);
        assert_eq!(retrieved.name, tenant.name);
    }

    #[test]
    fn test_duplicate_tenant() {
        let manager = TenantManager::new();
        let tenant = create_test_tenant();

        manager.register_tenant(tenant.clone()).unwrap();
        let result = manager.register_tenant(tenant.clone());

        assert!(result.is_err());
    }

    #[test]
    fn test_update_tenant() {
        let manager = TenantManager::new();
        let mut tenant = create_test_tenant();

        manager.register_tenant(tenant.clone()).unwrap();

        tenant.name = "Updated Name".to_string();
        manager.update_tenant(tenant.clone()).unwrap();

        let retrieved = manager.get_tenant(&tenant.id).unwrap();
        assert_eq!(retrieved.name, "Updated Name");
    }

    #[test]
    fn test_delete_tenant() {
        let manager = TenantManager::new();
        let tenant = create_test_tenant();

        manager.register_tenant(tenant.clone()).unwrap();
        manager.delete_tenant(&tenant.id).unwrap();

        assert!(manager.get_tenant(&tenant.id).is_none());
    }

    #[test]
    fn test_suspend_activate_tenant() {
        let manager = TenantManager::new();
        let tenant = create_test_tenant();

        manager.register_tenant(tenant.clone()).unwrap();
        assert!(manager.is_active(&tenant.id));

        manager.suspend_tenant(&tenant.id).unwrap();
        assert!(!manager.is_active(&tenant.id));

        manager.activate_tenant(&tenant.id).unwrap();
        assert!(manager.is_active(&tenant.id));
    }

    #[test]
    fn test_list_active_tenants() {
        let manager = TenantManager::new();

        let tenant1 = create_test_tenant();
        let mut tenant2 = create_test_tenant();
        tenant2.id = TenantId::new("tenant2");

        manager.register_tenant(tenant1.clone()).unwrap();
        manager.register_tenant(tenant2.clone()).unwrap();
        manager.suspend_tenant(&tenant2.id).unwrap();

        let active = manager.list_active_tenants();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, tenant1.id);
    }

    #[test]
    fn test_quota_check() {
        let manager = TenantManager::new();
        let tenant = create_test_tenant();

        manager.register_tenant(tenant.clone()).unwrap();

        // Should pass - within quota
        manager
            .check_quota(&tenant.id, QuotaResource::ConcurrentWorkflows(5))
            .unwrap();

        // Should fail - exceeds quota
        let result = manager.check_quota(&tenant.id, QuotaResource::ConcurrentWorkflows(20));
        assert!(result.is_err());
    }

    #[test]
    fn test_tenant_context() {
        let tenant_id = TenantId::new("test");
        let context = TenantContext::new(tenant_id.clone());

        assert_eq!(context.tenant_id, tenant_id);
        assert!(!context.request_id.is_empty());
    }
}
