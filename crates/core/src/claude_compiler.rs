// Claude Config Compiler - generates .claude/config.json from organization config

use crate::types::{
    ClaudeConfig, ClaudeSettings, McpServerConfig, Organization, PolicySpec, RoleId, RoleSpec,
    ToolConfig,
};
use anyhow::Result;
use std::collections::HashMap;
use std::path::PathBuf;

/// Compiler for generating Claude Code configuration from organization setup
pub struct ClaudeCompiler {
    org: Organization,
    roles: Vec<RoleSpec>,
    policies: Vec<PolicySpec>,
}

impl ClaudeCompiler {
    pub fn new(org: Organization, roles: Vec<RoleSpec>, policies: Vec<PolicySpec>) -> Self {
        Self { org, roles, policies }
    }

    /// Generate Claude configuration for a specific role
    pub fn compile_for_role(&self, role_id: &RoleId) -> Result<ClaudeConfig> {
        let role = self
            .roles
            .iter()
            .find(|r| &r.id == role_id)
            .ok_or_else(|| anyhow::anyhow!("Role {} not found", role_id.0))?;

        // Generate MCP server configs
        let mcp_servers = self.generate_mcp_servers();

        // Generate tool configs based on role permissions
        let tools = self.generate_tool_configs(role);

        // Generate Claude settings based on role budgets
        let settings = self.generate_settings(role);

        Ok(ClaudeConfig {
            mcp_servers,
            tools,
            settings,
        })
    }

    /// Generate MCP server configurations
    fn generate_mcp_servers(&self) -> HashMap<String, McpServerConfig> {
        let mut servers = HashMap::new();

        // Add Shiioo MCP server
        servers.insert(
            "shiioo".to_string(),
            McpServerConfig {
                command: "shiioo-mcp".to_string(),
                args: vec![],
                env: [
                    ("SHIIOO_DATA_DIR".to_string(), "./data".to_string()),
                    ("SHIIOO_ORG_ID".to_string(), self.org.id.0.clone()),
                ]
                .iter()
                .cloned()
                .collect(),
            },
        );

        servers
    }

    /// Generate tool configurations based on role permissions
    fn generate_tool_configs(&self, role: &RoleSpec) -> Vec<ToolConfig> {
        let mut tools = Vec::new();

        // Define all available tools
        let all_tools = vec![
            ("context_get", 0),
            ("context_search", 0),
            ("context_events", 0),
            ("repo_read", 0),
            ("web_fetch", 0),
            ("repo_write", 1),
            ("database_execute", 2),
            ("deploy_production", 2),
        ];

        for (tool_name, tier) in all_tools {
            let enabled = if role.allowed_tools.is_empty() {
                // Empty allowlist means all tools allowed
                true
            } else {
                role.allowed_tools.contains(&tool_name.to_string())
            };

            let requires_approval = role
                .requires_approval_for
                .iter()
                .any(|req| req == tool_name || req == &format!("tier{}", tier));

            tools.push(ToolConfig {
                name: tool_name.to_string(),
                enabled,
                tier,
                requires_approval,
            });
        }

        tools
    }

    /// Generate Claude settings from role budgets
    fn generate_settings(&self, role: &RoleSpec) -> ClaudeSettings {
        let max_tokens = role.budgets.daily_tokens.map(|t| t / 10); // Conservative per-request limit

        ClaudeSettings {
            max_tokens,
            temperature: Some(0.7),
            model: Some("claude-opus-4-5".to_string()),
        }
    }

    /// Write configuration to .claude/config.json
    pub fn write_config(&self, role_id: &RoleId, output_dir: &PathBuf) -> Result<()> {
        let config = self.compile_for_role(role_id)?;

        let config_path = output_dir.join(".claude").join("config.json");

        // Create .claude directory if it doesn't exist
        std::fs::create_dir_all(config_path.parent().unwrap())?;

        let json = serde_json::to_string_pretty(&config)?;
        std::fs::write(&config_path, json)?;

        Ok(())
    }

