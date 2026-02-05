//! Agent governance policies.
//!
//! Policies define what an agent can and cannot do, what requires approval,
//! and delegation rules.

use chrono::Duration;
use serde::{Deserialize, Serialize};

use crate::types::{ApprovalBoardId, OrgId, PersonId, TeamId};

use super::{AgentId, ArchetypeId};

/// Governance policies for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPolicies {
    /// Explicit allow rules (whitelist).
    #[serde(default)]
    pub allow: Vec<PolicyRule>,

    /// Explicit deny rules (blacklist, takes precedence).
    #[serde(default)]
    pub deny: Vec<PolicyRule>,

    /// Actions requiring approval.
    #[serde(default)]
    pub requires_approval: Vec<ApprovalRule>,

    /// Delegation rules.
    pub delegation: DelegationRules,

    /// Inherited policies (from archetype, role, org).
    #[serde(default)]
    pub inherits_from: Vec<PolicyInheritance>,
}

impl Default for AgentPolicies {
    fn default() -> Self {
        Self {
            allow: Vec::new(),
            deny: Vec::new(),
            requires_approval: Vec::new(),
            delegation: DelegationRules::default(),
            inherits_from: Vec::new(),
        }
    }
}

impl AgentPolicies {
    /// Create new empty policies.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an allow rule.
    pub fn with_allow(mut self, rule: PolicyRule) -> Self {
        self.allow.push(rule);
        self
    }

    /// Add a deny rule.
    pub fn with_deny(mut self, rule: PolicyRule) -> Self {
        self.deny.push(rule);
        self
    }

    /// Add an approval requirement.
    pub fn with_approval(mut self, rule: ApprovalRule) -> Self {
        self.requires_approval.push(rule);
        self
    }

    /// Set delegation rules.
    pub fn with_delegation(mut self, delegation: DelegationRules) -> Self {
        self.delegation = delegation;
        self
    }

    /// Add policy inheritance.
    pub fn with_inheritance(mut self, inheritance: PolicyInheritance) -> Self {
        self.inherits_from.push(inheritance);
        self
    }
}

/// A policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Rule identifier.
    pub id: String,

    /// Human-readable description.
    pub description: String,

    /// What this rule applies to.
    pub scope: PolicyScope,

    /// Conditions for the rule to apply.
    #[serde(default)]
    pub conditions: Vec<PolicyCondition>,
}

impl PolicyRule {
    /// Create a new policy rule.
    pub fn new(id: impl Into<String>, description: impl Into<String>, scope: PolicyScope) -> Self {
        Self {
            id: id.into(),
            description: description.into(),
            scope,
            conditions: Vec::new(),
        }
    }

    /// Add a condition.
    pub fn with_condition(mut self, condition: PolicyCondition) -> Self {
        self.conditions.push(condition);
        self
    }
}

/// What a policy rule applies to.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PolicyScope {
    /// Applies to specific tools.
    Tools { tool_ids: Vec<String> },

    /// Applies to tool tiers (0 = read-only, 1 = write, 2 = dangerous).
    ToolTier { min_tier: u8 },

    /// Applies to specific resources (path patterns).
    Resources { patterns: Vec<String> },

    /// Applies to specific actions.
    Actions { actions: Vec<String> },

    /// Applies to everything.
    All,
}

impl PolicyScope {
    /// Create a scope for specific tools.
    pub fn tools(tool_ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Tools {
            tool_ids: tool_ids.into_iter().map(Into::into).collect(),
        }
    }

    /// Create a scope for a minimum tool tier.
    pub fn tool_tier(min_tier: u8) -> Self {
        Self::ToolTier { min_tier }
    }

    /// Create a scope for resource patterns.
    pub fn resources(patterns: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Resources {
            patterns: patterns.into_iter().map(Into::into).collect(),
        }
    }

    /// Create a scope for specific actions.
    pub fn actions(actions: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Actions {
            actions: actions.into_iter().map(Into::into).collect(),
        }
    }
}

/// Conditions for a policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PolicyCondition {
    /// Time-based condition.
    TimeWindow {
        start_hour: u8,
        end_hour: u8,
        timezone: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        days: Option<Vec<String>>,
    },

    /// Environment condition.
    Environment { environments: Vec<String> },

    /// Resource pattern match.
    ResourcePattern { field: String, patterns: Vec<String> },

    /// Cost threshold.
    CostThreshold { max_cost_cents: u64 },

    /// Token threshold.
    TokenThreshold { max_tokens: u64 },

    /// Custom condition (evaluated by policy engine).
    Custom { expression: String },
}

