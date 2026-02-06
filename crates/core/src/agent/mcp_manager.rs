//! MCP binding manager for agents.
//!
//! Manages MCP server connections, tool access, and credential resolution
//! for agents in the virtual enterprise.

use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{
    Agent, AgentId, CredentialRef, McpBinding, McpServerRef, McpToolAccess, PolicyRule,
    SecretCondition, SecretGrant,
};

/// Resolved MCP configuration for an agent.
#[derive(Debug, Clone)]
pub struct ResolvedMcpConfig {
    /// Agent ID this config belongs to.
    pub agent_id: AgentId,

    /// Resolved server configurations.
    pub servers: Vec<ResolvedMcpServer>,
}

/// A resolved MCP server configuration.
#[derive(Debug, Clone)]
pub struct ResolvedMcpServer {
    /// Binding ID.
    pub binding_id: String,

    /// Server configuration.
    pub server: McpServerConfig,

    /// Available tools from this server.
    pub tools: Vec<ResolvedTool>,

    /// Policy constraints for this server.
    pub policy_constraints: Vec<PolicyRule>,
}

/// Concrete MCP server configuration (ready to spawn).
#[derive(Debug, Clone)]
pub struct McpServerConfig {
    /// Server type.
    pub server_type: McpServerType,

    /// Environment variables (with secrets resolved).
    pub env: HashMap<String, String>,
}

/// Type of MCP server.
#[derive(Debug, Clone)]
pub enum McpServerType {
    /// Built-in Shiioo MCP server.
    Builtin,

    /// External stdio-based server.
    Stdio {
        command: String,
        args: Vec<String>,
    },

    /// External HTTP-based server.
    Http {
        url: String,
        headers: HashMap<String, String>,
    },
}

/// A resolved tool with its configuration.
#[derive(Debug, Clone)]
pub struct ResolvedTool {
    /// Tool ID.
    pub tool_id: String,

    /// Tool tier (0 = read-only, 1 = write, 2 = dangerous).
    pub tier: u8,

    /// Whether the tool is enabled for this agent.
    pub enabled: bool,

    /// Credential to use for this tool (if any).
    pub credential: Option<ResolvedCredential>,
}

/// A resolved credential (value retrieved from secret store).
#[derive(Debug, Clone)]
pub struct ResolvedCredential {
    /// Credential type.
    pub credential_type: CredentialType,

    /// The credential value (only if agent has read permission).
    /// For use-only permissions, this will be None and the value
    /// will be injected at runtime.
    pub value: Option<String>,

    /// Secret ID this credential came from.
    pub source_secret_id: Option<String>,
}

/// Type of credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialType {
    /// API key or token.
    ApiKey,
    /// Bearer token.
    BearerToken,
    /// Basic auth (username:password).
    BasicAuth,
    /// OAuth token.
    OAuth,
    /// Custom header value.
    CustomHeader { header_name: String },
}

/// Trait for secret storage (to resolve credentials).
#[async_trait::async_trait]
pub trait SecretStore: Send + Sync {
    /// Get a secret value by ID.
    async fn get_secret(&self, secret_id: &str) -> Result<Option<String>>;

    /// Check if a secret exists.
    async fn secret_exists(&self, secret_id: &str) -> Result<bool>;
}

/// Trait for MCP server registry.
#[async_trait::async_trait]
pub trait McpServerRegistry: Send + Sync {
    /// Get server configuration by ID.
    async fn get_server(&self, server_id: &str) -> Result<Option<McpServerRef>>;

    /// List available tools for a server.
    async fn list_tools(&self, server_id: &str) -> Result<Vec<ToolInfo>>;
}

/// Information about a tool.
#[derive(Debug, Clone)]
pub struct ToolInfo {
    /// Tool ID.
    pub id: String,

    /// Tool name.
    pub name: String,

    /// Tool description.
    pub description: String,

    /// Tool tier.
    pub tier: u8,

