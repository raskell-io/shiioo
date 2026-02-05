//! Agent policy engine for governance and authorization.
//!
//! The policy engine evaluates whether an agent can perform actions based on:
//! - Direct agent policies (allow, deny, requires_approval)
//! - Inherited policies (from archetype, team, organization)
//! - Budget limits (tokens, cost, requests, concurrency)
//! - Delegation and escalation rules

use anyhow::{anyhow, Result};
use chrono::{DateTime, Datelike, Timelike, Utc};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::types::{ApprovalBoardId, OrgId, PersonId, TeamId};

use super::{
    Agent, AgentBudgets, AgentId, AgentPolicies, ApprovalRule, ApprovalTrigger, ApproverSpec,
    Archetype, ArchetypeId, BudgetUsageStats, ConcurrencyBudget, CostBudget, DelegationRules,
    EscalationRule, EscalationTarget, PolicyCondition, PolicyInheritance, PolicyRule, PolicyScope,
    RequestBudget, TimeoutAction, TokenBudget,
};

/// Result of policy evaluation.
#[derive(Debug, Clone)]
pub enum PolicyDecision {
    /// Action is allowed.
    Allow,

    /// Action is denied.
    Deny { rule_id: String, reason: String },

    /// Action requires approval before execution.
    RequiresApproval {
        rule_id: String,
        approvers: ApproverSpec,
        timeout: Option<chrono::Duration>,
        timeout_action: TimeoutAction,
    },

    /// Action should be escalated to another party.
    Escalate {
        rule_id: String,
        target: EscalationTarget,
        reason: String,
    },

    /// Action should be delegated to another agent.
    Delegate {
        target_agent: AgentId,
        reason: String,
    },

    /// Budget limit exceeded.
    BudgetExceeded {
        budget_type: String,
        limit: u64,
        current: u64,
    },
}

impl PolicyDecision {
    /// Check if this decision allows the action.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow)
    }

    /// Check if this decision denies the action.
    pub fn is_denied(&self) -> bool {
        matches!(self, Self::Deny { .. } | Self::BudgetExceeded { .. })
    }

    /// Check if this decision requires approval.
    pub fn requires_approval(&self) -> bool {
        matches!(self, Self::RequiresApproval { .. })
    }
}

/// An action an agent wants to perform.
#[derive(Debug, Clone)]
pub struct AgentAction {
    /// Type of action.
    pub action_type: ActionType,

    /// Tool being used (if applicable).
    pub tool_id: Option<String>,

    /// Tool tier (0 = read-only, 1 = write, 2 = dangerous).
    pub tool_tier: Option<u8>,

    /// Resource being accessed (path, URL, etc.).
    pub resource: Option<String>,

    /// Action parameters.
    pub parameters: serde_json::Value,

    /// Estimated tokens for this action.
    pub estimated_tokens: Option<u64>,

    /// Estimated cost in cents.
    pub estimated_cost_cents: Option<u64>,
}

impl AgentAction {
    /// Create a new tool call action.
    pub fn tool_call(tool_id: impl Into<String>, tier: u8) -> Self {
        Self {
            action_type: ActionType::ToolCall,
            tool_id: Some(tool_id.into()),
            tool_tier: Some(tier),
            resource: None,
            parameters: serde_json::Value::Null,
            estimated_tokens: None,
            estimated_cost_cents: None,
        }
    }

    /// Create a new delegation action.
    pub fn delegation(target: AgentId) -> Self {
        Self {
            action_type: ActionType::Delegation,
            tool_id: None,
            tool_tier: None,
            resource: None,
            parameters: serde_json::json!({ "target": target.0 }),
            estimated_tokens: None,
            estimated_cost_cents: None,
        }
    }

    /// Set the resource.
    pub fn with_resource(mut self, resource: impl Into<String>) -> Self {
        self.resource = Some(resource.into());
        self
    }

    /// Set the parameters.
    pub fn with_parameters(mut self, parameters: serde_json::Value) -> Self {
        self.parameters = parameters;
        self
    }

    /// Set estimated tokens.
    pub fn with_estimated_tokens(mut self, tokens: u64) -> Self {
        self.estimated_tokens = Some(tokens);
        self
    }

    /// Set estimated cost.
    pub fn with_estimated_cost(mut self, cents: u64) -> Self {
        self.estimated_cost_cents = Some(cents);
        self
    }
}

/// Type of action being performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionType {
    /// Calling an MCP tool.
    ToolCall,
    /// Delegating to another agent.
    Delegation,
    /// Escalating to a supervisor/board.
    Escalation,
    /// Activating a skill.
    SkillActivation,
    /// Executing a workflow step.
    WorkflowStep,
    /// Making an external request.
    ExternalRequest,
}

/// Context for policy evaluation.
#[derive(Debug, Clone)]
pub struct ActionContext {
    /// Current timestamp.
    pub timestamp: DateTime<Utc>,

    /// Environment (e.g., "production", "staging", "development").
    pub environment: String,

