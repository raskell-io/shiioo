use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use shiioo_core::agent::{
    AgentOrchestrator, AgentRuntime, DefaultPolicyEngine, FormatConverterRegistry,
    InMemoryMcpResolver, InMemoryToolExecutor, OrchestrationConfig, RuntimeConfig, SkillRegistry,
};
use shiioo_core::analytics::PerformanceAnalytics;
use shiioo_core::approval::ApprovalManager;
use shiioo_core::audit::AuditLog;
use shiioo_core::cluster::ClusterManager;
use shiioo_core::cluster::NodeId;
use shiioo_core::compliance::{ComplianceChecker, SecurityScanner};
use shiioo_core::config_change::ConfigChangeManager;
use shiioo_core::metrics::MetricsCollector;
use shiioo_core::rbac::RbacManager;
use shiioo_core::scheduler::RoutineScheduler;
use shiioo_core::secrets::SecretManager;
use shiioo_core::storage::{
    FilesystemBlobStore, JsonlEventLog, RedbAgentStore, RedbIndexStore, RedbPolicyStorage,
    TenantStorage,
};
use shiioo_core::tenant::TenantManager;
use shiioo_core::workflow::WorkflowExecutor;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Environment variable for the encryption key used by SecretManager.
/// Must be exactly 32 bytes (256 bits) for AES-256 encryption.
pub const ENCRYPTION_KEY_ENV: &str = "SHIIOO_ENCRYPTION_KEY";

/// Required length for the encryption key (32 bytes for AES-256).
const ENCRYPTION_KEY_LENGTH: usize = 32;

/// Load and validate the encryption key from environment variable.
///
/// The key must be exactly 32 bytes for AES-256 encryption.
/// If not set, returns an error with instructions on how to generate a key.
fn load_encryption_key() -> Result<Vec<u8>> {
    match std::env::var(ENCRYPTION_KEY_ENV) {
        Ok(key) => {
            let key_bytes = key.as_bytes();
            if key_bytes.len() != ENCRYPTION_KEY_LENGTH {
                return Err(anyhow!(
                    "Invalid {} length: expected {} bytes, got {} bytes. \
                    Generate a key with: openssl rand -base64 32 | head -c 32",
                    ENCRYPTION_KEY_ENV,
                    ENCRYPTION_KEY_LENGTH,
                    key_bytes.len()
                ));
            }
            Ok(key_bytes.to_vec())
        }
        Err(_) => Err(anyhow!(
            "Missing required environment variable: {}. \
            This key is used to encrypt secrets and must be exactly 32 bytes. \
            Generate one with: openssl rand -base64 32 | head -c 32",
            ENCRYPTION_KEY_ENV
        )),
    }
}

/// Concrete type for AgentRuntime with our chosen implementations
pub type ConcreteAgentRuntime = AgentRuntime<
    DefaultPolicyEngine<RedbPolicyStorage>,
    InMemoryMcpResolver,
    InMemoryToolExecutor,
    JsonlEventLog,
>;

/// Concrete type for AgentOrchestrator with our chosen implementations
pub type ConcreteAgentOrchestrator = AgentOrchestrator<
    DefaultPolicyEngine<RedbPolicyStorage>,
    InMemoryMcpResolver,
    InMemoryToolExecutor,
    JsonlEventLog,
    RedbAgentStore,
>;

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

    #[serde(default = "default_skills_cache_dir")]
    pub skills_cache_dir: String,

    #[serde(default = "default_local_skills_dir")]
    pub local_skills_dir: String,
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

fn default_skills_cache_dir() -> String {
    "skills_cache".to_string()
}