    /// Whether the tool requires credentials.
    pub requires_credentials: bool,
}

/// Manager for MCP bindings.
pub struct McpBindingManager<S: SecretStore, R: McpServerRegistry> {
    secret_store: S,
    server_registry: R,

    /// Cache of resolved configurations.
    config_cache: Arc<RwLock<HashMap<AgentId, ResolvedMcpConfig>>>,

    /// Default tool tiers (tool_id -> tier).
    default_tool_tiers: HashMap<String, u8>,
}

impl<S: SecretStore, R: McpServerRegistry> McpBindingManager<S, R> {
    /// Create a new MCP binding manager.
    pub fn new(secret_store: S, server_registry: R) -> Self {
        Self {
            secret_store,
            server_registry,
            config_cache: Arc::new(RwLock::new(HashMap::new())),
            default_tool_tiers: Self::default_tool_tiers(),
        }
    }

    /// Get default tool tiers.
    fn default_tool_tiers() -> HashMap<String, u8> {
        let mut tiers = HashMap::new();

        // Read-only tools (tier 0)
        for tool in &[
            "repo_read",
            "context_search",
            "web_fetch",
            "db_query",
            "audit_log_read",
            "security_scan",
        ] {
            tiers.insert(tool.to_string(), 0);
        }

        // Write tools (tier 1)
        for tool in &[
            "repo_write",
            "pr_create",
            "pr_comment",
            "jira_create",
            "jira_comment",
            "data_export",
        ] {
            tiers.insert(tool.to_string(), 1);
        }

        // Dangerous tools (tier 2)
        for tool in &[
            "deploy",
            "deploy_production",
            "deploy_staging",
            "db_write",
            "db_migrate",
            "secret_create",
            "secret_rotate",
        ] {
            tiers.insert(tool.to_string(), 2);
        }

        tiers
    }

    /// Resolve MCP configuration for an agent.
    pub async fn resolve_config(&self, agent: &Agent) -> Result<ResolvedMcpConfig> {
        // Check cache first
        {
            let cache = self.config_cache.read().await;
            if let Some(config) = cache.get(&agent.id) {
                return Ok(config.clone());
            }
        }

        // Resolve each binding
        let mut servers = Vec::new();
        for binding in &agent.mcp_bindings {
            let server = self
                .resolve_binding(agent, binding)
                .await
                .with_context(|| format!("Failed to resolve binding: {}", binding.id))?;
            servers.push(server);
        }

        let config = ResolvedMcpConfig {
            agent_id: agent.id.clone(),
            servers,
        };

        // Cache it
        {
            let mut cache = self.config_cache.write().await;
            cache.insert(agent.id.clone(), config.clone());
        }

        Ok(config)
    }