impl PolicyCondition {
    /// Create a time window condition.
    pub fn time_window(start_hour: u8, end_hour: u8, timezone: impl Into<String>) -> Self {
        Self::TimeWindow {
            start_hour,
            end_hour,
            timezone: timezone.into(),
            days: None,
        }
    }

    /// Create an environment condition.
    pub fn environment(environments: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Environment {
            environments: environments.into_iter().map(Into::into).collect(),
        }
    }

    /// Create a resource pattern condition.
    pub fn resource_pattern(
        field: impl Into<String>,
        patterns: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::ResourcePattern {
            field: field.into(),
            patterns: patterns.into_iter().map(Into::into).collect(),
        }
    }

    /// Create a cost threshold condition.
    pub fn cost_threshold(max_cost_cents: u64) -> Self {
        Self::CostThreshold { max_cost_cents }
    }

    /// Create a token threshold condition.
    pub fn token_threshold(max_tokens: u64) -> Self {
        Self::TokenThreshold { max_tokens }
    }
}

/// Rule requiring approval before execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRule {
    /// Rule identifier.
    pub id: String,

    /// What triggers this approval requirement.
    pub trigger: ApprovalTrigger,

    /// Who can approve.
    pub approvers: ApproverSpec,

    /// Approval timeout.
    #[serde(
        skip_serializing_if = "Option::is_none",
        serialize_with = "serialize_duration_opt",
        deserialize_with = "deserialize_duration_opt",
        default
    )]
    pub timeout: Option<Duration>,

    /// What happens on timeout.
    pub timeout_action: TimeoutAction,
}

impl ApprovalRule {
    /// Create a new approval rule.
    pub fn new(
        id: impl Into<String>,
        trigger: ApprovalTrigger,
        approvers: ApproverSpec,
    ) -> Self {
        Self {
            id: id.into(),
            trigger,
            approvers,
            timeout: None,
            timeout_action: TimeoutAction::Deny,
        }
    }

    /// Set the timeout.
    pub fn with_timeout(mut self, timeout: Duration, action: TimeoutAction) -> Self {
        self.timeout = Some(timeout);
        self.timeout_action = action;
        self
    }
}

/// What triggers an approval requirement.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApprovalTrigger {
    /// Specific tools.
    Tools { tool_ids: Vec<String> },

    /// Tool tier threshold.
    ToolTier { min_tier: u8 },

    /// Cost threshold.
    CostExceeds { cents: u64 },

    /// Token threshold.
    TokensExceed { tokens: u64 },

    /// Resource patterns.
    ResourceMatches { patterns: Vec<String> },

    /// All external actions.
    ExternalActions,

    /// Custom expression.
    Custom { expression: String },
}

impl ApprovalTrigger {
    /// Create a trigger for specific tools.
    pub fn tools(tool_ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self::Tools {
            tool_ids: tool_ids.into_iter().map(Into::into).collect(),
        }
    }

    /// Create a trigger for a tool tier.
    pub fn tool_tier(min_tier: u8) -> Self {
        Self::ToolTier { min_tier }
    }

    /// Create a trigger for cost exceeding a threshold.
    pub fn cost_exceeds(cents: u64) -> Self {
        Self::CostExceeds { cents }
    }

    /// Create a trigger for tokens exceeding a threshold.
    pub fn tokens_exceed(tokens: u64) -> Self {
        Self::TokensExceed { tokens }
    }
}

/// Who can approve an action.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApproverSpec {
    /// Direct supervisor.
    Supervisor,

    /// Specific agents.
    Agents { agent_ids: Vec<AgentId> },

    /// Specific humans.
    Humans { person_ids: Vec<PersonId> },

    /// Anyone with a specific role.
    Role { role_id: String },

    /// Approval board.
    Board { board_id: ApprovalBoardId },

    /// Any of the specified approvers.
    AnyOf { approvers: Vec<ApproverSpec> },
}

impl ApproverSpec {
    /// Create a supervisor approver.
    pub fn supervisor() -> Self {
        Self::Supervisor
    }

    /// Create an approver for specific agents.
    pub fn agents(agent_ids: impl IntoIterator<Item = AgentId>) -> Self {
        Self::Agents {
            agent_ids: agent_ids.into_iter().collect(),
        }
    }

    /// Create an approver for specific humans.
    pub fn humans(person_ids: impl IntoIterator<Item = PersonId>) -> Self {
        Self::Humans {
            person_ids: person_ids.into_iter().collect(),
        }
    }

    /// Create an approver for anyone with a specific role.
    pub fn role(role_id: impl Into<String>) -> Self {
        Self::Role {
            role_id: role_id.into(),
        }
    }