    /// Current budget usage.
    pub budget_usage: BudgetUsageStats,
}

impl Default for ActionContext {
    fn default() -> Self {
        Self {
            timestamp: Utc::now(),
            environment: "development".to_string(),
            budget_usage: BudgetUsageStats::default(),
        }
    }
}

impl ActionContext {
    /// Create a new context with the current timestamp.
    pub fn now() -> Self {
        Self::default()
    }

    /// Set the environment.
    pub fn with_environment(mut self, env: impl Into<String>) -> Self {
        self.environment = env.into();
        self
    }

    /// Set the budget usage.
    pub fn with_budget_usage(mut self, usage: BudgetUsageStats) -> Self {
        self.budget_usage = usage;
        self
    }
}

/// Effective policies after inheritance resolution.
#[derive(Debug, Clone, Default)]
pub struct EffectivePolicies {
    /// Allow rules (from all sources, ordered by priority).
    pub allow_rules: Vec<(PolicyRule, PolicySource)>,

    /// Deny rules (from all sources, ordered by priority).
    pub deny_rules: Vec<(PolicyRule, PolicySource)>,

    /// Approval rules (from all sources).
    pub approval_rules: Vec<(ApprovalRule, PolicySource)>,

    /// Delegation rules (most specific wins).
    pub delegation_rules: DelegationRules,

    /// Escalation rules (from all sources).
    pub escalation_rules: Vec<(EscalationRule, PolicySource)>,

    /// Effective budget limits (most restrictive wins).
    pub budget_limits: AgentBudgets,
}

/// Source of a policy rule (for debugging/auditing).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicySource {
    /// Direct agent policy.
    Agent(AgentId),
    /// Inherited from archetype.
    Archetype(ArchetypeId),
    /// Inherited from team.
    Team(TeamId),
    /// Inherited from organization.
    Organization(OrgId),
    /// From a policy set.
    PolicySet(String),
}

/// Trait for the agent policy engine.
#[async_trait::async_trait]
pub trait AgentPolicyEngine: Send + Sync {
    /// Evaluate if an agent can perform an action.
    async fn evaluate(
        &self,
        agent: &Agent,
        action: &AgentAction,
        context: &ActionContext,
    ) -> Result<PolicyDecision>;

    /// Get the effective policies for an agent (with inheritance resolved).
    async fn get_effective_policies(&self, agent: &Agent) -> Result<EffectivePolicies>;

    /// Check if delegation is allowed from one agent to another.
    async fn can_delegate(
        &self,
        from_agent: &Agent,
        to_agent: &Agent,
        action: &AgentAction,
    ) -> Result<PolicyDecision>;

    /// Get the escalation target for an action (if escalation is required).
    async fn get_escalation_target(
        &self,
        agent: &Agent,
        action: &AgentAction,
    ) -> Result<Option<EscalationTarget>>;

    /// Record budget usage for an agent.
    async fn record_usage(
        &self,
        agent_id: &AgentId,
        tokens: u64,
        cost_cents: u64,
    ) -> Result<()>;

    /// Get current budget usage for an agent.
    async fn get_budget_usage(&self, agent_id: &AgentId) -> Result<BudgetUsageStats>;
}

/// Storage trait for policy-related data.
#[async_trait::async_trait]
pub trait PolicyStorage: Send + Sync {
    /// Get an archetype by ID.
    async fn get_archetype(&self, id: &ArchetypeId) -> Result<Option<Archetype>>;

    /// Get team policies.
    async fn get_team_policies(&self, team_id: &TeamId) -> Result<Option<AgentPolicies>>;

    /// Get organization policies.
    async fn get_org_policies(&self, org_id: &OrgId) -> Result<Option<AgentPolicies>>;

    /// Get a policy set by ID.
    async fn get_policy_set(&self, policy_set_id: &str) -> Result<Option<AgentPolicies>>;
}

/// Default implementation of the policy engine.
pub struct DefaultPolicyEngine<S: PolicyStorage> {
    storage: S,
    budget_usage: Arc<RwLock<HashMap<AgentId, BudgetUsageStats>>>,
}

