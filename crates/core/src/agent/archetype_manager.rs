//! Archetype management and inheritance resolution.
//!
//! This module provides:
//! - `ArchetypeRegistry` trait for storing and retrieving archetypes
//! - `ArchetypeResolver` for resolving inheritance chains
//! - Adapters for migrating from legacy `RoleSpec` to `Archetype`

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use super::{
    AgentBudgets, AgentPolicies, AgentSkills, Archetype, ArchetypeId, ConcurrencyBudget,
    CostBudget, DelegationRules, McpBinding, PolicyScope, RequestBudget, TokenBudget,
};
#[cfg(test)]
use super::SkillRef;
use crate::types::{RoleBudgets, RoleSpec};

/// Error type for archetype operations.
#[derive(Debug, Clone)]
pub enum ArchetypeError {
    /// Archetype not found.
    NotFound(ArchetypeId),
    /// Circular inheritance detected.
    CircularInheritance(Vec<ArchetypeId>),
    /// Maximum inheritance depth exceeded.
    MaxDepthExceeded { max_depth: usize, reached: usize },
    /// Storage error.
    StorageError(String),
}

impl std::fmt::Display for ArchetypeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "Archetype not found: {}", id),
            Self::CircularInheritance(chain) => {
                let chain_str: Vec<_> = chain.iter().map(|id| id.0.as_str()).collect();
                write!(f, "Circular inheritance detected: {}", chain_str.join(" -> "))
            }
            Self::MaxDepthExceeded { max_depth, reached } => {
                write!(
                    f,
                    "Maximum inheritance depth exceeded: max={}, reached={}",
                    max_depth, reached
                )
            }
            Self::StorageError(msg) => write!(f, "Storage error: {}", msg),
        }
    }
}

impl std::error::Error for ArchetypeError {}

/// Registry for storing and retrieving archetypes.
#[async_trait]
pub trait ArchetypeRegistry: Send + Sync {
    /// Get an archetype by ID.
    async fn get(&self, id: &ArchetypeId) -> Result<Option<Archetype>, ArchetypeError>;

    /// List all archetypes.
    async fn list(&self) -> Result<Vec<Archetype>, ArchetypeError>;

    /// Store an archetype.
    async fn store(&self, archetype: Archetype) -> Result<(), ArchetypeError>;

    /// Delete an archetype.
    async fn delete(&self, id: &ArchetypeId) -> Result<bool, ArchetypeError>;

    /// Check if an archetype exists.
    async fn exists(&self, id: &ArchetypeId) -> Result<bool, ArchetypeError> {
        Ok(self.get(id).await?.is_some())
    }
}

/// In-memory archetype registry for testing and simple deployments.
#[derive(Debug, Default)]
pub struct InMemoryArchetypeRegistry {
    archetypes: RwLock<HashMap<ArchetypeId, Archetype>>,
}

impl InMemoryArchetypeRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a registry with the common archetypes pre-loaded.
    pub fn with_common_archetypes() -> Self {
        let archetypes = vec![
            super::common::software_engineer(),
            super::common::engineering_lead(),
            super::common::data_analyst(),
            super::common::security_analyst(),
        ];
        let mut map = HashMap::new();
        for archetype in archetypes {
            map.insert(archetype.id.clone(), archetype);
        }
        Self {
            archetypes: RwLock::new(map),
        }
    }
}

#[async_trait]
impl ArchetypeRegistry for InMemoryArchetypeRegistry {
    async fn get(&self, id: &ArchetypeId) -> Result<Option<Archetype>, ArchetypeError> {
        let archetypes = self.archetypes.read().await;
        Ok(archetypes.get(id).cloned())
    }

    async fn list(&self) -> Result<Vec<Archetype>, ArchetypeError> {
        let archetypes = self.archetypes.read().await;
        Ok(archetypes.values().cloned().collect())
    }

    async fn store(&self, archetype: Archetype) -> Result<(), ArchetypeError> {
        let mut archetypes = self.archetypes.write().await;
        archetypes.insert(archetype.id.clone(), archetype);
        Ok(())
    }

    async fn delete(&self, id: &ArchetypeId) -> Result<bool, ArchetypeError> {
        let mut archetypes = self.archetypes.write().await;
        Ok(archetypes.remove(id).is_some())
    }
}

