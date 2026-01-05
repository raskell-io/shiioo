// Policy engine for governance and authorization

use crate::types::{ConfigDiff, PolicyId, PolicyRule, PolicySpec, RoleId, RoleSpec};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Policy decision
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
    RequiresApproval { approvers: Vec<String> },
}

/// Budget usage for a role
#[derive(Debug, Clone, Default)]
pub struct BudgetUsage {
    pub tokens_used: u64,
    pub cost_cents: u64,
    pub last_reset: DateTime<Utc>,
}

/// Policy context for evaluation
#[derive(Debug, Clone)]
pub struct PolicyContext {
    pub role_id: RoleId,
    pub tool_id: String,
    pub tool_tier: u8, // 0 = read-only, 1 = write, 2 = dangerous
    pub parameters: serde_json::Value,
    pub timestamp: DateTime<Utc>,
}

/// Policy engine trait
#[async_trait::async_trait]
pub trait PolicyEngine: Send + Sync {
    /// Check if a tool call is allowed for a role
    async fn check_tool_call(&self, context: &PolicyContext) -> Result<PolicyDecision>;

    /// Check if a configuration change is allowed
    async fn check_config_change(
        &self,
        proposed_by: &str,
        diff: &ConfigDiff,
    ) -> Result<PolicyDecision>;

    /// Record tool usage for budget tracking
    async fn record_usage(
        &self,
        role_id: &RoleId,
        tokens: u64,
        cost_cents: u64,
    ) -> Result<()>;

    /// Load policies
    async fn load_policies(&self, policies: Vec<PolicySpec>) -> Result<()>;

    /// Load roles
    async fn load_roles(&self, roles: Vec<RoleSpec>) -> Result<()>;

    /// Get a role by ID
    async fn get_role(&self, role_id: &RoleId) -> Result<Option<RoleSpec>>;

    /// Get budget usage for a role
    async fn get_budget_usage(&self, role_id: &RoleId) -> Result<BudgetUsage>;
}

/// In-memory policy engine implementation
pub struct InMemoryPolicyEngine {
    policies: Arc<RwLock<HashMap<PolicyId, PolicySpec>>>,
    roles: Arc<RwLock<HashMap<RoleId, RoleSpec>>>,
    budget_usage: Arc<RwLock<HashMap<RoleId, BudgetUsage>>>,
}

