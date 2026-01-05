use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;

use crate::tenant::TenantId;
use super::{FilesystemBlobStore, JsonlEventLog, RedbIndexStore};

/// Tenant-scoped blob storage
pub struct TenantBlobStore {
    base_path: PathBuf,
}

impl TenantBlobStore {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    /// Get a blob store for a specific tenant
    pub fn for_tenant(&self, tenant_id: &TenantId) -> Result<FilesystemBlobStore> {
        let tenant_path = self.base_path.join("tenants").join(&tenant_id.0);
        std::fs::create_dir_all(&tenant_path)?;
        FilesystemBlobStore::new(tenant_path)
    }

    /// Get storage path for a tenant
    pub fn tenant_path(&self, tenant_id: &TenantId) -> PathBuf {
        self.base_path.join("tenants").join(&tenant_id.0)
    }
}

/// Tenant-scoped event log
pub struct TenantEventLog {
    base_path: PathBuf,
}

impl TenantEventLog {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    /// Get an event log for a specific tenant
    pub fn for_tenant(&self, tenant_id: &TenantId) -> Result<JsonlEventLog> {
        let tenant_path = self.base_path.join("tenants").join(&tenant_id.0);
        std::fs::create_dir_all(&tenant_path)?;
        let log_path = tenant_path.join("events.jsonl");
        JsonlEventLog::new(log_path)
    }

    /// Get event log path for a tenant
    pub fn tenant_path(&self, tenant_id: &TenantId) -> PathBuf {
        self.base_path.join("tenants").join(&tenant_id.0).join("events.jsonl")
    }
}

/// Tenant-scoped index store
pub struct TenantIndexStore {
    base_path: PathBuf,
}

impl TenantIndexStore {
    pub fn new(base_path: PathBuf) -> Self {
        Self { base_path }
    }

    /// Get an index store for a specific tenant
    pub fn for_tenant(&self, tenant_id: &TenantId) -> Result<RedbIndexStore> {
        let tenant_path = self.base_path.join("tenants").join(&tenant_id.0);
        std::fs::create_dir_all(&tenant_path)?;
        let index_path = tenant_path.join("index.redb");
        RedbIndexStore::new(index_path)
    }

    /// Get index path for a tenant
    pub fn tenant_path(&self, tenant_id: &TenantId) -> PathBuf {
        self.base_path.join("tenants").join(&tenant_id.0).join("index.redb")
    }
}

/// Tenant-scoped storage manager
pub struct TenantStorage {
    blob_store: Arc<TenantBlobStore>,
    event_log: Arc<TenantEventLog>,
    index_store: Arc<TenantIndexStore>,
}

impl TenantStorage {
    /// Create a new tenant storage manager
    pub fn new(base_path: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&base_path)?;