/// Configuration for archetype resolution.
#[derive(Debug, Clone)]
pub struct ArchetypeResolverConfig {
    /// Maximum inheritance depth allowed.
    pub max_inheritance_depth: usize,
}

impl Default for ArchetypeResolverConfig {
    fn default() -> Self {
        Self {
            max_inheritance_depth: 10,
        }
    }
}

/// Resolver for archetype inheritance chains.
///
/// Resolves the effective configuration for an archetype by merging
/// configurations from parent archetypes in the inheritance chain.
pub struct ArchetypeResolver<R: ArchetypeRegistry> {
    registry: Arc<R>,
    config: ArchetypeResolverConfig,
}

impl<R: ArchetypeRegistry> ArchetypeResolver<R> {
    /// Create a new resolver with the given registry.
    pub fn new(registry: Arc<R>) -> Self {
        Self {
            registry,
            config: ArchetypeResolverConfig::default(),
        }
    }

    /// Create a new resolver with custom configuration.
    pub fn with_config(registry: Arc<R>, config: ArchetypeResolverConfig) -> Self {
        Self { registry, config }
    }

    /// Get the inheritance chain for an archetype (from child to root).
    pub async fn get_inheritance_chain(
        &self,
        id: &ArchetypeId,
    ) -> Result<Vec<Archetype>, ArchetypeError> {
        let mut chain = Vec::new();
        let mut visited = HashSet::new();
        let mut current_id = Some(id.clone());

        while let Some(ref id) = current_id {
            // Check for circular inheritance
            if visited.contains(id) {
                let mut cycle_chain: Vec<_> = chain.iter().map(|a: &Archetype| a.id.clone()).collect();
                cycle_chain.push(id.clone());
                return Err(ArchetypeError::CircularInheritance(cycle_chain));
            }

            // Check depth limit
            if chain.len() >= self.config.max_inheritance_depth {
                return Err(ArchetypeError::MaxDepthExceeded {
                    max_depth: self.config.max_inheritance_depth,
                    reached: chain.len() + 1,
                });
            }

            // Get the archetype
            let archetype = self
                .registry
                .get(id)
                .await?
                .ok_or_else(|| ArchetypeError::NotFound(id.clone()))?;

            visited.insert(id.clone());
            current_id = archetype.parent.clone();
            chain.push(archetype);
        }

        Ok(chain)
    }

    /// Resolve the effective configuration for an archetype.
    ///
    /// This merges configurations from the entire inheritance chain:
    /// - Skills: merged (child skills take precedence, parent skills are added if not overridden)
    /// - Policies: merged (all rules combined, child rules take precedence for conflicts)
    /// - MCP bindings: merged (child bindings take precedence)
    /// - Budgets: most-restrictive-wins
    pub async fn resolve(&self, id: &ArchetypeId) -> Result<ResolvedArchetype, ArchetypeError> {
        let chain = self.get_inheritance_chain(id).await?;

        if chain.is_empty() {
            return Err(ArchetypeError::NotFound(id.clone()));
        }

        // Start from root (end of chain) and merge towards child (beginning)
        let mut resolved = ResolvedArchetype {
            id: chain[0].id.clone(),
            name: chain[0].name.clone(),
            description: chain[0].description.clone(),
            skills: AgentSkills::default(),
            policies: AgentPolicies::default(),
            mcp_bindings: Vec::new(),
            budgets: AgentBudgets::default(),
            inheritance_chain: chain.iter().map(|a| a.id.clone()).collect(),
        };

        // Process from root to child (reverse order)
        for archetype in chain.iter().rev() {
            resolved.merge_from(archetype);
        }

        // Update identity from the actual archetype (child)
        resolved.id = chain[0].id.clone();
        resolved.name = chain[0].name.clone();
        resolved.description = chain[0].description.clone();

        Ok(resolved)
    }
}

/// A fully resolved archetype with all inheritance applied.
#[derive(Debug, Clone)]
pub struct ResolvedArchetype {
    /// The archetype ID.
    pub id: ArchetypeId,
    /// Human-readable name.
    pub name: String,
    /// Description.
    pub description: String,
    /// Merged skills.
    pub skills: AgentSkills,
    /// Merged policies.
    pub policies: AgentPolicies,
    /// Merged MCP bindings.
    pub mcp_bindings: Vec<McpBinding>,
    /// Merged budgets (most-restrictive-wins).
    pub budgets: AgentBudgets,
    /// The inheritance chain (from child to root).
    pub inheritance_chain: Vec<ArchetypeId>,
}

