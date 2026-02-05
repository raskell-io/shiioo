//! Agent archetypes (templates).
//!
//! Archetypes are reusable configurations that agents can inherit from.
//! They define base skills, policies, MCP bindings, and budgets that
//! multiple agents can share.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use super::{AgentBudgets, AgentPolicies, AgentSkills, McpBinding};

/// Unique identifier for an archetype.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArchetypeId(pub String);

impl ArchetypeId {
    /// Create a new archetype ID.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl fmt::Display for ArchetypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<&str> for ArchetypeId {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for ArchetypeId {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

/// Agent archetype (template).
///
/// An archetype defines a reusable configuration that agents can inherit from.
/// This includes base skills, default policies, MCP bindings, and budget limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Archetype {
    /// Unique identifier.
    pub id: ArchetypeId,

    /// Human-readable name.
    pub name: String,

    /// Description of what this archetype represents.
    pub description: String,

    /// Base skills for this archetype.
    pub skills: AgentSkills,

    /// Base policies.
    pub policies: AgentPolicies,

    /// Default MCP bindings.
    #[serde(default)]
    pub mcp_bindings: Vec<McpBinding>,

    /// Default budgets.
    pub budgets: AgentBudgets,

    /// Parent archetype (for inheritance chains).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<ArchetypeId>,

    /// When the archetype was created.
    pub created_at: DateTime<Utc>,

    /// When the archetype was last updated.
    pub updated_at: DateTime<Utc>,

    /// Who created this archetype.
    pub created_by: String,
}

impl Archetype {
    /// Create a new archetype builder.
    pub fn builder(id: impl Into<ArchetypeId>, name: impl Into<String>) -> ArchetypeBuilder {
        ArchetypeBuilder::new(id.into(), name.into())
    }
}

/// Builder for creating archetypes.
pub struct ArchetypeBuilder {
    id: ArchetypeId,
    name: String,
    description: String,
    skills: AgentSkills,
    policies: AgentPolicies,
    mcp_bindings: Vec<McpBinding>,
    budgets: AgentBudgets,
    parent: Option<ArchetypeId>,
    created_by: String,
}

impl ArchetypeBuilder {
    /// Create a new archetype builder.
    pub fn new(id: ArchetypeId, name: String) -> Self {
        Self {
            id,
            name,
            description: String::new(),
            skills: AgentSkills::default(),
            policies: AgentPolicies::default(),
            mcp_bindings: Vec::new(),
            budgets: AgentBudgets::default(),
            parent: None,
            created_by: String::from("system"),
        }
    }

    /// Set the description.
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set the skills.
    pub fn skills(mut self, skills: AgentSkills) -> Self {
        self.skills = skills;
        self
    }

    /// Set the policies.
    pub fn policies(mut self, policies: AgentPolicies) -> Self {
        self.policies = policies;
        self
    }

    /// Set the MCP bindings.
    pub fn mcp_bindings(mut self, bindings: Vec<McpBinding>) -> Self {
        self.mcp_bindings = bindings;
        self
    }

    /// Set the budgets.
    pub fn budgets(mut self, budgets: AgentBudgets) -> Self {
        self.budgets = budgets;
        self
    }

    /// Set the parent archetype.
    pub fn parent(mut self, parent: ArchetypeId) -> Self {
        self.parent = Some(parent);
        self
    }

    /// Set who created this archetype.
    pub fn created_by(mut self, created_by: impl Into<String>) -> Self {
        self.created_by = created_by.into();
        self
    }

    /// Build the archetype.
    pub fn build(self) -> Archetype {
        let now = Utc::now();
        Archetype {
            id: self.id,
            name: self.name,
            description: self.description,
            skills: self.skills,
            policies: self.policies,
            mcp_bindings: self.mcp_bindings,
            budgets: self.budgets,
            parent: self.parent,
            created_at: now,
            updated_at: now,
            created_by: self.created_by,
        }
    }
}

/// Common archetypes for enterprise agents.
pub mod common {
    use super::*;
    use crate::agent::{
        ApprovalRule, ApprovalTrigger, ApproverSpec, ConcurrencyBudget, CostBudget,
        DelegationRules, PolicyRule, PolicyScope, RequestBudget, SkillDiscovery, SkillRef,
        TimeoutAction, TokenBudget,
    };

