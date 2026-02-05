//! MCP (Model Context Protocol) bindings for agents.
//!
//! MCP bindings define what external systems an agent can interact with
//! and with what credentials.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::PolicyRule;

/// Binding between an agent and an MCP tool server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpBinding {
    /// Binding identifier.
    pub id: String,

    /// MCP server reference.
    pub server: McpServerRef,

    /// Tools available from this server.
    pub tools: McpToolAccess,

    /// Credential reference for this binding.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<CredentialRef>,

    /// Tool tier override (for policy evaluation).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier_override: Option<u8>,

    /// Additional policy constraints for this binding.
    #[serde(default)]
    pub policy_constraints: Vec<PolicyRule>,
}

impl McpBinding {
    /// Create a new MCP binding.
    pub fn new(id: impl Into<String>, server: McpServerRef) -> Self {
        Self {
            id: id.into(),
            server,
            tools: McpToolAccess::All,
            credentials: None,
            tier_override: None,
            policy_constraints: Vec::new(),
        }
    }

    /// Set the tool access configuration.
    pub fn with_tools(mut self, tools: McpToolAccess) -> Self {
        self.tools = tools;
        self
    }

    /// Set the credentials.
    pub fn with_credentials(mut self, credentials: CredentialRef) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// Set the tier override.
    pub fn with_tier(mut self, tier: u8) -> Self {
        self.tier_override = Some(tier);
        self
    }

    /// Add a policy constraint.
    pub fn with_constraint(mut self, constraint: PolicyRule) -> Self {
        self.policy_constraints.push(constraint);
        self
    }
}

/// Reference to an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpServerRef {
    /// Built-in Shiioo MCP server.
    Builtin,

    /// External MCP server (stdio transport).
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },

    /// External MCP server (HTTP/SSE transport).
    Http {
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
    },

    /// Reference by server ID (configured elsewhere).
    Reference { server_id: String },
}

impl McpServerRef {
    /// Create a builtin server reference.
    pub fn builtin() -> Self {
        Self::Builtin
    }

    /// Create a stdio server reference.
    pub fn stdio(command: impl Into<String>) -> Self {
        Self::Stdio {
            command: command.into(),
            args: Vec::new(),
            env: HashMap::new(),
        }
    }

    /// Create a stdio server reference with args.
    pub fn stdio_with_args(
        command: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::Stdio {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
            env: HashMap::new(),
        }
    }

    /// Create an HTTP server reference.
    pub fn http(url: impl Into<String>) -> Self {
        Self::Http {
            url: url.into(),
            headers: HashMap::new(),
        }
    }

    /// Create a reference to a configured server.
    pub fn reference(server_id: impl Into<String>) -> Self {
        Self::Reference {
            server_id: server_id.into(),
        }
    }
}

/// Tool access configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpToolAccess {
    /// All tools from the server.
    All,

    /// Specific tools only.
    Specific { tool_ids: Vec<String> },

    /// All except specific tools.
    Except { excluded_tool_ids: Vec<String> },
}

impl Default for McpToolAccess {
    fn default() -> Self {
        Self::All
    }
}

impl McpToolAccess {
    /// Create access for all tools.
    pub fn all() -> Self {
        Self::All
    }

    /// Create access for specific tools.
    pub fn specific(tool_ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Specific {
            tool_ids: tool_ids.into_iter().map(Into::into).collect(),
        }
    }

    /// Create access for all except specific tools.
    pub fn except(excluded_tool_ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Except {
            excluded_tool_ids: excluded_tool_ids.into_iter().map(Into::into).collect(),
        }
    }

    /// Check if a tool is accessible.
    pub fn allows(&self, tool_id: &str) -> bool {
        match self {
            Self::All => true,
            Self::Specific { tool_ids } => tool_ids.iter().any(|t| t == tool_id),
            Self::Except { excluded_tool_ids } => {
                !excluded_tool_ids.iter().any(|t| t == tool_id)
            }
        }
    }
}

/// Reference to credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialRef {
    /// Reference to a secret by ID.
    Secret { secret_id: String },

    /// Reference to an environment variable.
    EnvVar { name: String },

    /// Derived from agent's secret grants.
    FromGrants { grant_id: String },
}

impl CredentialRef {
    /// Create a reference to a secret.
    pub fn secret(secret_id: impl Into<String>) -> Self {
        Self::Secret {
            secret_id: secret_id.into(),
        }
    }

    /// Create a reference to an environment variable.
    pub fn env_var(name: impl Into<String>) -> Self {
        Self::EnvVar { name: name.into() }
    }

    /// Create a reference from agent's grants.
    pub fn from_grants(grant_id: impl Into<String>) -> Self {
        Self::FromGrants {
            grant_id: grant_id.into(),
        }
    }
}

/// Grant giving an agent access to a secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretGrant {
    /// Grant identifier.
    pub id: String,

    /// Secret being granted.
    pub secret_id: String,

    /// What the agent can do with this secret.
    pub permissions: SecretPermissions,

    /// Conditions for the grant.
    #[serde(default)]
    pub conditions: Vec<SecretCondition>,

    /// Expiration.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