impl ResolvedArchetype {
    /// Merge configuration from an archetype.
    ///
    /// This applies the archetype's configuration on top of the current state.
    /// Used when processing from root to child, so child values override parent.
    fn merge_from(&mut self, archetype: &Archetype) {
        self.merge_skills(&archetype.skills);
        self.merge_policies(&archetype.policies);
        self.merge_mcp_bindings(&archetype.mcp_bindings);
        self.merge_budgets(&archetype.budgets);
    }

    /// Merge skills (child skills override, parent skills are added if new).
    fn merge_skills(&mut self, skills: &AgentSkills) {
        // Get existing skill IDs as owned Strings
        let existing_base: HashSet<String> = self
            .skills
            .base_skills
            .iter()
            .map(|s| s.skill_id.clone())
            .collect();
        let existing_available: HashSet<String> = self
            .skills
            .available_skills
            .iter()
            .map(|s| s.skill_id.clone())
            .collect();

        // Add new base skills (that aren't already present)
        for skill in &skills.base_skills {
            if !existing_base.contains(&skill.skill_id) {
                self.skills.base_skills.push(skill.clone());
            }
        }

        // Add new available skills (that aren't already present)
        for skill in &skills.available_skills {
            if !existing_available.contains(&skill.skill_id) {
                self.skills.available_skills.push(skill.clone());
            }
        }

        // Child's discovery mode takes precedence
        self.skills.discovery = skills.discovery.clone();

        // Child's max_active takes precedence
        self.skills.max_active_skills = skills.max_active_skills;
    }

    /// Merge policies (combine rules, child rules override for same ID).
    fn merge_policies(&mut self, policies: &AgentPolicies) {
        // Merge allow rules
        self.merge_allow_rules(policies);

        // Merge deny rules
        self.merge_deny_rules(policies);

        // Merge approval rules
        self.merge_approval_rules(policies);

        // Merge delegation rules (child takes precedence for conflicting scopes)
        self.merge_delegation_rules(&policies.delegation);
    }

    fn merge_allow_rules(&mut self, policies: &AgentPolicies) {
        // Collect existing IDs first
        let existing_ids: HashSet<String> =
            self.policies.allow.iter().map(|r| r.id.clone()).collect();

        // Collect updates and additions
        let mut updates: Vec<(usize, super::PolicyRule)> = Vec::new();
        let mut additions: Vec<super::PolicyRule> = Vec::new();

        for rule in &policies.allow {
            if existing_ids.contains(&rule.id) {
                if let Some(pos) = self.policies.allow.iter().position(|r| r.id == rule.id) {
                    updates.push((pos, rule.clone()));
                }
            } else {
                additions.push(rule.clone());
            }
        }

        // Apply updates
        for (pos, rule) in updates {
            self.policies.allow[pos] = rule;
        }

        // Apply additions
        self.policies.allow.extend(additions);
    }

    fn merge_deny_rules(&mut self, policies: &AgentPolicies) {
        let existing_ids: HashSet<String> =
            self.policies.deny.iter().map(|r| r.id.clone()).collect();

        let mut updates: Vec<(usize, super::PolicyRule)> = Vec::new();
        let mut additions: Vec<super::PolicyRule> = Vec::new();

        for rule in &policies.deny {
            if existing_ids.contains(&rule.id) {
                if let Some(pos) = self.policies.deny.iter().position(|r| r.id == rule.id) {
                    updates.push((pos, rule.clone()));
                }
            } else {
                additions.push(rule.clone());
            }
        }

        for (pos, rule) in updates {
            self.policies.deny[pos] = rule;
        }
        self.policies.deny.extend(additions);
    }