    /// Create an approver for a board.
    pub fn board(board_id: ApprovalBoardId) -> Self {
        Self::Board { board_id }
    }

    /// Create an approver that accepts any of the specified approvers.
    pub fn any_of(approvers: impl IntoIterator<Item = ApproverSpec>) -> Self {
        Self::AnyOf {
            approvers: approvers.into_iter().collect(),
        }
    }
}

/// What happens when an approval times out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutAction {
    /// Deny the action.
    Deny,
    /// Escalate to next level.
    Escalate,
    /// Auto-approve (only for low-risk actions).
    AutoApprove,
}

impl Default for TimeoutAction {
    fn default() -> Self {
        Self::Deny
    }
}

/// Rules for delegating work to other agents.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationRules {
    /// Can this agent delegate to others?
    pub can_delegate: bool,

    /// Agents this agent can delegate to.
    #[serde(default)]
    pub delegate_to: Vec<DelegationTarget>,

    /// What can be delegated.
    #[serde(default)]
    pub delegatable_scopes: Vec<PolicyScope>,

    /// What must be escalated (not delegated).
    #[serde(default)]
    pub must_escalate: Vec<EscalationRule>,
}

impl Default for DelegationRules {
    fn default() -> Self {
        Self {
            can_delegate: false,
            delegate_to: Vec::new(),
            delegatable_scopes: Vec::new(),
            must_escalate: Vec::new(),
        }
    }
}

impl DelegationRules {
    /// Create new delegation rules that allow delegation.
    pub fn allowing() -> Self {
        Self {
            can_delegate: true,
            ..Default::default()
        }
    }

    /// Create new delegation rules that disallow delegation.
    pub fn disallowing() -> Self {
        Self::default()
    }

    /// Add a delegation target.
    pub fn with_target(mut self, target: DelegationTarget) -> Self {
        self.delegate_to.push(target);
        self
    }

    /// Add a delegatable scope.
    pub fn with_delegatable_scope(mut self, scope: PolicyScope) -> Self {
        self.delegatable_scopes.push(scope);
        self
    }

    /// Add an escalation rule.
    pub fn with_escalation(mut self, rule: EscalationRule) -> Self {
        self.must_escalate.push(rule);
        self
    }
}

/// A delegation target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationTarget {
    /// Target agent.
    pub agent_id: AgentId,

    /// What can be delegated to this agent.
    #[serde(default)]
    pub scopes: Vec<PolicyScope>,

    /// Requires approval before delegation.
    #[serde(default)]
    pub requires_approval: bool,
}

impl DelegationTarget {
    /// Create a new delegation target.
    pub fn new(agent_id: AgentId) -> Self {
        Self {
            agent_id,
            scopes: Vec::new(),
            requires_approval: false,
        }
    }

    /// Add a scope.
    pub fn with_scope(mut self, scope: PolicyScope) -> Self {
        self.scopes.push(scope);
        self
    }

    /// Require approval for delegation.
    pub fn with_approval_required(mut self) -> Self {
        self.requires_approval = true;
        self
    }
}

/// Rule for mandatory escalation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationRule {
    /// What triggers escalation.
    pub trigger: ApprovalTrigger,

    /// Who to escalate to.
    pub escalate_to: EscalationTarget,
}

impl EscalationRule {
    /// Create a new escalation rule.
    pub fn new(trigger: ApprovalTrigger, escalate_to: EscalationTarget) -> Self {
        Self {
            trigger,
            escalate_to,
        }
    }
}

/// Target for escalation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EscalationTarget {
    /// Escalate to supervisor.
    Supervisor,
    /// Escalate to a specific agent.
    Agent { agent_id: AgentId },
    /// Escalate to a specific human.
    Human { person_id: PersonId },
    /// Escalate to an approval board.
    Board { board_id: ApprovalBoardId },
}

impl EscalationTarget {
    /// Create a supervisor escalation target.
    pub fn supervisor() -> Self {
        Self::Supervisor
    }

    /// Create an agent escalation target.
    pub fn agent(agent_id: AgentId) -> Self {
        Self::Agent { agent_id }
    }

    /// Create a human escalation target.
    pub fn human(person_id: PersonId) -> Self {
        Self::Human { person_id }
    }

    /// Create a board escalation target.
    pub fn board(board_id: ApprovalBoardId) -> Self {
        Self::Board { board_id }
    }
}

/// Policy inheritance source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PolicyInheritance {
    /// Inherit from an archetype.
    Archetype { archetype_id: ArchetypeId },

    /// Inherit from organization-wide policies.
    Organization { org_id: OrgId },

    /// Inherit from team policies.
    Team { team_id: TeamId },

    /// Inherit from a policy set.
    PolicySet { policy_set_id: String },
}