impl SecretGrant {
    /// Create a new secret grant.
    pub fn new(id: impl Into<String>, secret_id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            secret_id: secret_id.into(),
            permissions: SecretPermissions::use_only(),
            conditions: Vec::new(),
            expires_at: None,
        }
    }

    /// Set the permissions.
    pub fn with_permissions(mut self, permissions: SecretPermissions) -> Self {
        self.permissions = permissions;
        self
    }

    /// Add a condition.
    pub fn with_condition(mut self, condition: SecretCondition) -> Self {
        self.conditions.push(condition);
        self
    }

    /// Set the expiration.
    pub fn with_expiration(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }

    /// Check if the grant has expired.
    pub fn is_expired(&self) -> bool {
        self.expires_at
            .map(|exp| exp < Utc::now())
            .unwrap_or(false)
    }
}

/// Permissions for a secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretPermissions {
    /// Can read the secret value.
    pub read: bool,

    /// Can use the secret (without seeing the value).
    #[serde(rename = "use")]
    pub use_: bool,

    /// Can rotate the secret.
    pub rotate: bool,
}

impl SecretPermissions {
    /// Create permissions that only allow using the secret.
    pub fn use_only() -> Self {
        Self {
            read: false,
            use_: true,
            rotate: false,
        }
    }

    /// Create permissions that allow reading and using the secret.
    pub fn read_and_use() -> Self {
        Self {
            read: true,
            use_: true,
            rotate: false,
        }
    }

    /// Create full permissions.
    pub fn full() -> Self {
        Self {
            read: true,
            use_: true,
            rotate: true,
        }
    }
}

impl Default for SecretPermissions {
    fn default() -> Self {
        Self::use_only()
    }
}

/// Conditions for a secret grant.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SecretCondition {
    /// Only for specific tools.
    ForTools { tool_ids: Vec<String> },

    /// Only for specific domains.
    ForDomains { domains: Vec<String> },

    /// Only during specific time windows.
    TimeWindow {
        start_hour: u8,
        end_hour: u8,
        timezone: String,
    },
}

impl SecretCondition {
    /// Create a condition for specific tools.
    pub fn for_tools(tool_ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::ForTools {
            tool_ids: tool_ids.into_iter().map(Into::into).collect(),
        }
    }

    /// Create a condition for specific domains.
    pub fn for_domains(domains: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::ForDomains {
            domains: domains.into_iter().map(Into::into).collect(),
        }
    }

    /// Create a time window condition.
    pub fn time_window(start_hour: u8, end_hour: u8, timezone: impl Into<String>) -> Self {
        Self::TimeWindow {
            start_hour,
            end_hour,
            timezone: timezone.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_binding_creation() {
        let binding = McpBinding::new("github", McpServerRef::reference("github-mcp"))
            .with_tools(McpToolAccess::all())
            .with_credentials(CredentialRef::from_grants("github-token"));

        assert_eq!(binding.id, "github");
        assert!(binding.credentials.is_some());
    }

    #[test]
    fn test_mcp_server_ref_stdio() {
        let server = McpServerRef::stdio_with_args("npx", ["-y", "@modelcontextprotocol/server-github"]);
        assert!(matches!(server, McpServerRef::Stdio { command, args, .. } if command == "npx" && args.len() == 2));
    }

    #[test]
    fn test_mcp_tool_access_allows() {
        let all = McpToolAccess::all();
        assert!(all.allows("any_tool"));

        let specific = McpToolAccess::specific(["tool1", "tool2"]);
        assert!(specific.allows("tool1"));
        assert!(!specific.allows("tool3"));

        let except = McpToolAccess::except(["dangerous_tool"]);
        assert!(except.allows("safe_tool"));
        assert!(!except.allows("dangerous_tool"));
    }

    #[test]
    fn test_credential_ref() {
        let secret = CredentialRef::secret("my-secret");
        assert!(matches!(secret, CredentialRef::Secret { secret_id } if secret_id == "my-secret"));

        let env = CredentialRef::env_var("API_KEY");
        assert!(matches!(env, CredentialRef::EnvVar { name } if name == "API_KEY"));
    }

    #[test]
    fn test_secret_grant() {
        let grant = SecretGrant::new("github-access", "github-pat")
            .with_permissions(SecretPermissions::use_only())
            .with_condition(SecretCondition::for_tools(["repo_read", "repo_write"]));

        assert_eq!(grant.id, "github-access");
        assert_eq!(grant.secret_id, "github-pat");
        assert!(!grant.permissions.read);
        assert!(grant.permissions.use_);
        assert_eq!(grant.conditions.len(), 1);
    }

    #[test]
    fn test_secret_grant_expiration() {
        let expired_grant = SecretGrant::new("expired", "secret")
            .with_expiration(Utc::now() - chrono::Duration::hours(1));

        assert!(expired_grant.is_expired());

        let valid_grant = SecretGrant::new("valid", "secret")
            .with_expiration(Utc::now() + chrono::Duration::hours(1));

        assert!(!valid_grant.is_expired());
    }
}