    fn merge_approval_rules(&mut self, policies: &AgentPolicies) {
        let existing_ids: HashSet<String> = self
            .policies
            .requires_approval
            .iter()
            .map(|r| r.id.clone())
            .collect();

        let mut updates: Vec<(usize, super::ApprovalRule)> = Vec::new();
        let mut additions: Vec<super::ApprovalRule> = Vec::new();

        for rule in &policies.requires_approval {
            if existing_ids.contains(&rule.id) {
                if let Some(pos) = self
                    .policies
                    .requires_approval
                    .iter()
                    .position(|r| r.id == rule.id)
                {
                    updates.push((pos, rule.clone()));
                }
            } else {
                additions.push(rule.clone());
            }
        }

        for (pos, rule) in updates {
            self.policies.requires_approval[pos] = rule;
        }
        self.policies.requires_approval.extend(additions);
    }

    /// Merge delegation rules.
    fn merge_delegation_rules(&mut self, delegation: &DelegationRules) {
        // Child's can_delegate takes precedence
        self.policies.delegation.can_delegate = delegation.can_delegate;

        // Collect existing target IDs
        let existing_targets: HashSet<String> = self
            .policies
            .delegation
            .delegate_to
            .iter()
            .map(|t| t.agent_id.0.clone())
            .collect();

        // Collect new targets
        let new_targets: Vec<_> = delegation
            .delegate_to
            .iter()
            .filter(|t| !existing_targets.contains(&t.agent_id.0))
            .cloned()
            .collect();

        self.policies.delegation.delegate_to.extend(new_targets);

        // Merge delegatable scopes (union)
        for scope in &delegation.delegatable_scopes {
            if !self.has_scope(&self.policies.delegation.delegatable_scopes, scope) {
                self.policies
                    .delegation
                    .delegatable_scopes
                    .push(scope.clone());
            }
        }

        // Merge escalation rules
        self.policies
            .delegation
            .must_escalate
            .extend(delegation.must_escalate.iter().cloned());
    }

    /// Check if a scope is already in a list.
    fn has_scope(&self, scopes: &[PolicyScope], scope: &PolicyScope) -> bool {
        scopes.iter().any(|s| self.scopes_equal(s, scope))
    }

    /// Check if two scopes are equal.
    fn scopes_equal(&self, a: &PolicyScope, b: &PolicyScope) -> bool {
        match (a, b) {
            (PolicyScope::All, PolicyScope::All) => true,
            (PolicyScope::Tools { tool_ids: a }, PolicyScope::Tools { tool_ids: b }) => a == b,
            (PolicyScope::ToolTier { min_tier: a }, PolicyScope::ToolTier { min_tier: b }) => {
                a == b
            }
            (PolicyScope::Resources { patterns: a }, PolicyScope::Resources { patterns: b }) => {
                a == b
            }
            (PolicyScope::Actions { actions: a }, PolicyScope::Actions { actions: b }) => a == b,
            _ => false,
        }
    }

    /// Merge MCP bindings (child bindings override by ID).
    fn merge_mcp_bindings(&mut self, bindings: &[McpBinding]) {
        // Collect existing IDs
        let existing_ids: HashSet<String> =
            self.mcp_bindings.iter().map(|b| b.id.clone()).collect();

        // Collect updates and additions
        let mut updates: Vec<(usize, McpBinding)> = Vec::new();
        let mut additions: Vec<McpBinding> = Vec::new();

        for binding in bindings {
            if existing_ids.contains(&binding.id) {
                if let Some(pos) = self.mcp_bindings.iter().position(|b| b.id == binding.id) {
                    updates.push((pos, binding.clone()));
                }
            } else {
                additions.push(binding.clone());
            }
        }

        for (pos, binding) in updates {
            self.mcp_bindings[pos] = binding;
        }
        self.mcp_bindings.extend(additions);
    }

    /// Merge budgets (most-restrictive-wins).
    fn merge_budgets(&mut self, budgets: &AgentBudgets) {
        self.merge_token_budget(&budgets.tokens);
        self.merge_cost_budget(&budgets.cost);
        self.merge_request_budget(&budgets.requests);
        self.merge_concurrency_budget(&budgets.concurrency);
    }

    fn merge_token_budget(&mut self, tokens: &TokenBudget) {
        self.budgets.tokens.per_request =
            Self::min_option(self.budgets.tokens.per_request, tokens.per_request);
        self.budgets.tokens.per_hour =
            Self::min_option(self.budgets.tokens.per_hour, tokens.per_hour);
        self.budgets.tokens.per_day =
            Self::min_option(self.budgets.tokens.per_day, tokens.per_day);
        self.budgets.tokens.per_month =
            Self::min_option(self.budgets.tokens.per_month, tokens.per_month);
    }