    /// Generate a README for the .claude directory explaining the setup
    pub fn generate_readme(&self, role_id: &RoleId) -> Result<String> {
        let role = self
            .roles
            .iter()
            .find(|r| &r.id == role_id)
            .ok_or_else(|| anyhow::anyhow!("Role {} not found", role_id.0))?;

        let readme = format!(
            r#"# Claude Code Configuration for {}

This configuration was automatically generated from the {} organization.

## Role: {}

{}

## Allowed Tools

{}

## Budget Limits

- Daily Tokens: {}
- Daily Cost: {}

## Approval Required For

{}

## Organization

- Organization: {}
- Team Structure: {} teams
- Total Members: {} people

## MCP Servers

This configuration includes access to the Shiioo MCP server for:
- Context search and retrieval
- Repository file access
- Event log querying

## Usage

This configuration is automatically loaded by Claude Code when you start a session.
The MCP server provides tools that respect your role's permissions and policies.

---

Generated at: {}
"#,
            role.name,
            self.org.name,
            role.name,
            role.description,
            role.allowed_tools
                .iter()
                .map(|t| format!("- {}", t))
                .collect::<Vec<_>>()
                .join("\n"),
            role.budgets
                .daily_tokens
                .map(|t| t.to_string())
                .unwrap_or_else(|| "Unlimited".to_string()),
            role.budgets
                .daily_cost_cents
                .map(|c| format!("${:.2}", c as f64 / 100.0))
                .unwrap_or_else(|| "Unlimited".to_string()),
            if role.requires_approval_for.is_empty() {
                "None".to_string()
            } else {
                role.requires_approval_for
                    .iter()
                    .map(|a| format!("- {}", a))
                    .collect::<Vec<_>>()
                    .join("\n")
            },
            self.org.name,
            self.org.teams.len(),
            self.org.people.len(),
            chrono::Utc::now().to_rfc3339()
        );

        Ok(readme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{OrgChart, OrgId, Person, PersonId, RoleBudgets, Team, TeamId};
    use chrono::Utc;

    fn create_test_setup() -> (Organization, Vec<RoleSpec>) {
        let org = Organization {
            id: OrgId::new("test_org"),
            name: "Test Organization".to_string(),
            description: "A test organization".to_string(),
            teams: vec![Team {
                id: TeamId::new("engineering"),
                name: "Engineering".to_string(),
                description: "Engineering team".to_string(),
                lead: Some(PersonId::new("alice")),
                members: vec![PersonId::new("alice")],
                parent_team: None,
            }],
            people: vec![Person {
                id: PersonId::new("alice"),
                name: "Alice".to_string(),
                email: "alice@example.com".to_string(),
                role: RoleId::new("engineer"),
                team: TeamId::new("engineering"),
                reports_to: None,
                can_approve: vec![],
            }],
            org_chart: OrgChart {
                root_team: TeamId::new("engineering"),
                reporting_structure: HashMap::new(),
            },
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let roles = vec![
            RoleSpec {
                id: RoleId::new("engineer"),
                name: "Software Engineer".to_string(),
                description: "Can read and write code".to_string(),
                prompt_template: "You are a software engineer".to_string(),
                allowed_tools: vec![
                    "context_search".to_string(),
                    "repo_read".to_string(),
                    "repo_write".to_string(),
                ],
                budgets: RoleBudgets {
                    daily_tokens: Some(100000),
                    daily_cost_cents: Some(1000),
                },
                requires_approval_for: vec!["repo_write".to_string()],
            },
            RoleSpec {
                id: RoleId::new("analyst"),
                name: "Data Analyst".to_string(),
                description: "Read-only access".to_string(),
                prompt_template: "You are a data analyst".to_string(),
                allowed_tools: vec!["context_search".to_string(), "repo_read".to_string()],
                budgets: RoleBudgets {
                    daily_tokens: Some(50000),
                    daily_cost_cents: Some(500),
                },
                requires_approval_for: vec![],
            },
        ];

        (org, roles)
    }

    #[test]
    fn test_compile_for_role() {
        let (org, roles) = create_test_setup();
        let compiler = ClaudeCompiler::new(org, roles, vec![]);

        let config = compiler
            .compile_for_role(&RoleId::new("engineer"))
            .unwrap();

        // Check MCP servers
        assert!(config.mcp_servers.contains_key("shiioo"));

        // Check tools
        assert!(!config.tools.is_empty());
        let context_search = config.tools.iter().find(|t| t.name == "context_search");
        assert!(context_search.is_some());
        assert!(context_search.unwrap().enabled);

        let repo_write = config.tools.iter().find(|t| t.name == "repo_write");
        assert!(repo_write.is_some());
        assert!(repo_write.unwrap().enabled);
        assert!(repo_write.unwrap().requires_approval);

        // Check settings
        assert!(config.settings.max_tokens.is_some());
        assert_eq!(config.settings.max_tokens.unwrap(), 10000); // 100000 / 10
    }

    #[test]
    fn test_tool_permissions() {
        let (org, roles) = create_test_setup();
        let compiler = ClaudeCompiler::new(org, roles, vec![]);

        let config = compiler.compile_for_role(&RoleId::new("analyst")).unwrap();

        // Analyst should have limited tools
        let repo_write = config.tools.iter().find(|t| t.name == "repo_write");
        assert!(repo_write.is_some());
        assert!(!repo_write.unwrap().enabled); // Not in allowed_tools

        let context_search = config.tools.iter().find(|t| t.name == "context_search");
        assert!(context_search.is_some());
        assert!(context_search.unwrap().enabled);
    }

    #[test]
    fn test_generate_readme() {
        let (org, roles) = create_test_setup();
        let compiler = ClaudeCompiler::new(org, roles, vec![]);

        let readme = compiler
            .generate_readme(&RoleId::new("engineer"))
            .unwrap();

        assert!(readme.contains("Software Engineer"));
        assert!(readme.contains("Test Organization"));
        assert!(readme.contains("context_search"));
        assert!(readme.contains("repo_write"));
    }
}