    /// Create a software engineer archetype.
    pub fn software_engineer() -> Archetype {
        Archetype::builder("software-engineer", "Software Engineer")
            .description("A software engineer who can read and write code, run tests, and review PRs")
            .skills(
                AgentSkills::new()
                    .with_base_skill(SkillRef::builtin("code-review"))
                    .with_available_skill(SkillRef::builtin("debugging"))
                    .with_available_skill(SkillRef::builtin("testing"))
                    .with_discovery(SkillDiscovery::TaskMatching)
                    .with_max_active(3),
            )
            .policies(
                AgentPolicies::new()
                    .with_allow(PolicyRule::new(
                        "allow-repo-access",
                        "Can access repositories",
                        PolicyScope::tools(["repo_read", "repo_write", "pr_create"]),
                    ))
                    .with_allow(PolicyRule::new(
                        "allow-search",
                        "Can search code and web",
                        PolicyScope::tools(["context_search", "web_fetch"]),
                    ))
                    .with_deny(PolicyRule::new(
                        "deny-production",
                        "Cannot access production directly",
                        PolicyScope::resources(["**/production/**", "**/prod/**"]),
                    ))
                    .with_deny(PolicyRule::new(
                        "deny-secrets",
                        "Cannot access secret files",
                        PolicyScope::resources(["**/.env", "**/credentials*", "**/secrets*"]),
                    ))
                    .with_approval(
                        ApprovalRule::new(
                            "approve-deploy",
                            ApprovalTrigger::tools(["deploy"]),
                            ApproverSpec::supervisor(),
                        )
                        .with_timeout(chrono::Duration::hours(1), TimeoutAction::Deny),
                    )
                    .with_delegation(
                        DelegationRules::allowing()
                            .with_delegatable_scope(PolicyScope::tool_tier(0)),
                    ),
            )
            .budgets(
                AgentBudgets::new()
                    .with_tokens(TokenBudget::new().with_per_day(5_000_000))
                    .with_cost(CostBudget::new().with_per_day(5000)) // $50/day
                    .with_requests(RequestBudget::new().with_per_minute(30))
                    .with_concurrency(ConcurrencyBudget::new().with_max_steps(3)),
            )
            .build()
    }

    /// Create an engineering lead archetype.
    pub fn engineering_lead() -> Archetype {
        Archetype::builder("engineering-lead", "Engineering Lead")
            .description("Engineering team lead who can approve changes and manage deployments")
            .parent(ArchetypeId::new("software-engineer"))
            .skills(
                AgentSkills::new()
                    .with_base_skill(SkillRef::builtin("code-review"))
                    .with_base_skill(SkillRef::builtin("architecture-review"))
                    .with_base_skill(SkillRef::builtin("incident-response"))
                    .with_discovery(SkillDiscovery::Explicit)
                    .with_max_active(5),
            )
            .policies(
                AgentPolicies::new()
                    .with_allow(PolicyRule::new(
                        "allow-all-repos",
                        "Can access all repositories",
                        PolicyScope::tools(["repo_read", "repo_write", "pr_create", "pr_merge"]),
                    ))
                    .with_allow(PolicyRule::new(
                        "allow-deploy",
                        "Can deploy to staging",
                        PolicyScope::tools(["deploy_staging"]),
                    ))
                    .with_approval(
                        ApprovalRule::new(
                            "approve-prod-deploy",
                            ApprovalTrigger::tools(["deploy_production"]),
                            ApproverSpec::role("vp-engineering"),
                        )
                        .with_timeout(chrono::Duration::hours(2), TimeoutAction::Deny),
                    )
                    .with_delegation(
                        DelegationRules::allowing()
                            .with_delegatable_scope(PolicyScope::All),
                    ),
            )
            .budgets(
                AgentBudgets::new()
                    .with_tokens(TokenBudget::new().with_per_day(10_000_000))
                    .with_cost(CostBudget::new().with_per_day(20000)) // $200/day
                    .with_requests(RequestBudget::new().with_per_minute(60))
                    .with_concurrency(ConcurrencyBudget::new().with_max_steps(10)),
            )
            .build()
    }