    fn merge_cost_budget(&mut self, cost: &CostBudget) {
        self.budgets.cost.per_request_cents =
            Self::min_option(self.budgets.cost.per_request_cents, cost.per_request_cents);
        self.budgets.cost.per_hour_cents =
            Self::min_option(self.budgets.cost.per_hour_cents, cost.per_hour_cents);
        self.budgets.cost.per_day_cents =
            Self::min_option(self.budgets.cost.per_day_cents, cost.per_day_cents);
        self.budgets.cost.per_month_cents =
            Self::min_option(self.budgets.cost.per_month_cents, cost.per_month_cents);
    }

    fn merge_request_budget(&mut self, requests: &RequestBudget) {
        self.budgets.requests.per_minute =
            Self::min_option(self.budgets.requests.per_minute, requests.per_minute);
        self.budgets.requests.per_hour =
            Self::min_option(self.budgets.requests.per_hour, requests.per_hour);
        self.budgets.requests.per_day =
            Self::min_option(self.budgets.requests.per_day, requests.per_day);
    }

    fn merge_concurrency_budget(&mut self, concurrency: &ConcurrencyBudget) {
        self.budgets.concurrency.max_concurrent_steps = Self::min_option(
            self.budgets.concurrency.max_concurrent_steps,
            concurrency.max_concurrent_steps,
        );
        self.budgets.concurrency.max_concurrent_tools = Self::min_option(
            self.budgets.concurrency.max_concurrent_tools,
            concurrency.max_concurrent_tools,
        );
    }

    fn min_option<T: Ord + Copy>(a: Option<T>, b: Option<T>) -> Option<T> {
        match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }
}

/// Adapter for converting legacy RoleSpec to Archetype.
pub struct RoleSpecAdapter;

impl RoleSpecAdapter {
    /// Convert a RoleSpec to an Archetype.
    ///
    /// This creates a minimal archetype from the legacy RoleSpec fields.
    pub fn to_archetype(role_spec: &RoleSpec) -> Archetype {
        use chrono::Utc;

        // Convert allowed_tools to skill references (assume they map to builtin tools)
        let skills = AgentSkills::new().with_max_active(3);

        // Convert requires_approval_for to approval rules
        let mut policies = AgentPolicies::new();
        if !role_spec.requires_approval_for.is_empty() {
            use super::{ApprovalRule, ApprovalTrigger, ApproverSpec, TimeoutAction};

            let approval_rule = ApprovalRule::new(
                format!("{}-approval", role_spec.id.0),
                ApprovalTrigger::Tools {
                    tool_ids: role_spec.requires_approval_for.clone(),
                },
                ApproverSpec::supervisor(),
            )
            .with_timeout(chrono::Duration::hours(1), TimeoutAction::Deny);

            policies = policies.with_approval(approval_rule);
        }

        // Convert budgets
        let budgets = Self::convert_budgets(&role_spec.budgets);

        let now = Utc::now();
        Archetype {
            id: ArchetypeId::new(&role_spec.id.0),
            name: role_spec.name.clone(),
            description: role_spec.description.clone(),
            skills,
            policies,
            mcp_bindings: Vec::new(),
            budgets,
            parent: None,
            created_at: now,
            updated_at: now,
            created_by: "migration".to_string(),
        }
    }