impl<S: PolicyStorage> DefaultPolicyEngine<S> {
    /// Create a new policy engine.
    pub fn new(storage: S) -> Self {
        Self {
            storage,
            budget_usage: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Resolve all inherited policies for an agent.
    async fn resolve_inheritance(&self, agent: &Agent) -> Result<EffectivePolicies> {
        let mut effective = EffectivePolicies::default();

        // Start with agent's direct policies (highest priority)
        self.add_policies_from(
            &agent.policies,
            PolicySource::Agent(agent.id.clone()),
            &mut effective,
        );

        // Apply budget limits from agent
        effective.budget_limits = agent.budgets.clone();

        // Process inheritance chain
        for inheritance in &agent.policies.inherits_from {
            match inheritance {
                PolicyInheritance::Archetype { archetype_id } => {
                    if let Some(archetype) = self.storage.get_archetype(archetype_id).await? {
                        self.add_policies_from(
                            &archetype.policies,
                            PolicySource::Archetype(archetype_id.clone()),
                            &mut effective,
                        );
                        // Merge budgets (more restrictive wins)
                        effective.budget_limits =
                            Self::merge_budgets(&effective.budget_limits, &archetype.budgets);
                    }
                }
                PolicyInheritance::Team { team_id } => {
                    if let Some(team_policies) = self.storage.get_team_policies(team_id).await? {
                        self.add_policies_from(
                            &team_policies,
                            PolicySource::Team(team_id.clone()),
                            &mut effective,
                        );
                    }
                }
                PolicyInheritance::Organization { org_id } => {
                    if let Some(org_policies) = self.storage.get_org_policies(org_id).await? {
                        self.add_policies_from(
                            &org_policies,
                            PolicySource::Organization(org_id.clone()),
                            &mut effective,
                        );
                    }
                }
                PolicyInheritance::PolicySet { policy_set_id } => {
                    if let Some(policy_set) =
                        self.storage.get_policy_set(policy_set_id).await?
                    {
                        self.add_policies_from(
                            &policy_set,
                            PolicySource::PolicySet(policy_set_id.clone()),
                            &mut effective,
                        );
                    }
                }
            }
        }

        // If archetype is set but not in inherits_from, also inherit from it
        if let Some(archetype_id) = &agent.archetype {
            let already_inherited = agent.policies.inherits_from.iter().any(|i| {
                matches!(i, PolicyInheritance::Archetype { archetype_id: id } if id == archetype_id)
            });

            if !already_inherited {
                if let Some(archetype) = self.storage.get_archetype(archetype_id).await? {
                    self.add_policies_from(
                        &archetype.policies,
                        PolicySource::Archetype(archetype_id.clone()),
                        &mut effective,
                    );
                    effective.budget_limits =
                        Self::merge_budgets(&effective.budget_limits, &archetype.budgets);
                }
            }
        }

        Ok(effective)
    }

    /// Add policies from a source to effective policies.
    fn add_policies_from(
        &self,
        policies: &AgentPolicies,
        source: PolicySource,
        effective: &mut EffectivePolicies,
    ) {
        // Add allow rules
        for rule in &policies.allow {
            effective.allow_rules.push((rule.clone(), source.clone()));
        }

        // Add deny rules
        for rule in &policies.deny {
            effective.deny_rules.push((rule.clone(), source.clone()));
        }

        // Add approval rules
        for rule in &policies.requires_approval {
            effective
                .approval_rules
                .push((rule.clone(), source.clone()));
        }

        // Add escalation rules
        for rule in &policies.delegation.must_escalate {
            effective
                .escalation_rules
                .push((rule.clone(), source.clone()));
        }

        // Use delegation rules from most specific source (agent > archetype > team > org)
        if policies.delegation.can_delegate && !effective.delegation_rules.can_delegate {
            effective.delegation_rules = policies.delegation.clone();
        }
    }

    /// Merge two budget configurations, taking the more restrictive option.
    fn merge_budgets(current: &AgentBudgets, other: &AgentBudgets) -> AgentBudgets {
        AgentBudgets {
            tokens: Self::merge_token_budget(&current.tokens, &other.tokens),
            cost: Self::merge_cost_budget(&current.cost, &other.cost),
            requests: Self::merge_request_budget(&current.requests, &other.requests),
            concurrency: Self::merge_concurrency_budget(&current.concurrency, &other.concurrency),
        }
    }

    fn merge_token_budget(a: &TokenBudget, b: &TokenBudget) -> TokenBudget {
        TokenBudget {
            per_request: Self::min_option(a.per_request, b.per_request),
            per_hour: Self::min_option(a.per_hour, b.per_hour),
            per_day: Self::min_option(a.per_day, b.per_day),
            per_month: Self::min_option(a.per_month, b.per_month),
        }
    }

    fn merge_cost_budget(a: &CostBudget, b: &CostBudget) -> CostBudget {
        CostBudget {
            per_request_cents: Self::min_option(a.per_request_cents, b.per_request_cents),
            per_hour_cents: Self::min_option(a.per_hour_cents, b.per_hour_cents),
            per_day_cents: Self::min_option(a.per_day_cents, b.per_day_cents),
            per_month_cents: Self::min_option(a.per_month_cents, b.per_month_cents),
        }
    }

    fn merge_request_budget(a: &RequestBudget, b: &RequestBudget) -> RequestBudget {
        RequestBudget {
            per_minute: Self::min_option_u32(a.per_minute, b.per_minute),
            per_hour: Self::min_option_u32(a.per_hour, b.per_hour),
            per_day: Self::min_option_u32(a.per_day, b.per_day),
        }
    }

    fn merge_concurrency_budget(a: &ConcurrencyBudget, b: &ConcurrencyBudget) -> ConcurrencyBudget {
        ConcurrencyBudget {
            max_concurrent_steps: Self::min_option_u32(
                a.max_concurrent_steps,
                b.max_concurrent_steps,
            ),
            max_concurrent_tools: Self::min_option_u32(
                a.max_concurrent_tools,
                b.max_concurrent_tools,
            ),
            max_concurrent_workflows: Self::min_option_u32(
                a.max_concurrent_workflows,
                b.max_concurrent_workflows,
            ),
        }
    }

    fn min_option(a: Option<u64>, b: Option<u64>) -> Option<u64> {
        match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    fn min_option_u32(a: Option<u32>, b: Option<u32>) -> Option<u32> {
        match (a, b) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (Some(a), None) => Some(a),
            (None, Some(b)) => Some(b),
            (None, None) => None,
        }
    }

    /// Check if an action matches a policy scope.
    fn matches_scope(&self, action: &AgentAction, scope: &PolicyScope) -> bool {
        match scope {
            PolicyScope::All => true,

            PolicyScope::Tools { tool_ids } => {
                if let Some(tool_id) = &action.tool_id {
                    tool_ids.iter().any(|t| t == tool_id)
                } else {
                    false
                }
            }

            PolicyScope::ToolTier { min_tier } => {
                action.tool_tier.map(|t| t >= *min_tier).unwrap_or(false)
            }

            PolicyScope::Resources { patterns } => {
                if let Some(resource) = &action.resource {
                    patterns.iter().any(|p| Self::matches_pattern(resource, p))
                } else {
                    false
                }
            }

            PolicyScope::Actions { actions } => {
                let action_name = format!("{:?}", action.action_type);
                actions.iter().any(|a| a.eq_ignore_ascii_case(&action_name))
            }
        }
    }

    /// Check if a resource matches a glob-like pattern.
    fn matches_pattern(resource: &str, pattern: &str) -> bool {
        // Simple glob matching: * matches any sequence, ** matches across path separators
        if pattern == "*" || pattern == "**" {
            return true;
        }

        if pattern.starts_with("**/") {
            // Match anywhere in path
            let suffix = &pattern[3..];
            return resource.contains(suffix) || resource.ends_with(suffix);
        }

        if pattern.ends_with("/**") {
            // Match any path under prefix
            let prefix = &pattern[..pattern.len() - 3];
            return resource.starts_with(prefix);
        }

        if pattern.contains('*') {
            // Simple wildcard matching
            let parts: Vec<&str> = pattern.split('*').collect();
            if parts.len() == 2 {
                return resource.starts_with(parts[0]) && resource.ends_with(parts[1]);
            }
        }

        // Exact match
        resource == pattern
    }

    /// Check if a condition is satisfied.
    fn check_condition(&self, condition: &PolicyCondition, context: &ActionContext) -> bool {
        match condition {
            PolicyCondition::TimeWindow {
                start_hour,
                end_hour,
                timezone: _,
                days,
            } => {
                let hour = context.timestamp.hour() as u8;
                let in_time_range = if start_hour <= end_hour {
                    hour >= *start_hour && hour < *end_hour
                } else {
                    // Wraps around midnight
                    hour >= *start_hour || hour < *end_hour
                };

                if !in_time_range {
                    return false;
                }

                if let Some(allowed_days) = days {
                    let day = context.timestamp.weekday().to_string().to_lowercase();
                    if !allowed_days.iter().any(|d| d.to_lowercase() == day) {
                        return false;
                    }
                }

                true
            }

            PolicyCondition::Environment { environments } => environments
                .iter()
                .any(|e| e.eq_ignore_ascii_case(&context.environment)),

            PolicyCondition::ResourcePattern { field, patterns } => {
                // Would need action parameters to check this
                // For now, return true (condition not blocking)
                true
            }

            PolicyCondition::CostThreshold { max_cost_cents } => {
                context.budget_usage.cost_today_cents < *max_cost_cents
            }

            PolicyCondition::TokenThreshold { max_tokens } => {
                context.budget_usage.tokens_today < *max_tokens
            }

            PolicyCondition::Custom { expression: _ } => {
                // Custom expressions would need an expression evaluator
                // For now, return true
                true
            }
        }
    }

    /// Check if an approval trigger matches the action.
    fn matches_trigger(&self, trigger: &ApprovalTrigger, action: &AgentAction) -> bool {
        match trigger {
            ApprovalTrigger::Tools { tool_ids } => {
                if let Some(tool_id) = &action.tool_id {
                    tool_ids.iter().any(|t| t == tool_id)
                } else {
                    false
                }
            }

            ApprovalTrigger::ToolTier { min_tier } => {
                action.tool_tier.map(|t| t >= *min_tier).unwrap_or(false)
            }

            ApprovalTrigger::CostExceeds { cents } => {
                action.estimated_cost_cents.map(|c| c > *cents).unwrap_or(false)
            }

            ApprovalTrigger::TokensExceed { tokens } => {
                action.estimated_tokens.map(|t| t > *tokens).unwrap_or(false)
            }

            ApprovalTrigger::ResourceMatches { patterns } => {
                if let Some(resource) = &action.resource {
                    patterns.iter().any(|p| Self::matches_pattern(resource, p))
                } else {
                    false
                }
            }

            ApprovalTrigger::ExternalActions => {
                matches!(action.action_type, ActionType::ExternalRequest)
            }

            ApprovalTrigger::Custom { expression: _ } => {
                // Would need expression evaluator
                false
            }
        }
    }

    /// Check budget limits.
    fn check_budgets(
        &self,
        budgets: &AgentBudgets,
        usage: &BudgetUsageStats,
        action: &AgentAction,
    ) -> Option<PolicyDecision> {
        // Check token limits
        if let Some(limit) = budgets.tokens.per_day {
            if usage.tokens_today >= limit {
                return Some(PolicyDecision::BudgetExceeded {
                    budget_type: "daily_tokens".to_string(),
                    limit,
                    current: usage.tokens_today,
                });
            }
        }

        if let Some(limit) = budgets.tokens.per_hour {
            if usage.tokens_this_hour >= limit {
                return Some(PolicyDecision::BudgetExceeded {
                    budget_type: "hourly_tokens".to_string(),
                    limit,
                    current: usage.tokens_this_hour,
                });
            }
        }

        // Check cost limits
        if let Some(limit) = budgets.cost.per_day_cents {
            if usage.cost_today_cents >= limit {
                return Some(PolicyDecision::BudgetExceeded {
                    budget_type: "daily_cost".to_string(),
                    limit,
                    current: usage.cost_today_cents,
                });
            }
        }

        if let Some(limit) = budgets.cost.per_hour_cents {
            if usage.cost_this_hour_cents >= limit {
                return Some(PolicyDecision::BudgetExceeded {
                    budget_type: "hourly_cost".to_string(),
                    limit,
                    current: usage.cost_this_hour_cents,
                });
            }
        }

        // Check request limits
        if let Some(limit) = budgets.requests.per_minute {
            if usage.requests_this_minute >= limit {
                return Some(PolicyDecision::BudgetExceeded {
                    budget_type: "requests_per_minute".to_string(),
                    limit: limit as u64,
                    current: usage.requests_this_minute as u64,
                });
            }
        }

        // Check concurrency limits
        if let Some(limit) = budgets.concurrency.max_concurrent_steps {
            if usage.current_concurrent_steps >= limit {
                return Some(PolicyDecision::BudgetExceeded {
                    budget_type: "concurrent_steps".to_string(),
                    limit: limit as u64,
                    current: usage.current_concurrent_steps as u64,
                });
            }
        }

        None
    }
}

#[async_trait::async_trait]
impl<S: PolicyStorage + Send + Sync> AgentPolicyEngine for DefaultPolicyEngine<S> {
    async fn evaluate(
        &self,
        agent: &Agent,
        action: &AgentAction,
        context: &ActionContext,
    ) -> Result<PolicyDecision> {
        // Check if agent is active
        if !agent.is_active() {
            return Ok(PolicyDecision::Deny {
                rule_id: "agent_status".to_string(),
                reason: format!("Agent is not active (status: {})", agent.status),
            });
        }

        // Resolve effective policies
        let effective = self.resolve_inheritance(agent).await?;

        // 1. Check budgets first
        if let Some(budget_decision) =
            self.check_budgets(&effective.budget_limits, &context.budget_usage, action)
        {
            return Ok(budget_decision);
        }

        // 2. Check deny rules (deny takes precedence)
        for (rule, _source) in &effective.deny_rules {
            if self.matches_scope(action, &rule.scope) {
                // Check conditions
                let conditions_met = rule.conditions.iter().all(|c| self.check_condition(c, context));
                if conditions_met {
                    return Ok(PolicyDecision::Deny {
                        rule_id: rule.id.clone(),
                        reason: rule.description.clone(),
                    });
                }
            }
        }

        // 3. Check if action matches any allow rule
        let mut allowed = false;
        for (rule, _source) in &effective.allow_rules {
            if self.matches_scope(action, &rule.scope) {
                let conditions_met = rule.conditions.iter().all(|c| self.check_condition(c, context));
                if conditions_met {
                    allowed = true;
                    break;
                }
            }
        }

        // If no allow rule matches and there are allow rules, deny by default
        if !allowed && !effective.allow_rules.is_empty() {
            // Check if there's a scope that could match
            let could_be_allowed = effective.allow_rules.iter().any(|(rule, _)| {
                matches!(rule.scope, PolicyScope::All)
            });

            if !could_be_allowed {
                return Ok(PolicyDecision::Deny {
                    rule_id: "no_matching_allow_rule".to_string(),
                    reason: "No allow rule matches this action".to_string(),
                });
            }
        }

        // 4. Check escalation rules
        for (rule, _source) in &effective.escalation_rules {
            if self.matches_trigger(&rule.trigger, action) {
                return Ok(PolicyDecision::Escalate {
                    rule_id: format!("escalation_{:?}", rule.trigger),
                    target: rule.escalate_to.clone(),
                    reason: "Action matches escalation trigger".to_string(),
                });
            }
        }

        // 5. Check approval rules
        for (rule, _source) in &effective.approval_rules {
            if self.matches_trigger(&rule.trigger, action) {
                return Ok(PolicyDecision::RequiresApproval {
                    rule_id: rule.id.clone(),
                    approvers: rule.approvers.clone(),
                    timeout: rule.timeout,
                    timeout_action: rule.timeout_action,
                });
            }
        }

        // All checks passed
        Ok(PolicyDecision::Allow)
    }

    async fn get_effective_policies(&self, agent: &Agent) -> Result<EffectivePolicies> {
        self.resolve_inheritance(agent).await
    }

    async fn can_delegate(
        &self,
        from_agent: &Agent,
        to_agent: &Agent,
        action: &AgentAction,
    ) -> Result<PolicyDecision> {
        let effective = self.resolve_inheritance(from_agent).await?;

        // Check if delegation is allowed at all
        if !effective.delegation_rules.can_delegate {
            return Ok(PolicyDecision::Deny {
                rule_id: "delegation_disabled".to_string(),
                reason: "This agent cannot delegate tasks".to_string(),
            });
        }

        // Check if target is in the allowed delegate list
        let target_allowed = effective
            .delegation_rules
            .delegate_to
            .iter()
            .find(|t| t.agent_id == to_agent.id);

        match target_allowed {
            Some(target) => {
                // Check if the action scope is delegatable to this target
                let scope_allowed = target.scopes.is_empty()
                    || target.scopes.iter().any(|s| self.matches_scope(action, s));

                if !scope_allowed {
                    return Ok(PolicyDecision::Deny {
                        rule_id: "delegation_scope".to_string(),
                        reason: format!(
                            "This action cannot be delegated to agent {}",
                            to_agent.id
                        ),
                    });
                }

                if target.requires_approval {
                    return Ok(PolicyDecision::RequiresApproval {
                        rule_id: "delegation_approval".to_string(),
                        approvers: ApproverSpec::Supervisor,
                        timeout: None,
                        timeout_action: TimeoutAction::Deny,
                    });
                }

                Ok(PolicyDecision::Allow)
            }
            None => {
                // Check if target is a peer
                if from_agent.is_peer(&to_agent.id) {
                    // Peers can receive delegated read-only tasks by default
                    if action.tool_tier.map(|t| t == 0).unwrap_or(true) {
                        return Ok(PolicyDecision::Allow);
                    }
                }

                Ok(PolicyDecision::Deny {
                    rule_id: "delegation_target".to_string(),
                    reason: format!(
                        "Agent {} is not in the allowed delegation targets",
                        to_agent.id
                    ),
                })
            }
        }
    }

    async fn get_escalation_target(
        &self,
        agent: &Agent,
        action: &AgentAction,
    ) -> Result<Option<EscalationTarget>> {
        let effective = self.resolve_inheritance(agent).await?;

        for (rule, _source) in &effective.escalation_rules {
            if self.matches_trigger(&rule.trigger, action) {
                return Ok(Some(rule.escalate_to.clone()));
            }
        }

        Ok(None)
    }

    async fn record_usage(
        &self,
        agent_id: &AgentId,
        tokens: u64,
        cost_cents: u64,
    ) -> Result<()> {
        let mut usage_map = self.budget_usage.write().await;
        let usage = usage_map.entry(agent_id.clone()).or_default();

        usage.tokens_this_hour += tokens;
        usage.tokens_today += tokens;
        usage.tokens_this_month += tokens;

        usage.cost_this_hour_cents += cost_cents;
        usage.cost_today_cents += cost_cents;
        usage.cost_this_month_cents += cost_cents;

        usage.requests_this_minute += 1;
        usage.requests_this_hour += 1;
        usage.requests_today += 1;

        Ok(())
    }

    async fn get_budget_usage(&self, agent_id: &AgentId) -> Result<BudgetUsageStats> {
        let usage_map = self.budget_usage.read().await;
        Ok(usage_map.get(agent_id).cloned().unwrap_or_default())
    }
}

/// In-memory policy storage for testing.
#[derive(Default)]
pub struct InMemoryPolicyStorage {
    archetypes: HashMap<ArchetypeId, Archetype>,
    team_policies: HashMap<TeamId, AgentPolicies>,
    org_policies: HashMap<OrgId, AgentPolicies>,
    policy_sets: HashMap<String, AgentPolicies>,
}

impl InMemoryPolicyStorage {
    /// Create new empty storage.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an archetype.
    pub fn add_archetype(&mut self, archetype: Archetype) {
        self.archetypes.insert(archetype.id.clone(), archetype);
    }

    /// Add team policies.
    pub fn add_team_policies(&mut self, team_id: TeamId, policies: AgentPolicies) {
        self.team_policies.insert(team_id, policies);
    }

    /// Add organization policies.
    pub fn add_org_policies(&mut self, org_id: OrgId, policies: AgentPolicies) {
        self.org_policies.insert(org_id, policies);
    }

    /// Add a policy set.
    pub fn add_policy_set(&mut self, id: impl Into<String>, policies: AgentPolicies) {
        self.policy_sets.insert(id.into(), policies);
    }
}

#[async_trait::async_trait]
impl PolicyStorage for InMemoryPolicyStorage {
    async fn get_archetype(&self, id: &ArchetypeId) -> Result<Option<Archetype>> {
        Ok(self.archetypes.get(id).cloned())
    }

    async fn get_team_policies(&self, team_id: &TeamId) -> Result<Option<AgentPolicies>> {
        Ok(self.team_policies.get(team_id).cloned())
    }

    async fn get_org_policies(&self, org_id: &OrgId) -> Result<Option<AgentPolicies>> {
        Ok(self.org_policies.get(org_id).cloned())
    }

    async fn get_policy_set(&self, policy_set_id: &str) -> Result<Option<AgentPolicies>> {
        Ok(self.policy_sets.get(policy_set_id).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{AgentOrganization, AgentSkills};

    fn create_test_agent(id: &str, policies: AgentPolicies) -> Agent {
        Agent::builder(id, format!("Agent {}", id))
            .organization(AgentOrganization::new(TeamId::new("test-team")))
            .policies(policies)
            .build()
    }

    #[test]
    fn test_matches_pattern() {
        // Exact match
        assert!(DefaultPolicyEngine::<InMemoryPolicyStorage>::matches_pattern(
            "/path/to/file",
            "/path/to/file"
        ));

        // ** prefix
        assert!(DefaultPolicyEngine::<InMemoryPolicyStorage>::matches_pattern(
            "/some/path/.env",
            "**/.env"
        ));

        // ** suffix
        assert!(DefaultPolicyEngine::<InMemoryPolicyStorage>::matches_pattern(
            "/production/config/file",
            "/production/**"
        ));

        // Simple wildcard
        assert!(DefaultPolicyEngine::<InMemoryPolicyStorage>::matches_pattern(
            "file.txt",
            "*.txt"
        ));

        // No match
        assert!(!DefaultPolicyEngine::<InMemoryPolicyStorage>::matches_pattern(
            "/other/path",
            "/some/path"
        ));
    }

    #[test]
    fn test_matches_scope_tools() {
        let engine = DefaultPolicyEngine::new(InMemoryPolicyStorage::new());
        let action = AgentAction::tool_call("repo_read", 0);
        let scope = PolicyScope::tools(["repo_read", "repo_write"]);

        assert!(engine.matches_scope(&action, &scope));

        let other_action = AgentAction::tool_call("deploy", 2);
        assert!(!engine.matches_scope(&other_action, &scope));
    }

    #[test]
    fn test_matches_scope_tier() {
        let engine = DefaultPolicyEngine::new(InMemoryPolicyStorage::new());
        let action = AgentAction::tool_call("deploy", 2);
        let scope = PolicyScope::tool_tier(2);

        assert!(engine.matches_scope(&action, &scope));

        let lower_tier = AgentAction::tool_call("read", 0);
        assert!(!engine.matches_scope(&lower_tier, &scope));
    }

    #[tokio::test]
    async fn test_evaluate_allow() {
        let storage = InMemoryPolicyStorage::new();
        let engine = DefaultPolicyEngine::new(storage);

        let policies = AgentPolicies::new().with_allow(PolicyRule::new(
            "allow-read",
            "Allow read operations",
            PolicyScope::tools(["repo_read"]),
        ));

        let agent = create_test_agent("agent-1", policies);
        let action = AgentAction::tool_call("repo_read", 0);
        let context = ActionContext::now();

        let decision = engine.evaluate(&agent, &action, &context).await.unwrap();
        assert!(decision.is_allowed());
    }

    #[tokio::test]
    async fn test_evaluate_deny() {
        let storage = InMemoryPolicyStorage::new();
        let engine = DefaultPolicyEngine::new(storage);

        let policies = AgentPolicies::new()
            .with_allow(PolicyRule::new(
                "allow-all",
                "Allow all",
                PolicyScope::All,
            ))
            .with_deny(PolicyRule::new(
                "deny-secrets",
                "Deny secret files",
                PolicyScope::resources(["**/.env", "**/secrets*"]),
            ));

        let agent = create_test_agent("agent-1", policies);
        let action = AgentAction::tool_call("repo_read", 0).with_resource("/app/.env");
        let context = ActionContext::now();

        let decision = engine.evaluate(&agent, &action, &context).await.unwrap();
        assert!(decision.is_denied());
    }

    #[tokio::test]
    async fn test_evaluate_requires_approval() {
        let storage = InMemoryPolicyStorage::new();
        let engine = DefaultPolicyEngine::new(storage);

        let policies = AgentPolicies::new()
            .with_allow(PolicyRule::new(
                "allow-all",
                "Allow all",
                PolicyScope::All,
            ))
            .with_approval(ApprovalRule::new(
                "approve-deploy",
                ApprovalTrigger::tools(["deploy"]),
                ApproverSpec::supervisor(),
            ));

        let agent = create_test_agent("agent-1", policies);
        let action = AgentAction::tool_call("deploy", 2);
        let context = ActionContext::now();

        let decision = engine.evaluate(&agent, &action, &context).await.unwrap();
        assert!(decision.requires_approval());
    }

    #[tokio::test]
    async fn test_evaluate_budget_exceeded() {
        let storage = InMemoryPolicyStorage::new();
        let engine = DefaultPolicyEngine::new(storage);

        let policies = AgentPolicies::new().with_allow(PolicyRule::new(
            "allow-all",
            "Allow all",
            PolicyScope::All,
        ));

        let agent = Agent::builder("agent-1", "Agent 1")
            .organization(AgentOrganization::new(TeamId::new("test")))
            .policies(policies)
            .budgets(
                AgentBudgets::new()
                    .with_tokens(super::super::TokenBudget::new().with_per_day(1000)),
            )
            .build();

        let action = AgentAction::tool_call("repo_read", 0);
        let context = ActionContext::now().with_budget_usage(BudgetUsageStats {
            tokens_today: 1000,
            ..Default::default()
        });

        let decision = engine.evaluate(&agent, &action, &context).await.unwrap();
        assert!(matches!(decision, PolicyDecision::BudgetExceeded { .. }));
    }

    #[tokio::test]
    async fn test_evaluate_inactive_agent() {
        let storage = InMemoryPolicyStorage::new();
        let engine = DefaultPolicyEngine::new(storage);

        let agent = Agent::builder("agent-1", "Agent 1")
            .organization(AgentOrganization::new(TeamId::new("test")))
            .status(super::super::AgentStatus::Suspended)
            .build();

        let action = AgentAction::tool_call("repo_read", 0);
        let context = ActionContext::now();

        let decision = engine.evaluate(&agent, &action, &context).await.unwrap();
        assert!(decision.is_denied());
    }

    #[tokio::test]
    async fn test_can_delegate() {
        let storage = InMemoryPolicyStorage::new();
        let engine = DefaultPolicyEngine::new(storage);

        let from_policies = AgentPolicies::new().with_delegation(
            DelegationRules::allowing().with_target(
                super::super::DelegationTarget::new(AgentId::new("agent-2"))
                    .with_scope(PolicyScope::tool_tier(0)),
            ),
        );

        let from_agent = create_test_agent("agent-1", from_policies);
        let to_agent = create_test_agent("agent-2", AgentPolicies::new());

        let action = AgentAction::tool_call("repo_read", 0);
        let decision = engine.can_delegate(&from_agent, &to_agent, &action).await.unwrap();
        assert!(decision.is_allowed());
    }

    #[tokio::test]
    async fn test_cannot_delegate_to_unknown() {
        let storage = InMemoryPolicyStorage::new();
        let engine = DefaultPolicyEngine::new(storage);

        let from_policies = AgentPolicies::new().with_delegation(
            DelegationRules::allowing().with_target(
                super::super::DelegationTarget::new(AgentId::new("agent-2")),
            ),
        );

        let from_agent = create_test_agent("agent-1", from_policies);
        let to_agent = create_test_agent("agent-3", AgentPolicies::new()); // Different agent

        let action = AgentAction::tool_call("repo_read", 0);
        let decision = engine.can_delegate(&from_agent, &to_agent, &action).await.unwrap();
        assert!(decision.is_denied());
    }

    #[tokio::test]
    async fn test_record_and_get_usage() {
        let storage = InMemoryPolicyStorage::new();
        let engine = DefaultPolicyEngine::new(storage);

        let agent_id = AgentId::new("agent-1");

        engine.record_usage(&agent_id, 1000, 50).await.unwrap();
        engine.record_usage(&agent_id, 500, 25).await.unwrap();

        let usage = engine.get_budget_usage(&agent_id).await.unwrap();
        assert_eq!(usage.tokens_today, 1500);
        assert_eq!(usage.cost_today_cents, 75);
        assert_eq!(usage.requests_today, 2);
    }

    #[test]
    fn test_merge_budgets() {
        let a = AgentBudgets::new()
            .with_tokens(TokenBudget::new().with_per_day(1000))
            .with_cost(CostBudget::new().with_per_day(500));

        let b = AgentBudgets::new()
            .with_tokens(TokenBudget::new().with_per_day(2000).with_per_hour(100))
            .with_cost(CostBudget::new().with_per_day(300));

        let merged = DefaultPolicyEngine::<InMemoryPolicyStorage>::merge_budgets(&a, &b);

        // Should take the more restrictive (lower) value
        assert_eq!(merged.tokens.per_day, Some(1000));
        assert_eq!(merged.tokens.per_hour, Some(100)); // From b
        assert_eq!(merged.cost.per_day_cents, Some(300)); // From b (more restrictive)
    }
}