    /// Resolve a single MCP binding.
    async fn resolve_binding(
        &self,
        agent: &Agent,
        binding: &McpBinding,
    ) -> Result<ResolvedMcpServer> {
        // Resolve server reference
        let server_config = self.resolve_server_ref(&binding.server).await?;

        // Resolve tools
        let tools = self
            .resolve_tools(agent, binding, &binding.server)
            .await?;

        // Resolve credentials and add to env
        let mut env = server_config.env.clone();
        if let Some(cred_ref) = &binding.credentials {
            if let Some(cred) = self.resolve_credential(agent, cred_ref).await? {
                // Add credential to environment
                match &cred.credential_type {
                    CredentialType::ApiKey => {
                        if let Some(value) = &cred.value {
                            env.insert("API_KEY".to_string(), value.clone());
                        }
                    }
                    CredentialType::BearerToken => {
                        if let Some(value) = &cred.value {
                            env.insert("BEARER_TOKEN".to_string(), value.clone());
                        }
                    }
                    CredentialType::CustomHeader { header_name } => {
                        if let Some(value) = &cred.value {
                            env.insert(
                                format!("HEADER_{}", header_name.to_uppercase().replace('-', "_")),
                                value.clone(),
                            );
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(ResolvedMcpServer {
            binding_id: binding.id.clone(),
            server: McpServerConfig {
                server_type: server_config.server_type,
                env,
            },
            tools,
            policy_constraints: binding.policy_constraints.clone(),
        })
    }

    /// Resolve a server reference to a concrete configuration.
    async fn resolve_server_ref(&self, server_ref: &McpServerRef) -> Result<McpServerConfig> {
        // Handle reference resolution first (non-recursive)
        let resolved_ref = match server_ref {
            McpServerRef::Reference { server_id } => {
                let fetched = self
                    .server_registry
                    .get_server(server_id)
                    .await?
                    .ok_or_else(|| anyhow!("Server not found: {}", server_id))?;

                // Prevent reference chains
                if matches!(fetched, McpServerRef::Reference { .. }) {
                    return Err(anyhow!("Server reference chain detected: {}", server_id));
                }

                fetched
            }
            other => other.clone(),
        };

        // Now resolve the concrete type
        match resolved_ref {
            McpServerRef::Builtin => Ok(McpServerConfig {
                server_type: McpServerType::Builtin,
                env: HashMap::new(),
            }),

            McpServerRef::Stdio { command, args, env } => Ok(McpServerConfig {
                server_type: McpServerType::Stdio {
                    command: command.clone(),
                    args: args.clone(),
                },
                env: env.clone(),
            }),

            McpServerRef::Http { url, headers } => Ok(McpServerConfig {
                server_type: McpServerType::Http {
                    url: url.clone(),
                    headers: headers.clone(),
                },
                env: HashMap::new(),
            }),

            McpServerRef::Reference { server_id } => {
                // This shouldn't happen due to the check above, but handle it anyway
                Err(anyhow!("Unexpected reference after resolution: {}", server_id))
            }
        }
    }

    /// Resolve tools for a binding.
    async fn resolve_tools(
        &self,
        agent: &Agent,
        binding: &McpBinding,
        server_ref: &McpServerRef,
    ) -> Result<Vec<ResolvedTool>> {
        // Get available tools from the server
        let server_id = match server_ref {
            McpServerRef::Reference { server_id } => server_id.clone(),
            McpServerRef::Builtin => "builtin".to_string(),
            _ => binding.id.clone(),
        };

        let available_tools = self.server_registry.list_tools(&server_id).await?;

        // Filter based on access configuration
        let mut resolved_tools = Vec::new();
        for tool_info in available_tools {
            let enabled = match &binding.tools {
                McpToolAccess::All => true,
                McpToolAccess::Specific { tool_ids } => tool_ids.contains(&tool_info.id),
                McpToolAccess::Except { excluded_tool_ids } => {
                    !excluded_tool_ids.contains(&tool_info.id)
                }
            };

            // Determine tier (use override if provided)
            let tier = binding
                .tier_override
                .unwrap_or_else(|| {
                    self.default_tool_tiers
                        .get(&tool_info.id)
                        .copied()
                        .unwrap_or(tool_info.tier)
                });

            // Resolve credential for this tool if needed
            let credential = if tool_info.requires_credentials {
                if let Some(cred_ref) = &binding.credentials {
                    self.resolve_credential_for_tool(agent, cred_ref, &tool_info.id)
                        .await?
                } else {
                    None
                }
            } else {
                None
            };

            resolved_tools.push(ResolvedTool {
                tool_id: tool_info.id,
                tier,
                enabled,
                credential,
            });
        }

        Ok(resolved_tools)
    }

    /// Resolve a credential reference.
    async fn resolve_credential(
        &self,
        agent: &Agent,
        cred_ref: &CredentialRef,
    ) -> Result<Option<ResolvedCredential>> {
        match cred_ref {
            CredentialRef::Secret { secret_id } => {
                // Check if agent has access via grants
                let grant = agent
                    .secret_grants
                    .iter()
                    .find(|g| g.secret_id == *secret_id);

                match grant {
                    Some(grant) => {
                        // Check if grant is expired
                        if grant.is_expired() {
                            return Err(anyhow!("Secret grant has expired: {}", secret_id));
                        }

                        // Get the value only if read permission is granted
                        let value = if grant.permissions.read {
                            self.secret_store.get_secret(secret_id).await?
                        } else if grant.permissions.use_ {
                            // Agent can use but not read - value will be injected at runtime
                            None
                        } else {
                            return Err(anyhow!(
                                "Agent does not have permission to use secret: {}",
                                secret_id
                            ));
                        };

                        Ok(Some(ResolvedCredential {
                            credential_type: CredentialType::ApiKey,
                            value,
                            source_secret_id: Some(secret_id.clone()),
                        }))
                    }
                    None => Err(anyhow!(
                        "Agent does not have a grant for secret: {}",
                        secret_id
                    )),
                }
            }

            CredentialRef::EnvVar { name } => {
                let value = std::env::var(name).ok();
                Ok(Some(ResolvedCredential {
                    credential_type: CredentialType::ApiKey,
                    value,
                    source_secret_id: None,
                }))
            }

            CredentialRef::FromGrants { grant_id } => {
                let grant = agent
                    .secret_grants
                    .iter()
                    .find(|g| g.id == *grant_id)
                    .ok_or_else(|| anyhow!("Grant not found: {}", grant_id))?;

                if grant.is_expired() {
                    return Err(anyhow!("Secret grant has expired: {}", grant_id));
                }

                let value = if grant.permissions.read {
                    self.secret_store.get_secret(&grant.secret_id).await?
                } else {
                    None
                };

                Ok(Some(ResolvedCredential {
                    credential_type: CredentialType::ApiKey,
                    value,
                    source_secret_id: Some(grant.secret_id.clone()),
                }))
            }
        }
    }

    /// Resolve a credential for a specific tool.
    async fn resolve_credential_for_tool(
        &self,
        agent: &Agent,
        cred_ref: &CredentialRef,
        tool_id: &str,
    ) -> Result<Option<ResolvedCredential>> {
        // Find a grant that applies to this tool
        let applicable_grant = match cred_ref {
            CredentialRef::FromGrants { grant_id } => {
                agent.secret_grants.iter().find(|g| {
                    g.id == *grant_id && self.grant_applies_to_tool(g, tool_id)
                })
            }
            CredentialRef::Secret { secret_id } => {
                agent.secret_grants.iter().find(|g| {
                    g.secret_id == *secret_id && self.grant_applies_to_tool(g, tool_id)
                })
            }
            CredentialRef::EnvVar { .. } => {
                // Env vars apply to all tools
                return self.resolve_credential(agent, cred_ref).await;
            }
        };

        match applicable_grant {
            Some(grant) => {
                if grant.is_expired() {
                    return Ok(None);
                }

                let value = if grant.permissions.read {
                    self.secret_store.get_secret(&grant.secret_id).await?
                } else {
                    None
                };

                Ok(Some(ResolvedCredential {
                    credential_type: CredentialType::ApiKey,
                    value,
                    source_secret_id: Some(grant.secret_id.clone()),
                }))
            }
            None => Ok(None),
        }
    }

    /// Check if a grant applies to a specific tool.
    fn grant_applies_to_tool(&self, grant: &SecretGrant, tool_id: &str) -> bool {
        if grant.conditions.is_empty() {
            return true;
        }

        for condition in &grant.conditions {
            match condition {
                SecretCondition::ForTools { tool_ids } => {
                    if !tool_ids.iter().any(|t| t == tool_id) {
                        return false;
                    }
                }
                // Other conditions don't affect tool applicability
                _ => {}
            }
        }

        true
    }

    /// Invalidate cached configuration for an agent.
    pub async fn invalidate_cache(&self, agent_id: &AgentId) {
        let mut cache = self.config_cache.write().await;
        cache.remove(agent_id);
    }

    /// Clear all cached configurations.
    pub async fn clear_cache(&self) {
        let mut cache = self.config_cache.write().await;
        cache.clear();
    }

    /// Get a tool's tier.
    pub fn get_tool_tier(&self, tool_id: &str) -> u8 {
        self.default_tool_tiers.get(tool_id).copied().unwrap_or(1)
    }

    /// Check if an agent can use a specific tool.
    pub async fn can_use_tool(&self, agent: &Agent, tool_id: &str) -> Result<bool> {
        let config = self.resolve_config(agent).await?;

        for server in &config.servers {
            for tool in &server.tools {
                if tool.tool_id == tool_id && tool.enabled {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    /// Get all available tools for an agent.
    pub async fn get_available_tools(&self, agent: &Agent) -> Result<Vec<ResolvedTool>> {
        let config = self.resolve_config(agent).await?;

        let mut tools = Vec::new();
        for server in config.servers {
            for tool in server.tools {
                if tool.enabled {
                    tools.push(tool);
                }
            }
        }

        Ok(tools)
    }
}

/// In-memory secret store for testing.
#[derive(Default)]
pub struct InMemorySecretStore {
    secrets: HashMap<String, String>,
}

impl InMemorySecretStore {
    /// Create a new empty secret store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a secret.
    pub fn add_secret(&mut self, id: impl Into<String>, value: impl Into<String>) {
        self.secrets.insert(id.into(), value.into());
    }
}

#[async_trait::async_trait]
impl SecretStore for InMemorySecretStore {
    async fn get_secret(&self, secret_id: &str) -> Result<Option<String>> {
        Ok(self.secrets.get(secret_id).cloned())
    }

    async fn secret_exists(&self, secret_id: &str) -> Result<bool> {
        Ok(self.secrets.contains_key(secret_id))
    }
}

/// In-memory MCP server registry for testing.
#[derive(Default)]
pub struct InMemoryServerRegistry {
    servers: HashMap<String, McpServerRef>,
    tools: HashMap<String, Vec<ToolInfo>>,
}

impl InMemoryServerRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a server.
    pub fn add_server(&mut self, id: impl Into<String>, server: McpServerRef) {
        self.servers.insert(id.into(), server);
    }

    /// Add tools for a server.
    pub fn add_tools(&mut self, server_id: impl Into<String>, tools: Vec<ToolInfo>) {
        self.tools.insert(server_id.into(), tools);
    }

    /// Create a registry with default builtin tools.
    pub fn with_builtin_tools() -> Self {
        let mut registry = Self::new();

        registry.add_server("builtin", McpServerRef::Builtin);
        registry.add_tools(
            "builtin",
            vec![
                ToolInfo {
                    id: "repo_read".to_string(),
                    name: "Read Repository".to_string(),
                    description: "Read files from a repository".to_string(),
                    tier: 0,
                    requires_credentials: false,
                },
                ToolInfo {
                    id: "repo_write".to_string(),
                    name: "Write Repository".to_string(),
                    description: "Write files to a repository".to_string(),
                    tier: 1,
                    requires_credentials: false,
                },
                ToolInfo {
                    id: "context_search".to_string(),
                    name: "Context Search".to_string(),
                    description: "Search code and documentation".to_string(),
                    tier: 0,
                    requires_credentials: false,
                },
                ToolInfo {
                    id: "web_fetch".to_string(),
                    name: "Web Fetch".to_string(),
                    description: "Fetch content from URLs".to_string(),
                    tier: 0,
                    requires_credentials: false,
                },
                ToolInfo {
                    id: "deploy".to_string(),
                    name: "Deploy".to_string(),
                    description: "Deploy to an environment".to_string(),
                    tier: 2,
                    requires_credentials: true,
                },
            ],
        );

        registry
    }
}

#[async_trait::async_trait]
impl McpServerRegistry for InMemoryServerRegistry {
    async fn get_server(&self, server_id: &str) -> Result<Option<McpServerRef>> {
        Ok(self.servers.get(server_id).cloned())
    }

    async fn list_tools(&self, server_id: &str) -> Result<Vec<ToolInfo>> {
        Ok(self.tools.get(server_id).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentOrganization, AgentPolicies, AgentSkills, SecretPermissions};
    use crate::types::TeamId;

    fn create_test_agent_with_bindings(
        bindings: Vec<McpBinding>,
        grants: Vec<SecretGrant>,
    ) -> Agent {
        Agent::builder("test-agent", "Test Agent")
            .organization(AgentOrganization::new(TeamId::new("test-team")))
            .mcp_bindings(bindings)
            .secret_grants(grants)
            .build()
    }

    #[tokio::test]
    async fn test_resolve_builtin_binding() {
        let secret_store = InMemorySecretStore::new();
        let server_registry = InMemoryServerRegistry::with_builtin_tools();
        let manager = McpBindingManager::new(secret_store, server_registry);

        let binding = McpBinding::new("builtin", McpServerRef::Builtin);
        let agent = create_test_agent_with_bindings(vec![binding], vec![]);

        let config = manager.resolve_config(&agent).await.unwrap();
        assert_eq!(config.servers.len(), 1);
        assert_eq!(config.servers[0].binding_id, "builtin");
        assert!(matches!(
            config.servers[0].server.server_type,
            McpServerType::Builtin
        ));
    }

    #[tokio::test]
    async fn test_resolve_tools_with_access() {
        let secret_store = InMemorySecretStore::new();
        let server_registry = InMemoryServerRegistry::with_builtin_tools();
        let manager = McpBindingManager::new(secret_store, server_registry);

        // Only allow specific tools
        let binding = McpBinding::new("builtin", McpServerRef::Builtin)
            .with_tools(McpToolAccess::specific(["repo_read", "context_search"]));

        let agent = create_test_agent_with_bindings(vec![binding], vec![]);

        let config = manager.resolve_config(&agent).await.unwrap();
        let enabled_tools: Vec<_> = config.servers[0]
            .tools
            .iter()
            .filter(|t| t.enabled)
            .map(|t| t.tool_id.as_str())
            .collect();

        assert!(enabled_tools.contains(&"repo_read"));
        assert!(enabled_tools.contains(&"context_search"));
        assert!(!enabled_tools.contains(&"deploy"));
    }

    #[tokio::test]
    async fn test_resolve_with_secret_grant() {
        let mut secret_store = InMemorySecretStore::new();
        secret_store.add_secret("github-token", "ghp_test123");

        let server_registry = InMemoryServerRegistry::with_builtin_tools();
        let manager = McpBindingManager::new(secret_store, server_registry);

        let binding = McpBinding::new("builtin", McpServerRef::Builtin)
            .with_credentials(CredentialRef::from_grants("github-grant"));

        let grant = SecretGrant::new("github-grant", "github-token")
            .with_permissions(SecretPermissions::read_and_use());

        let agent = create_test_agent_with_bindings(vec![binding], vec![grant]);

        let config = manager.resolve_config(&agent).await.unwrap();
        assert!(config.servers[0].server.env.contains_key("API_KEY"));
    }

    #[tokio::test]
    async fn test_resolve_without_grant_fails() {
        let secret_store = InMemorySecretStore::new();
        let server_registry = InMemoryServerRegistry::with_builtin_tools();
        let manager = McpBindingManager::new(secret_store, server_registry);

        let binding = McpBinding::new("builtin", McpServerRef::Builtin)
            .with_credentials(CredentialRef::secret("missing-secret"));

        let agent = create_test_agent_with_bindings(vec![binding], vec![]);

        let result = manager.resolve_config(&agent).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_can_use_tool() {
        let secret_store = InMemorySecretStore::new();
        let server_registry = InMemoryServerRegistry::with_builtin_tools();
        let manager = McpBindingManager::new(secret_store, server_registry);

        let binding = McpBinding::new("builtin", McpServerRef::Builtin)
            .with_tools(McpToolAccess::specific(["repo_read"]));

        let agent = create_test_agent_with_bindings(vec![binding], vec![]);

        assert!(manager.can_use_tool(&agent, "repo_read").await.unwrap());
        assert!(!manager.can_use_tool(&agent, "deploy").await.unwrap());
    }

    #[tokio::test]
    async fn test_get_available_tools() {
        let secret_store = InMemorySecretStore::new();
        let server_registry = InMemoryServerRegistry::with_builtin_tools();
        let manager = McpBindingManager::new(secret_store, server_registry);

        let binding = McpBinding::new("builtin", McpServerRef::Builtin);
        let agent = create_test_agent_with_bindings(vec![binding], vec![]);

        let tools = manager.get_available_tools(&agent).await.unwrap();
        assert!(!tools.is_empty());

        // Check that tiers are set correctly
        let repo_read = tools.iter().find(|t| t.tool_id == "repo_read").unwrap();
        assert_eq!(repo_read.tier, 0);

        let deploy = tools.iter().find(|t| t.tool_id == "deploy").unwrap();
        assert_eq!(deploy.tier, 2);
    }

    #[tokio::test]
    async fn test_tier_override() {
        let secret_store = InMemorySecretStore::new();
        let server_registry = InMemoryServerRegistry::with_builtin_tools();
        let manager = McpBindingManager::new(secret_store, server_registry);

        let binding = McpBinding::new("builtin", McpServerRef::Builtin)
            .with_tier(1); // Override all tools to tier 1

        let agent = create_test_agent_with_bindings(vec![binding], vec![]);

        let config = manager.resolve_config(&agent).await.unwrap();
        for tool in &config.servers[0].tools {
            assert_eq!(tool.tier, 1);
        }
    }

    #[tokio::test]
    async fn test_cache_invalidation() {
        let secret_store = InMemorySecretStore::new();
        let server_registry = InMemoryServerRegistry::with_builtin_tools();
        let manager = McpBindingManager::new(secret_store, server_registry);

        let binding = McpBinding::new("builtin", McpServerRef::Builtin);
        let agent = create_test_agent_with_bindings(vec![binding], vec![]);

        // First resolve
        let _ = manager.resolve_config(&agent).await.unwrap();

        // Check cache
        {
            let cache = manager.config_cache.read().await;
            assert!(cache.contains_key(&agent.id));
        }

        // Invalidate
        manager.invalidate_cache(&agent.id).await;

        // Check cache is empty
        {
            let cache = manager.config_cache.read().await;
            assert!(!cache.contains_key(&agent.id));
        }
    }

    #[test]
    fn test_default_tool_tiers() {
        let secret_store = InMemorySecretStore::new();
        let server_registry = InMemoryServerRegistry::new();
        let manager = McpBindingManager::new(secret_store, server_registry);

        assert_eq!(manager.get_tool_tier("repo_read"), 0);
        assert_eq!(manager.get_tool_tier("repo_write"), 1);
        assert_eq!(manager.get_tool_tier("deploy"), 2);
        assert_eq!(manager.get_tool_tier("unknown"), 1); // Default
    }

    #[test]
    fn test_grant_applies_to_tool() {
        let secret_store = InMemorySecretStore::new();
        let server_registry = InMemoryServerRegistry::new();
        let manager = McpBindingManager::new(secret_store, server_registry);

        // Grant without conditions applies to all tools
        let grant_all = SecretGrant::new("grant-all", "secret");
        assert!(manager.grant_applies_to_tool(&grant_all, "any_tool"));

        // Grant with tool condition
        let grant_specific = SecretGrant::new("grant-specific", "secret")
            .with_condition(SecretCondition::for_tools(["repo_read", "repo_write"]));

        assert!(manager.grant_applies_to_tool(&grant_specific, "repo_read"));
        assert!(!manager.grant_applies_to_tool(&grant_specific, "deploy"));
    }
}
