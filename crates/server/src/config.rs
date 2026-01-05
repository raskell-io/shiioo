use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use shiioo_core::approval::ApprovalManager;
use shiioo_core::config_change::ConfigChangeManager;
use shiioo_core::scheduler::RoutineScheduler;
use shiioo_core::storage::{FilesystemBlobStore, JsonlEventLog, RedbIndexStore};
use shiioo_core::workflow::WorkflowExecutor;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(skip)]
    pub data_dir: PathBuf,

    #[serde(default)]
    pub storage: StorageConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    #[serde(default = "default_blob_dir")]
    pub blob_dir: String,

    #[serde(default = "default_event_log_dir")]
    pub event_log_dir: String,

    #[serde(default = "default_index_file")]
    pub index_file: String,
}

fn default_blob_dir() -> String {
    "blobs".to_string()
}

fn default_event_log_dir() -> String {
    "events".to_string()
}

fn default_index_file() -> String {
    "index.redb".to_string()
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            blob_dir: default_blob_dir(),
            event_log_dir: default_event_log_dir(),
            index_file: default_index_file(),
        }
    }
}

impl ServerConfig {
    pub fn load(config_path: &PathBuf, data_dir: PathBuf) -> Result<Self> {
        // Create data directory if it doesn't exist
        std::fs::create_dir_all(&data_dir).context("Failed to create data directory")?;

        // Load config file if it exists, otherwise use defaults
        let mut config: Self = if config_path.exists() {
            let content = std::fs::read_to_string(config_path)
                .context("Failed to read configuration file")?;
            toml::from_str(&content).context("Failed to parse configuration file")?
        } else {
            tracing::info!("Configuration file not found, using defaults");
            Self {
                data_dir: data_dir.clone(),
                storage: Default::default(),
            }
        };

        config.data_dir = data_dir;

        Ok(config)
    }

    /// Get the blob storage path
    pub fn blob_path(&self) -> PathBuf {
        self.data_dir.join(&self.storage.blob_dir)
    }

    /// Get the event log path
    pub fn event_log_path(&self) -> PathBuf {
        self.data_dir.join(&self.storage.event_log_dir)
    }

    /// Get the index file path
    pub fn index_path(&self) -> PathBuf {
        self.data_dir.join(&self.storage.index_file)
    }
}

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub blob_store: Arc<FilesystemBlobStore>,
    pub event_log: Arc<JsonlEventLog>,
    pub index_store: Arc<RedbIndexStore>,
    pub workflow_executor: Arc<WorkflowExecutor>,
    pub routine_scheduler: Arc<RoutineScheduler>,
    pub approval_manager: Arc<ApprovalManager>,
    pub config_change_manager: Arc<ConfigChangeManager>,
}

impl AppState {
    pub fn new(config: &ServerConfig) -> Result<Self> {
        let blob_store = Arc::new(
            FilesystemBlobStore::new(config.blob_path())
                .context("Failed to create blob store")?,
        );

        let event_log = Arc::new(
            JsonlEventLog::new(config.event_log_path()).context("Failed to create event log")?,
        );

        let index_store = Arc::new(
            RedbIndexStore::new(config.index_path()).context("Failed to create index store")?,
        );

        let workflow_executor = Arc::new(WorkflowExecutor::new(
            event_log.clone(),
            blob_store.clone(),
            index_store.clone(),
        ));

        // Phase 5: Routine scheduler, approval boards, and config changes
        let approval_manager = Arc::new(ApprovalManager::new());
        let config_change_manager = Arc::new(ConfigChangeManager::new(approval_manager.clone()));
        let routine_scheduler = Arc::new(RoutineScheduler::new(workflow_executor.clone()));

        Ok(Self {
            blob_store,
            event_log,
            index_store,
            workflow_executor,
            routine_scheduler,
            approval_manager,
            config_change_manager,
        })
    }
}