        Ok(Self {
            blob_store: Arc::new(TenantBlobStore::new(base_path.clone())),
            event_log: Arc::new(TenantEventLog::new(base_path.clone())),
            index_store: Arc::new(TenantIndexStore::new(base_path)),
        })
    }

    /// Get blob store for a tenant
    pub fn blob_store(&self, tenant_id: &TenantId) -> Result<FilesystemBlobStore> {
        self.blob_store.for_tenant(tenant_id)
    }

    /// Get event log for a tenant
    pub fn event_log(&self, tenant_id: &TenantId) -> Result<JsonlEventLog> {
        self.event_log.for_tenant(tenant_id)
    }

    /// Get index store for a tenant
    pub fn index_store(&self, tenant_id: &TenantId) -> Result<RedbIndexStore> {
        self.index_store.for_tenant(tenant_id)
    }

    /// Initialize storage for a new tenant
    pub fn initialize_tenant(&self, tenant_id: &TenantId) -> Result<()> {
        // Create all storage directories and files
        let _ = self.blob_store(tenant_id)?;
        let _ = self.event_log(tenant_id)?;
        let _ = self.index_store(tenant_id)?;

        tracing::info!("Initialized storage for tenant: {}", tenant_id.0);
        Ok(())
    }

    /// Delete all data for a tenant
    pub fn delete_tenant_data(&self, tenant_id: &TenantId) -> Result<()> {
        let tenant_blob_path = self.blob_store.tenant_path(tenant_id);
        let tenant_event_path = self.event_log.tenant_path(tenant_id).parent()
            .ok_or_else(|| anyhow::anyhow!("Invalid event log path"))?
            .to_path_buf();

        // Remove tenant directories
        if tenant_blob_path.exists() {
            std::fs::remove_dir_all(&tenant_blob_path)?;
        }
        if tenant_event_path.exists() {
            std::fs::remove_dir_all(&tenant_event_path)?;
        }

        tracing::info!("Deleted storage for tenant: {}", tenant_id.0);
        Ok(())
    }

    /// Get storage statistics for a tenant
    pub fn tenant_stats(&self, tenant_id: &TenantId) -> Result<TenantStorageStats> {
        let blob_path = self.blob_store.tenant_path(tenant_id);
        let event_path = self.event_log.tenant_path(tenant_id);
        let index_path = self.index_store.tenant_path(tenant_id);

        let mut total_bytes = 0u64;
        let mut file_count = 0usize;

        // Calculate blob storage size
        if blob_path.exists() {
            for entry in walkdir::WalkDir::new(&blob_path) {
                if let Ok(entry) = entry {
                    if entry.file_type().is_file() {
                        if let Ok(metadata) = entry.metadata() {
                            total_bytes += metadata.len();
                            file_count += 1;
                        }
                    }
                }
            }
        }

        // Add event log size
        if event_path.exists() {
            if let Ok(metadata) = std::fs::metadata(&event_path) {
                total_bytes += metadata.len();
                file_count += 1;
            }
        }

        // Add index size
        if index_path.exists() {
            if let Ok(metadata) = std::fs::metadata(&index_path) {
                total_bytes += metadata.len();
                file_count += 1;
            }
        }

        Ok(TenantStorageStats {
            total_bytes,
            file_count,
        })
    }
}

/// Storage statistics for a tenant
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TenantStorageStats {
    pub total_bytes: u64,
    pub file_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_tenant_storage_initialization() {
        let temp_dir = TempDir::new().unwrap();
        let storage = TenantStorage::new(temp_dir.path().to_path_buf()).unwrap();

        let tenant_id = TenantId::new("test-tenant");
        storage.initialize_tenant(&tenant_id).unwrap();

        // Verify directories were created
        let blob_path = storage.blob_store.tenant_path(&tenant_id);
        let event_path = storage.event_log.tenant_path(&tenant_id);
        let index_path = storage.index_store.tenant_path(&tenant_id);

        assert!(blob_path.exists());
        assert!(event_path.parent().unwrap().exists());
        assert!(index_path.exists());
    }

    #[test]
    fn test_tenant_isolation() {
        let temp_dir = TempDir::new().unwrap();
        let storage = TenantStorage::new(temp_dir.path().to_path_buf()).unwrap();

        let tenant1 = TenantId::new("tenant1");
        let tenant2 = TenantId::new("tenant2");

        storage.initialize_tenant(&tenant1).unwrap();
        storage.initialize_tenant(&tenant2).unwrap();

        let path1 = storage.blob_store.tenant_path(&tenant1);
        let path2 = storage.blob_store.tenant_path(&tenant2);

        assert_ne!(path1, path2);
        assert!(path1.to_string_lossy().contains("tenant1"));
        assert!(path2.to_string_lossy().contains("tenant2"));
    }

    #[test]
    fn test_delete_tenant_data() {
        let temp_dir = TempDir::new().unwrap();
        let storage = TenantStorage::new(temp_dir.path().to_path_buf()).unwrap();

        let tenant_id = TenantId::new("test-tenant");
        storage.initialize_tenant(&tenant_id).unwrap();

        let blob_path = storage.blob_store.tenant_path(&tenant_id);
        assert!(blob_path.exists());

        storage.delete_tenant_data(&tenant_id).unwrap();
        assert!(!blob_path.exists());
    }

    #[test]
    fn test_tenant_storage_stats() {
        let temp_dir = TempDir::new().unwrap();
        let storage = TenantStorage::new(temp_dir.path().to_path_buf()).unwrap();

        let tenant_id = TenantId::new("test-tenant");
        storage.initialize_tenant(&tenant_id).unwrap();

        let stats = storage.tenant_stats(&tenant_id).unwrap();
        assert!(stats.total_bytes > 0); // Index file should exist
        assert!(stats.file_count > 0);
    }
}