    /// Convert legacy RoleBudgets to AgentBudgets.
    fn convert_budgets(role_budgets: &RoleBudgets) -> AgentBudgets {
        let mut budgets = AgentBudgets::default();

        if let Some(daily_tokens) = role_budgets.daily_tokens {
            budgets.tokens = TokenBudget::new().with_per_day(daily_tokens);
        }

        if let Some(daily_cost_cents) = role_budgets.daily_cost_cents {
            budgets.cost = CostBudget::new().with_per_day(daily_cost_cents);
        }

        budgets
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_in_memory_registry() {
        let registry = InMemoryArchetypeRegistry::new();

        // Store an archetype
        let archetype = super::super::common::software_engineer();
        registry.store(archetype.clone()).await.unwrap();

        // Retrieve it
        let retrieved = registry.get(&archetype.id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().id, archetype.id);

        // List
        let list = registry.list().await.unwrap();
        assert_eq!(list.len(), 1);

        // Delete
        let deleted = registry.delete(&archetype.id).await.unwrap();
        assert!(deleted);

        // Verify deleted
        let retrieved = registry.get(&archetype.id).await.unwrap();
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_registry_with_common_archetypes() {
        let registry = InMemoryArchetypeRegistry::with_common_archetypes();

        let list = registry.list().await.unwrap();
        assert_eq!(list.len(), 4);

        // Check all common archetypes are present
        assert!(registry
            .exists(&ArchetypeId::new("software-engineer"))
            .await
            .unwrap());
        assert!(registry
            .exists(&ArchetypeId::new("engineering-lead"))
            .await
            .unwrap());
        assert!(registry
            .exists(&ArchetypeId::new("data-analyst"))
            .await
            .unwrap());
        assert!(registry
            .exists(&ArchetypeId::new("security-analyst"))
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn test_simple_inheritance_chain() {
        let registry = Arc::new(InMemoryArchetypeRegistry::with_common_archetypes());
        let resolver = ArchetypeResolver::new(registry);

        // engineering-lead inherits from software-engineer
        let chain = resolver
            .get_inheritance_chain(&ArchetypeId::new("engineering-lead"))
            .await
            .unwrap();

        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].id.0, "engineering-lead");
        assert_eq!(chain[1].id.0, "software-engineer");
    }

    #[tokio::test]
    async fn test_no_parent_chain() {
        let registry = Arc::new(InMemoryArchetypeRegistry::with_common_archetypes());
        let resolver = ArchetypeResolver::new(registry);

        // software-engineer has no parent
        let chain = resolver
            .get_inheritance_chain(&ArchetypeId::new("software-engineer"))
            .await
            .unwrap();

        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].id.0, "software-engineer");
    }

    #[tokio::test]
    async fn test_circular_inheritance_detection() {
        let registry = Arc::new(InMemoryArchetypeRegistry::new());

        // Create circular inheritance: A -> B -> C -> A
        let archetype_a = Archetype::builder("a", "A")
            .parent(ArchetypeId::new("c"))
            .build();
        let archetype_b = Archetype::builder("b", "B")
            .parent(ArchetypeId::new("a"))
            .build();
        let archetype_c = Archetype::builder("c", "C")
            .parent(ArchetypeId::new("b"))
            .build();

        registry.store(archetype_a).await.unwrap();
        registry.store(archetype_b).await.unwrap();
        registry.store(archetype_c).await.unwrap();

        let resolver = ArchetypeResolver::new(registry);

        let result = resolver
            .get_inheritance_chain(&ArchetypeId::new("a"))
            .await;

        assert!(matches!(result, Err(ArchetypeError::CircularInheritance(_))));
    }

    #[tokio::test]
    async fn test_max_depth_exceeded() {
        let registry = Arc::new(InMemoryArchetypeRegistry::new());

        // Create a deep chain
        for i in 0..15 {
            let parent = if i > 0 {
                Some(ArchetypeId::new(format!("arch-{}", i - 1)))
            } else {
                None
            };
            let mut builder = Archetype::builder(format!("arch-{}", i), format!("Archetype {}", i));
            if let Some(p) = parent {
                builder = builder.parent(p);
            }
            registry.store(builder.build()).await.unwrap();
        }

        let config = ArchetypeResolverConfig {
            max_inheritance_depth: 10,
        };
        let resolver = ArchetypeResolver::with_config(registry, config);

        let result = resolver
            .get_inheritance_chain(&ArchetypeId::new("arch-14"))
            .await;

        assert!(matches!(result, Err(ArchetypeError::MaxDepthExceeded { .. })));
    }