    /// Create a data analyst archetype.
    pub fn data_analyst() -> Archetype {
        Archetype::builder("data-analyst", "Data Analyst")
            .description("A data analyst who can query databases and create reports")
            .skills(
                AgentSkills::new()
                    .with_base_skill(SkillRef::builtin("data-analysis"))
                    .with_base_skill(SkillRef::builtin("sql-queries"))
                    .with_available_skill(SkillRef::builtin("visualization"))
                    .with_discovery(SkillDiscovery::TaskMatching)
                    .with_max_active(3),
            )
            .policies(
                AgentPolicies::new()
                    .with_allow(PolicyRule::new(
                        "allow-data-access",
                        "Can access data sources",
                        PolicyScope::tools(["db_query", "data_export", "report_generate"]),
                    ))
                    .with_deny(PolicyRule::new(
                        "deny-pii",
                        "Cannot access PII data directly",
                        PolicyScope::resources(["**/pii/**", "**/personal/**"]),
                    ))
                    .with_approval(
                        ApprovalRule::new(
                            "approve-large-export",
                            ApprovalTrigger::Custom {
                                expression: "row_count > 100000".to_string(),
                            },
                            ApproverSpec::supervisor(),
                        )
                        .with_timeout(chrono::Duration::hours(4), TimeoutAction::Deny),
                    )
                    .with_delegation(DelegationRules::disallowing()),
            )
            .budgets(
                AgentBudgets::new()
                    .with_tokens(TokenBudget::new().with_per_day(2_000_000))
                    .with_cost(CostBudget::new().with_per_day(2000)) // $20/day
                    .with_requests(RequestBudget::new().with_per_minute(20))
                    .with_concurrency(ConcurrencyBudget::new().with_max_steps(2)),
            )
            .build()
    }

    /// Create a security analyst archetype.
    pub fn security_analyst() -> Archetype {
        Archetype::builder("security-analyst", "Security Analyst")
            .description("A security analyst who can audit code and scan for vulnerabilities")
            .skills(
                AgentSkills::new()
                    .with_base_skill(SkillRef::builtin("security-review"))
                    .with_base_skill(SkillRef::builtin("vulnerability-scanning"))
                    .with_available_skill(SkillRef::builtin("incident-response"))
                    .with_discovery(SkillDiscovery::Explicit)
                    .with_max_active(3),
            )
            .policies(
                AgentPolicies::new()
                    .with_allow(PolicyRule::new(
                        "allow-audit",
                        "Can audit all repositories",
                        PolicyScope::tools(["repo_read", "security_scan", "audit_log_read"]),
                    ))
                    .with_deny(PolicyRule::new(
                        "deny-write",
                        "Cannot modify code",
                        PolicyScope::tools(["repo_write", "pr_create"]),
                    ))
                    .with_delegation(DelegationRules::disallowing()),
            )
            .budgets(
                AgentBudgets::new()
                    .with_tokens(TokenBudget::new().with_per_day(3_000_000))
                    .with_cost(CostBudget::new().with_per_day(3000)) // $30/day
                    .with_requests(RequestBudget::new().with_per_minute(30))
                    .with_concurrency(ConcurrencyBudget::new().with_max_steps(5)),
            )
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_archetype_id() {
        let id = ArchetypeId::new("software-engineer");
        assert_eq!(id.0, "software-engineer");
        assert_eq!(id.to_string(), "software-engineer");
    }

    #[test]
    fn test_archetype_builder() {
        let archetype = Archetype::builder("test-archetype", "Test Archetype")
            .description("A test archetype")
            .created_by("admin")
            .build();

        assert_eq!(archetype.id.0, "test-archetype");
        assert_eq!(archetype.name, "Test Archetype");
        assert_eq!(archetype.description, "A test archetype");
        assert_eq!(archetype.created_by, "admin");
    }

    #[test]
    fn test_archetype_with_parent() {
        let archetype = Archetype::builder("child", "Child")
            .parent(ArchetypeId::new("parent"))
            .build();

        assert_eq!(archetype.parent, Some(ArchetypeId::new("parent")));
    }

    #[test]
    fn test_common_software_engineer() {
        let archetype = common::software_engineer();

        assert_eq!(archetype.id.0, "software-engineer");
        assert!(!archetype.skills.base_skills.is_empty());
        assert!(!archetype.policies.allow.is_empty());
        assert!(!archetype.policies.deny.is_empty());
        assert!(archetype.budgets.tokens.per_day.is_some());
    }

    #[test]
    fn test_common_engineering_lead() {
        let archetype = common::engineering_lead();

        assert_eq!(archetype.id.0, "engineering-lead");
        assert_eq!(
            archetype.parent,
            Some(ArchetypeId::new("software-engineer"))
        );
        // Lead should have higher budgets than engineer
        assert!(archetype.budgets.tokens.per_day.unwrap() > 5_000_000);
    }

    #[test]
    fn test_common_data_analyst() {
        let archetype = common::data_analyst();

        assert_eq!(archetype.id.0, "data-analyst");
        // Data analyst should not be able to delegate
        assert!(!archetype.policies.delegation.can_delegate);
    }

    #[test]
    fn test_common_security_analyst() {
        let archetype = common::security_analyst();

        assert_eq!(archetype.id.0, "security-analyst");
        // Security analyst should have deny rules for write operations
        assert!(archetype
            .policies
            .deny
            .iter()
            .any(|r| r.id == "deny-write"));
    }
}