impl PolicyInheritance {
    /// Create inheritance from an archetype.
    pub fn archetype(archetype_id: ArchetypeId) -> Self {
        Self::Archetype { archetype_id }
    }

    /// Create inheritance from an organization.
    pub fn organization(org_id: OrgId) -> Self {
        Self::Organization { org_id }
    }

    /// Create inheritance from a team.
    pub fn team(team_id: TeamId) -> Self {
        Self::Team { team_id }
    }

    /// Create inheritance from a policy set.
    pub fn policy_set(policy_set_id: impl Into<String>) -> Self {
        Self::PolicySet {
            policy_set_id: policy_set_id.into(),
        }
    }
}

// Helper functions for serializing Duration
fn serialize_duration_opt<S>(duration: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    match duration {
        Some(d) => serializer.serialize_str(&format!("{}s", d.num_seconds())),
        None => serializer.serialize_none(),
    }
}

fn deserialize_duration_opt<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        Some(s) => {
            // Parse formats like "1h", "30m", "3600s"
            let s = s.trim();
            if let Some(hours) = s.strip_suffix('h') {
                let hours: i64 = hours.parse().map_err(serde::de::Error::custom)?;
                Ok(Some(Duration::hours(hours)))
            } else if let Some(minutes) = s.strip_suffix('m') {
                let minutes: i64 = minutes.parse().map_err(serde::de::Error::custom)?;
                Ok(Some(Duration::minutes(minutes)))
            } else if let Some(seconds) = s.strip_suffix('s') {
                let seconds: i64 = seconds.parse().map_err(serde::de::Error::custom)?;
                Ok(Some(Duration::seconds(seconds)))
            } else {
                // Try to parse as seconds
                let seconds: i64 = s.parse().map_err(serde::de::Error::custom)?;
                Ok(Some(Duration::seconds(seconds)))
            }
        }
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_rule_creation() {
        let rule = PolicyRule::new(
            "allow-repo-access",
            "Can access company repositories",
            PolicyScope::tools(["repo_read", "repo_write"]),
        );

        assert_eq!(rule.id, "allow-repo-access");
        assert!(rule.conditions.is_empty());
    }

    #[test]
    fn test_policy_scope_tools() {
        let scope = PolicyScope::tools(["tool1", "tool2"]);
        assert!(matches!(scope, PolicyScope::Tools { tool_ids } if tool_ids.len() == 2));
    }

    #[test]
    fn test_approval_rule() {
        let rule = ApprovalRule::new(
            "approve-deploy",
            ApprovalTrigger::tools(["deploy"]),
            ApproverSpec::supervisor(),
        )
        .with_timeout(Duration::hours(1), TimeoutAction::Deny);

        assert_eq!(rule.id, "approve-deploy");
        assert!(rule.timeout.is_some());
        assert_eq!(rule.timeout_action, TimeoutAction::Deny);
    }

    #[test]
    fn test_delegation_rules() {
        let rules = DelegationRules::allowing()
            .with_target(
                DelegationTarget::new(AgentId::new("eng-charlie"))
                    .with_scope(PolicyScope::tool_tier(0)),
            )
            .with_escalation(EscalationRule::new(
                ApprovalTrigger::cost_exceeds(1000),
                EscalationTarget::supervisor(),
            ));

        assert!(rules.can_delegate);
        assert_eq!(rules.delegate_to.len(), 1);
        assert_eq!(rules.must_escalate.len(), 1);
    }

    #[test]
    fn test_agent_policies_builder() {
        let policies = AgentPolicies::new()
            .with_allow(PolicyRule::new(
                "allow-read",
                "Allow read operations",
                PolicyScope::tool_tier(0),
            ))
            .with_deny(PolicyRule::new(
                "deny-secrets",
                "Deny access to secrets",
                PolicyScope::resources(["**/.env", "**/secrets*"]),
            ))
            .with_approval(ApprovalRule::new(
                "approve-write",
                ApprovalTrigger::tool_tier(1),
                ApproverSpec::supervisor(),
            ));

        assert_eq!(policies.allow.len(), 1);
        assert_eq!(policies.deny.len(), 1);
        assert_eq!(policies.requires_approval.len(), 1);
    }

    #[test]
    fn test_approver_spec_any_of() {
        let approvers = ApproverSpec::any_of([
            ApproverSpec::supervisor(),
            ApproverSpec::role("tech-lead"),
        ]);

        assert!(matches!(approvers, ApproverSpec::AnyOf { approvers } if approvers.len() == 2));
    }
}