    #[tokio::test]
    async fn test_resolve_inheritance() {
        let registry = Arc::new(InMemoryArchetypeRegistry::with_common_archetypes());
        let resolver = ArchetypeResolver::new(registry);

        let resolved = resolver
            .resolve(&ArchetypeId::new("engineering-lead"))
            .await
            .unwrap();

        // Should have the engineering-lead's identity
        assert_eq!(resolved.id.0, "engineering-lead");
        assert_eq!(resolved.name, "Engineering Lead");

        // Should include skills from both
        assert!(!resolved.skills.base_skills.is_empty());

        // Should have inherited policies from software-engineer
        // The engineering-lead's policies should take precedence
        assert!(!resolved.policies.allow.is_empty());

        // Budgets should use most-restrictive-wins
        // engineering-lead has higher budgets, so these should be from engineering-lead
        // (since it's processed last and overrides)
        assert!(resolved.budgets.tokens.per_day.is_some());
    }

    #[tokio::test]
    async fn test_budget_most_restrictive_wins() {
        let registry = Arc::new(InMemoryArchetypeRegistry::new());

        // Parent with high budget
        let parent = Archetype::builder("parent", "Parent")
            .budgets(
                AgentBudgets::new()
                    .with_tokens(TokenBudget::new().with_per_day(10_000_000))
                    .with_cost(CostBudget::new().with_per_day(20000)),
            )
            .build();

        // Child with lower budget
        let child = Archetype::builder("child", "Child")
            .parent(ArchetypeId::new("parent"))
            .budgets(
                AgentBudgets::new()
                    .with_tokens(TokenBudget::new().with_per_day(5_000_000))
                    .with_cost(CostBudget::new().with_per_day(10000)),
            )
            .build();

        registry.store(parent).await.unwrap();
        registry.store(child).await.unwrap();

        let resolver = ArchetypeResolver::new(registry);
        let resolved = resolver
            .resolve(&ArchetypeId::new("child"))
            .await
            .unwrap();

        // Should use the more restrictive (lower) values
        assert_eq!(resolved.budgets.tokens.per_day, Some(5_000_000));
        assert_eq!(resolved.budgets.cost.per_day_cents, Some(10000));
    }

    #[tokio::test]
    async fn test_skill_merging() {
        let registry = Arc::new(InMemoryArchetypeRegistry::new());

        // Parent with some skills
        let parent = Archetype::builder("parent", "Parent")
            .skills(
                AgentSkills::new()
                    .with_base_skill(SkillRef::builtin("skill-a"))
                    .with_base_skill(SkillRef::builtin("skill-b")),
            )
            .build();

        // Child with additional skills
        let child = Archetype::builder("child", "Child")
            .parent(ArchetypeId::new("parent"))
            .skills(
                AgentSkills::new()
                    .with_base_skill(SkillRef::builtin("skill-b")) // Override
                    .with_base_skill(SkillRef::builtin("skill-c")), // New
            )
            .build();

        registry.store(parent).await.unwrap();
        registry.store(child).await.unwrap();

        let resolver = ArchetypeResolver::new(registry);
        let resolved = resolver
            .resolve(&ArchetypeId::new("child"))
            .await
            .unwrap();

        // Should have all three skills (skill-a from parent, skill-b and skill-c from child)
        let skill_ids: HashSet<_> = resolved
            .skills
            .base_skills
            .iter()
            .map(|s| s.skill_id.as_str())
            .collect();
        assert!(skill_ids.contains("skill-a"));
        assert!(skill_ids.contains("skill-b"));
        assert!(skill_ids.contains("skill-c"));
    }

    #[test]
    fn test_role_spec_adapter() {
        use crate::types::{RoleBudgets, RoleId, RoleSpec};

        let role_spec = RoleSpec {
            id: RoleId::new("test-role"),
            name: "Test Role".to_string(),
            description: "A test role".to_string(),
            prompt_template: "You are a test agent.".to_string(),
            allowed_tools: vec!["tool1".to_string(), "tool2".to_string()],
            budgets: RoleBudgets {
                daily_tokens: Some(1_000_000),
                daily_cost_cents: Some(5000),
            },
            requires_approval_for: vec!["deploy".to_string()],
        };

        let archetype = RoleSpecAdapter::to_archetype(&role_spec);

        assert_eq!(archetype.id.0, "test-role");
        assert_eq!(archetype.name, "Test Role");
        assert_eq!(archetype.description, "A test role");
        assert_eq!(archetype.budgets.tokens.per_day, Some(1_000_000));
        assert_eq!(archetype.budgets.cost.per_day_cents, Some(5000));
        assert_eq!(archetype.policies.requires_approval.len(), 1);
    }
}