fn default_local_skills_dir() -> String {
    "skills".to_string()
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            blob_dir: default_blob_dir(),
            event_log_dir: default_event_log_dir(),
            index_file: default_index_file(),
            skills_cache_dir: default_skills_cache_dir(),
            local_skills_dir: default_local_skills_dir(),
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
    #[allow(dead_code)]
    pub blob_store: Arc<FilesystemBlobStore>,
    pub event_log: Arc<JsonlEventLog>,
    pub index_store: Arc<RedbIndexStore>,
    pub agent_store: Arc<RedbAgentStore>,
    pub workflow_executor: Arc<WorkflowExecutor>,
    pub routine_scheduler: Arc<RoutineScheduler>,
    pub approval_manager: Arc<ApprovalManager>,
    pub config_change_manager: Arc<ConfigChangeManager>,
    pub metrics: Arc<MetricsCollector>,
    pub analytics: Arc<PerformanceAnalytics>,
    pub tenant_manager: Arc<TenantManager>,
    pub tenant_storage: Arc<TenantStorage>,
    pub cluster_manager: Arc<ClusterManager>,
    pub secret_manager: Arc<SecretManager>,
    pub audit_log: Arc<AuditLog>,
    pub rbac_manager: Arc<RbacManager>,
    pub compliance_checker: Arc<ComplianceChecker>,
    pub security_scanner: Arc<SecurityScanner>,
    // Agent Runtime (Phase 13-14)
    pub agent_runtime: Arc<ConcreteAgentRuntime>,
    pub agent_orchestrator: Arc<ConcreteAgentOrchestrator>,
    // Skill Registry & Import
    pub skill_registry: Arc<RwLock<SkillRegistry>>,
    pub format_converter_registry: Arc<FormatConverterRegistry>,
    pub skills_cache_dir: PathBuf,
    pub local_skills_dir: PathBuf,
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

        let agent_store = Arc::new(
            RedbAgentStore::new(config.data_dir.join("agents.redb"))
                .context("Failed to create agent store")?,
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

        // Phase 6: Observability - metrics and analytics
        let metrics = Arc::new(MetricsCollector::new());
        let analytics = Arc::new(PerformanceAnalytics::new());

        // Phase 7: Multi-tenancy and high availability
        let tenant_manager = Arc::new(TenantManager::new());
        let tenant_storage = Arc::new(
            TenantStorage::new(config.data_dir.clone())
                .context("Failed to create tenant storage")?,
        );

        // Generate a unique node ID for this server instance
        let local_node_id = NodeId::generate();
        let cluster_manager = Arc::new(ClusterManager::new(local_node_id, 30)); // 30 sec heartbeat timeout

        // Phase 8: Secret management
        let encryption_key = load_encryption_key()
            .context("Failed to load encryption key for secret management")?;
        let secret_manager = Arc::new(SecretManager::new(&encryption_key));

        // Phase 9: Security and compliance
        let audit_log = Arc::new(AuditLog::new());
        let rbac_manager = Arc::new(RbacManager::new());

        // Initialize system roles
        for role in shiioo_core::rbac::create_system_roles() {
            rbac_manager.register_role(role)
                .context("Failed to register system role")?;
        }

        let compliance_checker = Arc::new(ComplianceChecker::new(
            (*audit_log).clone(),
            (*rbac_manager).clone(),
        ));
        let security_scanner = Arc::new(SecurityScanner::new((*audit_log).clone()));

        // Phase 13-14: Agent Runtime and Orchestration
        let policy_storage = RedbPolicyStorage::new(config.data_dir.join("policies.redb"))
            .context("Failed to create policy storage")?;
        let policy_engine = Arc::new(DefaultPolicyEngine::new(policy_storage));
        let mcp_resolver = Arc::new(InMemoryMcpResolver::new());
        let tool_executor = Arc::new(InMemoryToolExecutor::new());

        let runtime_config = RuntimeConfig {
            max_task_duration_secs: 300,
            enforce_budgets: true,
            require_policy_check: true,
            default_tool_timeout_secs: 60,
        };

        let agent_runtime = Arc::new(AgentRuntime::new(
            runtime_config,
            policy_engine,
            mcp_resolver,
            tool_executor,
            event_log.clone(),
        ));

        let orchestration_config = OrchestrationConfig::default();
        let agent_orchestrator = Arc::new(AgentOrchestrator::new(
            orchestration_config,
            agent_runtime.clone(),
            agent_store.clone(),
            event_log.clone(),
        ));

        // Skill Registry & Import
        let skill_registry = Arc::new(RwLock::new(SkillRegistry::new()));
        let format_converter_registry = Arc::new(FormatConverterRegistry::new());
        let skills_cache_dir = config.data_dir.join(&config.storage.skills_cache_dir);
        let local_skills_dir = config.data_dir.join(&config.storage.local_skills_dir);

        // Ensure skill directories exist
        std::fs::create_dir_all(&skills_cache_dir)
            .context("Failed to create skills cache directory")?;
        std::fs::create_dir_all(&local_skills_dir)
            .context("Failed to create local skills directory")?;

        Ok(Self {
            blob_store,
            event_log,
            index_store,
            agent_store,
            workflow_executor,
            routine_scheduler,
            approval_manager,
            config_change_manager,
            metrics,
            analytics,
            tenant_manager,
            tenant_storage,
            cluster_manager,
            secret_manager,
            audit_log,
            rbac_manager,
            compliance_checker,
            security_scanner,
            agent_runtime,
            agent_orchestrator,
            skill_registry,
            format_converter_registry,
            skills_cache_dir,
            local_skills_dir,
        })
    }
}