impl InMemoryPolicyEngine {
    pub fn new() -> Self {
        Self {
            policies: Arc::new(RwLock::new(HashMap::new())),
            roles: Arc::new(RwLock::new(HashMap::new())),
            budget_usage: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check if a role is allowed to use a specific tool
    async fn check_role_tool_permission(
        &self,
        role: &RoleSpec,
        tool_id: &str,
    ) -> PolicyDecision {
        // Check if tool is explicitly allowed
        if !role.allowed_tools.is_empty() && !role.allowed_tools.contains(&tool_id.to_string()) {
            return PolicyDecision::Deny {
                reason: format!(
                    "Tool '{}' not in allowed list for role '{}'",
                    tool_id, role.id.0
                ),
            };
        }

        PolicyDecision::Allow
    }

    /// Check if approval is required for this tool
    async fn check_approval_requirement(
        &self,
        role: &RoleSpec,
        tool_id: &str,
        tool_tier: u8,
    ) -> PolicyDecision {
        // Check tool-specific approval requirements
        if role.requires_approval_for.contains(&tool_id.to_string()) {
            return PolicyDecision::RequiresApproval {
                approvers: vec!["ceo".to_string(), "cto".to_string()], // TODO: configurable
            };
        }

        // Check tier-based approval requirements
        let tier_str = format!("tier{}", tool_tier);
        if role.requires_approval_for.contains(&tier_str) {
            return PolicyDecision::RequiresApproval {
                approvers: vec!["ceo".to_string()],
            };
        }

        PolicyDecision::Allow
    }

    /// Check budget limits
    async fn check_budget_limits(&self, role: &RoleSpec, role_id: &RoleId) -> PolicyDecision {
        let usage_map = self.budget_usage.read().await;
        let usage = usage_map.get(role_id).cloned().unwrap_or_default();

        // Check if we need to reset (new day)
        let now = Utc::now();
        let should_reset = usage.last_reset.date_naive() < now.date_naive();

        if !should_reset {
            // Check daily token limit
            if let Some(limit) = role.budgets.daily_tokens {
                if usage.tokens_used >= limit {
                    return PolicyDecision::Deny {
                        reason: format!(
                            "Daily token budget exceeded: {} / {}",
                            usage.tokens_used, limit
                        ),
                    };
                }
            }

            // Check daily cost limit
            if let Some(limit) = role.budgets.daily_cost_cents {
                if usage.cost_cents >= limit {
                    return PolicyDecision::Deny {
                        reason: format!(
                            "Daily cost budget exceeded: {} / {} cents",
                            usage.cost_cents, limit
                        ),
                    };
                }
            }
        }

        PolicyDecision::Allow
    }

    /// Evaluate policy rules against a context
    async fn evaluate_policy_rules(&self, context: &PolicyContext) -> PolicyDecision {
        let policies = self.policies.read().await;

        for policy in policies.values() {
            for rule in &policy.rules {
                match rule {
                    PolicyRule::DenyPath { patterns } => {
                        // Check if any parameter values match denied patterns
                        if let Some(obj) = context.parameters.as_object() {
                            for (key, value) in obj {
                                if let Some(val_str) = value.as_str() {
                                    for pattern in patterns {
                                        if val_str.contains(pattern) {
                                            return PolicyDecision::Deny {
                                                reason: format!(
                                                    "Path matches denied pattern '{}' in parameter '{}'",
                                                    pattern, key
                                                ),
                                            };
                                        }
                                    }
                                }
                            }
                        }
                    }
                    PolicyRule::AllowDomain { domains } => {
                        // For web fetch tools, check domain allowlist
                        if context.tool_id == "web_fetch" {
                            if let Some(url) = context.parameters.get("url").and_then(|v| v.as_str())
                            {
                                let is_allowed = domains.iter().any(|d| url.contains(d));
                                if !is_allowed {
                                    return PolicyDecision::Deny {
                                        reason: format!(
                                            "Domain not in allowlist. Allowed domains: {}",
                                            domains.join(", ")
                                        ),
                                    };
                                }
                            }
                        }
                    }
                    PolicyRule::RequireApproval { tool_ids } => {
                        if tool_ids.contains(&context.tool_id) {
                            return PolicyDecision::RequiresApproval {
                                approvers: vec!["ceo".to_string()],
                            };
                        }
                    }
                    PolicyRule::EnforceEnvironment { environment } => {
                        // Check environment-specific constraints
                        // For now, this is a placeholder
                        if environment == "production" {
                            // Production might require additional checks
                            // Could check for specific parameter patterns, etc.
                        }
                    }
                }
            }
        }

        PolicyDecision::Allow
    }
}

impl Default for InMemoryPolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl PolicyEngine for InMemoryPolicyEngine {
    async fn check_tool_call(&self, context: &PolicyContext) -> Result<PolicyDecision> {
        // Get the role
        let roles = self.roles.read().await;
        let role = roles
            .get(&context.role_id)
            .context(format!("Role not found: {}", context.role_id.0))?;

        // 1. Check role-based tool permissions
        let decision = self.check_role_tool_permission(role, &context.tool_id).await;
        if matches!(decision, PolicyDecision::Deny { .. }) {
            return Ok(decision);
        }

        // 2. Check budget limits
        let decision = self.check_budget_limits(role, &context.role_id).await;
        if matches!(decision, PolicyDecision::Deny { .. }) {
            return Ok(decision);
        }

        // 3. Check approval requirements
        let decision = self
            .check_approval_requirement(role, &context.tool_id, context.tool_tier)
            .await;
        if matches!(decision, PolicyDecision::RequiresApproval { .. }) {
            return Ok(decision);
        }

        // 4. Evaluate policy rules
        let decision = self.evaluate_policy_rules(context).await;
        if !matches!(decision, PolicyDecision::Allow) {
            return Ok(decision);
        }

        Ok(PolicyDecision::Allow)
    }

    async fn check_config_change(
        &self,
        _proposed_by: &str,
        diff: &ConfigDiff,
    ) -> Result<PolicyDecision> {
        // For now, all config changes require approval
        // In a real system, you might have admin roles that can approve their own changes

        let has_changes = !diff.roles_added.is_empty()
            || !diff.roles_modified.is_empty()
            || !diff.roles_removed.is_empty()
            || !diff.policies_added.is_empty()
            || !diff.policies_modified.is_empty()
            || !diff.policies_removed.is_empty();

        if !has_changes {
            return Ok(PolicyDecision::Allow);
        }

        // Count significant changes
        let change_count = diff.roles_added.len()
            + diff.roles_modified.len()
            + diff.roles_removed.len()
            + diff.policies_added.len()
            + diff.policies_modified.len()
            + diff.policies_removed.len();

        if change_count > 5 {
            // Large changes require multiple approvers
            Ok(PolicyDecision::RequiresApproval {
                approvers: vec![
                    "ceo".to_string(),
                    "cto".to_string(),
                    "security_lead".to_string(),
                ],
            })
        } else {
            // Small changes require single approver
            Ok(PolicyDecision::RequiresApproval {
                approvers: vec!["ceo".to_string(), "cto".to_string()],
            })
        }
    }

    async fn record_usage(
        &self,
        role_id: &RoleId,
        tokens: u64,
        cost_cents: u64,
    ) -> Result<()> {
        let mut usage_map = self.budget_usage.write().await;
        let now = Utc::now();

        let usage = usage_map.entry(role_id.clone()).or_insert(BudgetUsage {
            tokens_used: 0,
            cost_cents: 0,
            last_reset: now,
        });

        // Reset if it's a new day
        if usage.last_reset.date_naive() < now.date_naive() {
            usage.tokens_used = 0;
            usage.cost_cents = 0;
            usage.last_reset = now;
        }

        usage.tokens_used += tokens;
        usage.cost_cents += cost_cents;

        Ok(())
    }

    async fn load_policies(&self, policies: Vec<PolicySpec>) -> Result<()> {
        let mut policy_map = self.policies.write().await;
        for policy in policies {
            policy_map.insert(policy.id.clone(), policy);
        }
        Ok(())
    }

    async fn load_roles(&self, roles: Vec<RoleSpec>) -> Result<()> {
        let mut role_map = self.roles.write().await;
        for role in roles {
            role_map.insert(role.id.clone(), role);
        }
        Ok(())
    }

    async fn get_role(&self, role_id: &RoleId) -> Result<Option<RoleSpec>> {
        let roles = self.roles.read().await;
        Ok(roles.get(role_id).cloned())
    }

    async fn get_budget_usage(&self, role_id: &RoleId) -> Result<BudgetUsage> {
        let usage_map = self.budget_usage.read().await;
        Ok(usage_map.get(role_id).cloned().unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{RoleBudgets, RoleId};

    #[tokio::test]
    async fn test_policy_engine_allow() {
        let engine = InMemoryPolicyEngine::new();

        let role = RoleSpec {
            id: RoleId::new("analyst"),
            name: "Analyst".to_string(),
            description: "Data analyst role".to_string(),
            prompt_template: "You are an analyst".to_string(),
            allowed_tools: vec!["context_search".to_string(), "web_fetch".to_string()],
            budgets: RoleBudgets {
                daily_tokens: Some(100000),
                daily_cost_cents: Some(1000),
            },
            requires_approval_for: vec![],
        };

        engine.load_roles(vec![role]).await.unwrap();

        let context = PolicyContext {
            role_id: RoleId::new("analyst"),
            tool_id: "context_search".to_string(),
            tool_tier: 0,
            parameters: serde_json::json!({"query": "test"}),
            timestamp: Utc::now(),
        };

        let decision = engine.check_tool_call(&context).await.unwrap();
        assert_eq!(decision, PolicyDecision::Allow);
    }

    #[tokio::test]
    async fn test_policy_engine_deny_tool_not_allowed() {
        let engine = InMemoryPolicyEngine::new();

        let role = RoleSpec {
            id: RoleId::new("analyst"),
            name: "Analyst".to_string(),
            description: "Data analyst role".to_string(),
            prompt_template: "You are an analyst".to_string(),
            allowed_tools: vec!["context_search".to_string()],
            budgets: RoleBudgets {
                daily_tokens: None,
                daily_cost_cents: None,
            },
            requires_approval_for: vec![],
        };

        engine.load_roles(vec![role]).await.unwrap();

        let context = PolicyContext {
            role_id: RoleId::new("analyst"),
            tool_id: "web_fetch".to_string(),
            tool_tier: 0,
            parameters: serde_json::json!({"url": "https://example.com"}),
            timestamp: Utc::now(),
        };

        let decision = engine.check_tool_call(&context).await.unwrap();
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn test_policy_engine_budget_limit() {
        let engine = InMemoryPolicyEngine::new();

        let role = RoleSpec {
            id: RoleId::new("analyst"),
            name: "Analyst".to_string(),
            description: "Data analyst role".to_string(),
            prompt_template: "You are an analyst".to_string(),
            allowed_tools: vec!["context_search".to_string()],
            budgets: RoleBudgets {
                daily_tokens: Some(1000),
                daily_cost_cents: None,
            },
            requires_approval_for: vec![],
        };

        engine.load_roles(vec![role]).await.unwrap();

        // Record usage that exceeds limit
        engine
            .record_usage(&RoleId::new("analyst"), 1500, 50)
            .await
            .unwrap();

        let context = PolicyContext {
            role_id: RoleId::new("analyst"),
            tool_id: "context_search".to_string(),
            tool_tier: 0,
            parameters: serde_json::json!({"query": "test"}),
            timestamp: Utc::now(),
        };

        let decision = engine.check_tool_call(&context).await.unwrap();
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }

    #[tokio::test]
    async fn test_policy_engine_requires_approval() {
        let engine = InMemoryPolicyEngine::new();

        let role = RoleSpec {
            id: RoleId::new("engineer"),
            name: "Engineer".to_string(),
            description: "Software engineer role".to_string(),
            prompt_template: "You are an engineer".to_string(),
            allowed_tools: vec!["repo_write".to_string()],
            budgets: RoleBudgets {
                daily_tokens: None,
                daily_cost_cents: None,
            },
            requires_approval_for: vec!["repo_write".to_string()],
        };

        engine.load_roles(vec![role]).await.unwrap();

        let context = PolicyContext {
            role_id: RoleId::new("engineer"),
            tool_id: "repo_write".to_string(),
            tool_tier: 1,
            parameters: serde_json::json!({"path": "src/main.rs"}),
            timestamp: Utc::now(),
        };

        let decision = engine.check_tool_call(&context).await.unwrap();
        assert!(matches!(decision, PolicyDecision::RequiresApproval { .. }));
    }

    #[tokio::test]
    async fn test_budget_tracking() {
        let engine = InMemoryPolicyEngine::new();
        let role_id = RoleId::new("analyst");

        // Record usage
        engine.record_usage(&role_id, 1000, 50).await.unwrap();
        engine.record_usage(&role_id, 500, 25).await.unwrap();

        let usage = engine.get_budget_usage(&role_id).await.unwrap();
        assert_eq!(usage.tokens_used, 1500);
        assert_eq!(usage.cost_cents, 75);
    }

    #[tokio::test]
    async fn test_policy_rule_deny_path() {
        let engine = InMemoryPolicyEngine::new();

        let policy = PolicySpec {
            id: PolicyId("no_secrets".to_string()),
            name: "No Secrets".to_string(),
            description: "Deny access to secret files".to_string(),
            rules: vec![PolicyRule::DenyPath {
                patterns: vec![".env".to_string(), "credentials".to_string()],
            }],
        };

        engine.load_policies(vec![policy]).await.unwrap();

        let role = RoleSpec {
            id: RoleId::new("analyst"),
            name: "Analyst".to_string(),
            description: "Data analyst role".to_string(),
            prompt_template: "You are an analyst".to_string(),
            allowed_tools: vec!["repo_read".to_string()],
            budgets: RoleBudgets {
                daily_tokens: None,
                daily_cost_cents: None,
            },
            requires_approval_for: vec![],
        };

        engine.load_roles(vec![role]).await.unwrap();

        let context = PolicyContext {
            role_id: RoleId::new("analyst"),
            tool_id: "repo_read".to_string(),
            tool_tier: 0,
            parameters: serde_json::json!({"path": ".env"}),
            timestamp: Utc::now(),
        };

        let decision = engine.check_tool_call(&context).await.unwrap();
        assert!(matches!(decision, PolicyDecision::Deny { .. }));
    }
}
