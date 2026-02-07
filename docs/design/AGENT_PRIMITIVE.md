# Agent Primitive Design

> **Note:** Internal Rust types (`Agent`, `AgentId`, `Archetype`, etc.) retain their current names. The company metaphor language (employee, job title, etc.) is used in user-facing surfaces only.

> The core abstraction for employees in the virtual company.

## Overview

An **Agent** is the fundamental unit of work in Shiioo — a virtual employee with defined skills, governed by policies, and connected to external systems via MCP tools. This design unifies several existing concepts (Role, Person, Policy) into a coherent primitive based on the [Agent Skills](https://agentskills.io) open format.

```
┌─────────────────────────────────────────────────────────────────┐
│                            AGENT                                │
├─────────────────────────────────────────────────────────────────┤
│  Identity        │  who this agent is                           │
│  Skills          │  what this agent knows (Agent Skills format) │
│  Policies        │  what this agent can/cannot do               │
│  MCP Bindings    │  what systems this agent can access          │
│  Secrets         │  credentials this agent can use              │
└─────────────────────────────────────────────────────────────────┘
```

## Design Goals

1. **Skills as the primitive** — Agent capabilities defined via portable SKILL.md files
2. **Policy-first governance** — Every action evaluated against explicit rules
3. **Explicit delegation** — Clear escalation paths for approval and handoff
4. **Credential isolation** — Each agent has scoped access to secrets/tools
5. **Progressive disclosure** — Skills loaded on-demand to minimize context usage
6. **Composable** — Agents can inherit from archetypes, share skill libraries

---

## Core Types

### AgentId

```rust
/// Unique identifier for an agent
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl AgentId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}
```

### Agent

```rust
/// A virtual employee in the enterprise
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    /// Unique identifier
    pub id: AgentId,

    /// Human-readable name
    pub name: String,

    /// What this agent does (used for routing/discovery)
    pub description: String,

    /// Agent archetype (optional inheritance)
    pub archetype: Option<ArchetypeId>,

    /// Organizational placement
    pub organization: AgentOrganization,

    /// Skills this agent has (Agent Skills format)
    pub skills: AgentSkills,

    /// Governance policies
    pub policies: AgentPolicies,

    /// MCP tool bindings
    pub mcp_bindings: Vec<McpBinding>,

    /// Secret access grants
    pub secret_grants: Vec<SecretGrant>,

    /// Resource budgets
    pub budgets: AgentBudgets,

    /// Agent status
    pub status: AgentStatus,

    /// Metadata
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: String,
}
```

### AgentOrganization

```rust
/// Where this agent sits in the org structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentOrganization {
    /// Team membership
    pub team: TeamId,

    /// Direct supervisor (for escalation)
    pub reports_to: Option<AgentId>,

    /// Agents this agent supervises
    pub supervises: Vec<AgentId>,

    /// Peer agents (can delegate to)
    pub peers: Vec<AgentId>,
}
```

### AgentStatus

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Agent is active and can execute tasks
    Active,
    /// Agent is paused (won't be assigned new work)
    Paused,
    /// Agent is suspended (policy violation, budget exceeded)
    Suspended,
    /// Agent is archived (soft delete)
    Archived,
}
```

---

## Skills Layer

Skills define **what an agent knows** and **how to do specific tasks**. We adopt the Agent Skills format directly.

### AgentSkills

```rust
/// Skills configuration for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkills {
    /// Base skills always loaded (core competencies)
    pub base_skills: Vec<SkillRef>,

    /// Skills available for dynamic activation
    pub available_skills: Vec<SkillRef>,

    /// Skill discovery mode
    pub discovery: SkillDiscovery,

    /// Maximum skills that can be active simultaneously
    pub max_active_skills: usize,
}

/// Reference to a skill
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillRef {
    /// Skill identifier (matches SKILL.md name field)
    pub skill_id: String,

    /// Source location
    pub source: SkillSource,

    /// Override allowed-tools from skill
    pub allowed_tools_override: Option<Vec<String>>,
}

/// Where to load a skill from
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SkillSource {
    /// Built-in skill (bundled with Shiioo)
    Builtin { name: String },

    /// Skill from a local directory
    Local { path: PathBuf },

    /// Skill from a Git repository
    Git {
        repo: String,
        path: Option<String>,
        ref_: Option<String>,  // branch/tag/commit
    },

    /// Skill from a registry
    Registry {
        registry: String,  // e.g., "skills.shiioo.io"
        name: String,
        version: Option<String>,
    },
}

/// How skills are discovered and activated
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillDiscovery {
    /// Only use explicitly assigned skills
    Explicit,

    /// Match skills by task description
    TaskMatching,

    /// Agent can request skills from a library
    OnDemand { library: Vec<SkillRef> },
}
```

### Skill (parsed from SKILL.md)

```rust
/// A parsed skill definition (from SKILL.md)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    /// Skill identifier (from frontmatter)
    pub name: String,

    /// When to use this skill (from frontmatter)
    pub description: String,

    /// License information
    pub license: Option<String>,

    /// Environment requirements
    pub compatibility: Option<String>,

    /// Pre-approved tools
    pub allowed_tools: Vec<String>,

    /// Custom metadata
    pub metadata: HashMap<String, String>,

    /// Full instructions (markdown body)
    pub instructions: String,

    /// Available scripts
    pub scripts: Vec<SkillScript>,

    /// Reference documents
    pub references: Vec<SkillReference>,

    /// Asset files
    pub assets: Vec<SkillAsset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillScript {
    pub path: String,
    pub language: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillReference {
    pub path: String,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillAsset {
    pub path: String,
    pub content_type: Option<String>,
}
```

### Skill Loading (Progressive Disclosure)

```rust
/// Skill loading stages
pub enum SkillLoadState {
    /// Only metadata loaded (name, description)
    Metadata(SkillMetadata),

    /// Full instructions loaded
    Full(Skill),
}

#[derive(Debug, Clone)]
pub struct SkillMetadata {
    pub name: String,
    pub description: String,
    pub allowed_tools: Vec<String>,
}

/// Skill manager for an agent
#[async_trait]
pub trait SkillManager: Send + Sync {
    /// Load skill metadata for discovery
    async fn load_metadata(&self, skill_ref: &SkillRef) -> Result<SkillMetadata>;

    /// Fully load a skill (instructions + resources)
    async fn load_full(&self, skill_ref: &SkillRef) -> Result<Skill>;

    /// Get currently active skills
    fn active_skills(&self) -> &[Skill];

    /// Activate a skill (load into context)
    async fn activate(&mut self, skill_id: &str) -> Result<()>;

    /// Deactivate a skill (free context)
    fn deactivate(&mut self, skill_id: &str);

    /// Match task to available skills
    async fn match_skills(&self, task_description: &str) -> Vec<SkillMetadata>;
}
```

---

## Policy Layer

Policies define **what an agent can and cannot do**, **what requires approval**, and **delegation rules**.

### AgentPolicies

```rust
/// Governance policies for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentPolicies {
    /// Explicit allow rules (whitelist)
    pub allow: Vec<PolicyRule>,

    /// Explicit deny rules (blacklist, takes precedence)
    pub deny: Vec<PolicyRule>,

    /// Actions requiring approval
    pub requires_approval: Vec<ApprovalRule>,

    /// Delegation rules
    pub delegation: DelegationRules,

    /// Inherited policies (from archetype, role, org)
    pub inherits_from: Vec<PolicyInheritance>,
}

/// A policy rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Rule identifier
    pub id: String,

    /// Human-readable description
    pub description: String,

    /// What this rule applies to
    pub scope: PolicyScope,

    /// Conditions for the rule to apply
    pub conditions: Vec<PolicyCondition>,
}

/// What a policy rule applies to
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PolicyScope {
    /// Applies to specific tools
    Tools { tool_ids: Vec<String> },

    /// Applies to tool tiers
    ToolTier { min_tier: u8 },

    /// Applies to specific resources
    Resources { patterns: Vec<String> },

    /// Applies to specific actions
    Actions { actions: Vec<String> },

    /// Applies to everything
    All,
}

/// Conditions for a policy rule
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PolicyCondition {
    /// Time-based condition
    TimeWindow {
        start_hour: u8,
        end_hour: u8,
        timezone: String,
        days: Option<Vec<String>>,  // e.g., ["monday", "friday"]
    },

    /// Environment condition
    Environment { environments: Vec<String> },

    /// Resource pattern match
    ResourcePattern {
        field: String,  // e.g., "path", "url", "domain"
        patterns: Vec<String>,
    },

    /// Cost threshold
    CostThreshold { max_cost_cents: u64 },

    /// Token threshold
    TokenThreshold { max_tokens: u64 },

    /// Custom condition (evaluated by policy engine)
    Custom { expression: String },
}
```

### ApprovalRule

```rust
/// Rule requiring approval before execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRule {
    /// Rule identifier
    pub id: String,

    /// What triggers this approval requirement
    pub trigger: ApprovalTrigger,

    /// Who can approve
    pub approvers: ApproverSpec,

    /// Approval timeout
    pub timeout: Option<Duration>,

    /// What happens on timeout
    pub timeout_action: TimeoutAction,
}

/// What triggers an approval requirement
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApprovalTrigger {
    /// Specific tools
    Tools { tool_ids: Vec<String> },

    /// Tool tier threshold
    ToolTier { min_tier: u8 },

    /// Cost threshold
    CostExceeds { cents: u64 },

    /// Token threshold
    TokensExceed { tokens: u64 },

    /// Resource patterns
    ResourceMatches { patterns: Vec<String> },

    /// All external actions
    ExternalActions,

    /// Custom expression
    Custom { expression: String },
}

/// Who can approve an action
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ApproverSpec {
    /// Direct supervisor
    Supervisor,

    /// Specific agents
    Agents { agent_ids: Vec<AgentId> },

    /// Specific humans
    Humans { person_ids: Vec<PersonId> },

    /// Anyone with a specific role
    Role { role_id: String },

    /// Approval board
    Board { board_id: ApprovalBoardId },

    /// Any of the specified approvers
    AnyOf { approvers: Vec<ApproverSpec> },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeoutAction {
    Deny,
    Escalate,
    AutoApprove,  // Only for low-risk actions
}
```

### DelegationRules

```rust
/// Rules for delegating work to other agents
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationRules {
    /// Can this agent delegate to others?
    pub can_delegate: bool,

    /// Agents this agent can delegate to
    pub delegate_to: Vec<DelegationTarget>,

    /// What can be delegated
    pub delegatable_scopes: Vec<PolicyScope>,

    /// What must be escalated (not delegated)
    pub must_escalate: Vec<EscalationRule>,
}

/// A delegation target
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationTarget {
    /// Target agent
    pub agent_id: AgentId,

    /// What can be delegated to this agent
    pub scopes: Vec<PolicyScope>,

    /// Requires approval before delegation
    pub requires_approval: bool,
}

/// Rule for mandatory escalation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationRule {
    /// What triggers escalation
    pub trigger: ApprovalTrigger,

    /// Who to escalate to
    pub escalate_to: EscalationTarget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EscalationTarget {
    Supervisor,
    Agent { agent_id: AgentId },
    Human { person_id: PersonId },
    Board { board_id: ApprovalBoardId },
}
```

### PolicyInheritance

```rust
/// Policy inheritance source
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PolicyInheritance {
    /// Inherit from an archetype
    Archetype { archetype_id: ArchetypeId },

    /// Inherit from organization-wide policies
    Organization { org_id: OrgId },

    /// Inherit from team policies
    Team { team_id: TeamId },

    /// Inherit from a policy set
    PolicySet { policy_set_id: String },
}
```

---

## MCP Bindings

MCP bindings define **what external systems an agent can interact with** and **with what credentials**.

### McpBinding

```rust
/// Binding between an agent and an MCP tool server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpBinding {
    /// Binding identifier
    pub id: String,

    /// MCP server reference
    pub server: McpServerRef,

    /// Tools available from this server
    pub tools: McpToolAccess,

    /// Credential reference for this binding
    pub credentials: Option<CredentialRef>,

    /// Tool tier override (for policy evaluation)
    pub tier_override: Option<u8>,

    /// Additional policy constraints for this binding
    pub policy_constraints: Vec<PolicyRule>,
}

/// Reference to an MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpServerRef {
    /// Built-in Shiioo MCP server
    Builtin,

    /// External MCP server (stdio)
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },

    /// External MCP server (HTTP/SSE)
    Http {
        url: String,
        headers: HashMap<String, String>,
    },

    /// Reference by server ID (configured elsewhere)
    Reference { server_id: String },
}

/// Tool access configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum McpToolAccess {
    /// All tools from the server
    All,

    /// Specific tools only
    Specific { tool_ids: Vec<String> },

    /// All except specific tools
    Except { excluded_tool_ids: Vec<String> },
}

/// Reference to credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialRef {
    /// Reference to a secret by ID
    Secret { secret_id: String },

    /// Reference to an environment variable
    EnvVar { name: String },

    /// Inline value (not recommended for sensitive data)
    Inline { value: String },

    /// Derived from agent's secret grants
    FromGrants { grant_id: String },
}
```

---

## Secret Grants

Secret grants define **what credentials an agent has access to**.

### SecretGrant

```rust
/// Grant giving an agent access to a secret
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretGrant {
    /// Grant identifier
    pub id: String,

    /// Secret being granted
    pub secret_id: String,

    /// What the agent can do with this secret
    pub permissions: SecretPermissions,

    /// Conditions for the grant
    pub conditions: Vec<SecretCondition>,

    /// Expiration
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretPermissions {
    /// Can read the secret value
    pub read: bool,

    /// Can use the secret (without seeing the value)
    pub use_: bool,

    /// Can rotate the secret
    pub rotate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SecretCondition {
    /// Only for specific tools
    ForTools { tool_ids: Vec<String> },

    /// Only for specific domains
    ForDomains { domains: Vec<String> },

    /// Only during specific time windows
    TimeWindow { start_hour: u8, end_hour: u8, timezone: String },
}
```

---

## Budgets

Budgets define **resource limits** for an agent.

### AgentBudgets

```rust
/// Resource budgets for an agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBudgets {
    /// Token limits
    pub tokens: TokenBudget,

    /// Cost limits
    pub cost: CostBudget,

    /// Request limits
    pub requests: RequestBudget,

    /// Concurrent execution limits
    pub concurrency: ConcurrencyBudget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBudget {
    pub per_request: Option<u64>,
    pub per_hour: Option<u64>,
    pub per_day: Option<u64>,
    pub per_month: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostBudget {
    pub per_request_cents: Option<u64>,
    pub per_hour_cents: Option<u64>,
    pub per_day_cents: Option<u64>,
    pub per_month_cents: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestBudget {
    pub per_minute: Option<u32>,
    pub per_hour: Option<u32>,
    pub per_day: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConcurrencyBudget {
    /// Max concurrent workflow steps
    pub max_concurrent_steps: Option<u32>,

    /// Max concurrent tool calls
    pub max_concurrent_tools: Option<u32>,
}
```

---

## Archetypes

Archetypes are **templates for agents** — reusable configurations that agents can inherit from.

### Archetype

```rust
/// Unique identifier for an archetype
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArchetypeId(pub String);

/// Agent archetype (template)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Archetype {
    pub id: ArchetypeId,
    pub name: String,
    pub description: String,

    /// Base skills for this archetype
    pub skills: AgentSkills,

    /// Base policies
    pub policies: AgentPolicies,

    /// Default MCP bindings
    pub mcp_bindings: Vec<McpBinding>,

    /// Default budgets
    pub budgets: AgentBudgets,

    /// Metadata
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

---

## Policy Evaluation

### PolicyEngine (Extended)

```rust
/// Extended policy engine for agent governance
#[async_trait]
pub trait AgentPolicyEngine: Send + Sync {
    /// Evaluate if an agent can perform an action
    async fn evaluate(
        &self,
        agent: &Agent,
        action: &AgentAction,
        context: &ActionContext,
    ) -> Result<PolicyDecision>;

    /// Get the full policy set for an agent (including inherited)
    async fn get_effective_policies(&self, agent: &Agent) -> Result<EffectivePolicies>;

    /// Check if delegation is allowed
    async fn can_delegate(
        &self,
        from_agent: &Agent,
        to_agent: &Agent,
        action: &AgentAction,
    ) -> Result<PolicyDecision>;

    /// Get escalation target for an action
    async fn get_escalation_target(
        &self,
        agent: &Agent,
        action: &AgentAction,
    ) -> Result<Option<EscalationTarget>>;
}

/// An action an agent wants to perform
#[derive(Debug, Clone)]
pub struct AgentAction {
    pub action_type: ActionType,
    pub tool_id: Option<String>,
    pub tool_tier: Option<u8>,
    pub resource: Option<String>,
    pub parameters: serde_json::Value,
    pub estimated_tokens: Option<u64>,
    pub estimated_cost_cents: Option<u64>,
}

#[derive(Debug, Clone)]
pub enum ActionType {
    ToolCall,
    Delegation,
    Escalation,
    SkillActivation,
    WorkflowStep,
    ExternalRequest,
}

/// Context for policy evaluation
#[derive(Debug, Clone)]
pub struct ActionContext {
    pub timestamp: DateTime<Utc>,
    pub environment: String,
    pub run_id: Option<RunId>,
    pub step_id: Option<StepId>,
    pub budget_usage: BudgetUsage,
}

/// Effective policies after inheritance resolution
#[derive(Debug, Clone)]
pub struct EffectivePolicies {
    pub allow_rules: Vec<PolicyRule>,
    pub deny_rules: Vec<PolicyRule>,
    pub approval_rules: Vec<ApprovalRule>,
    pub delegation_rules: DelegationRules,
    pub budget_limits: AgentBudgets,
}

/// Policy evaluation result
#[derive(Debug, Clone)]
pub enum PolicyDecision {
    /// Action is allowed
    Allow,

    /// Action is denied
    Deny {
        rule_id: String,
        reason: String,
    },

    /// Action requires approval
    RequiresApproval {
        rule_id: String,
        approvers: ApproverSpec,
        timeout: Option<Duration>,
    },

    /// Action should be escalated
    Escalate {
        rule_id: String,
        target: EscalationTarget,
        reason: String,
    },

    /// Action should be delegated
    Delegate {
        target_agent: AgentId,
        reason: String,
    },

    /// Budget exceeded
    BudgetExceeded {
        budget_type: String,
        limit: u64,
        current: u64,
    },
}
```

---

## Example Configurations

### Example: Software Engineer Agent

```yaml
id: "eng-alice"
name: "Alice (Software Engineer)"
description: "Full-stack engineer specializing in Rust and TypeScript"

archetype: "software-engineer"

organization:
  team: "platform-team"
  reports_to: "eng-lead-bob"
  peers: ["eng-charlie", "eng-diana"]

skills:
  base_skills:
    - skill_id: "code-review"
      source: { type: "builtin", name: "code-review" }
    - skill_id: "rust-development"
      source: { type: "git", repo: "github.com/company/skills", path: "rust-dev" }
  available_skills:
    - skill_id: "debugging"
      source: { type: "builtin", name: "debugging" }
    - skill_id: "performance-analysis"
      source: { type: "registry", registry: "skills.shiioo.io", name: "perf-analysis" }
  discovery: "task_matching"
  max_active_skills: 3

policies:
  allow:
    - id: "allow-repo-access"
      description: "Can access company repositories"
      scope: { type: "tools", tool_ids: ["repo_read", "repo_write"] }
      conditions: []
    - id: "allow-web-docs"
      description: "Can fetch documentation sites"
      scope: { type: "tools", tool_ids: ["web_fetch"] }
      conditions:
        - type: "resource_pattern"
          field: "url"
          patterns: ["docs.*", "*.github.io", "developer.*"]

  deny:
    - id: "deny-production-direct"
      description: "Cannot directly modify production"
      scope: { type: "resources", patterns: ["**/production/**", "**/prod/**"] }
      conditions: []
    - id: "deny-secrets"
      description: "Cannot access secret files"
      scope: { type: "resources", patterns: ["**/.env", "**/credentials*", "**/secrets*"] }
      conditions: []

  requires_approval:
    - id: "approve-deploy"
      trigger: { type: "tools", tool_ids: ["deploy", "release"] }
      approvers: { type: "supervisor" }
      timeout: "1h"
      timeout_action: "deny"
    - id: "approve-large-changes"
      trigger: { type: "custom", expression: "files_changed > 50" }
      approvers: { type: "role", role_id: "tech-lead" }
      timeout: "4h"
      timeout_action: "escalate"

  delegation:
    can_delegate: true
    delegate_to:
      - agent_id: "eng-charlie"
        scopes: [{ type: "tools", tool_ids: ["repo_read", "context_search"] }]
        requires_approval: false
    delegatable_scopes:
      - { type: "tool_tier", min_tier: 0 }  # Can delegate read-only tasks
    must_escalate:
      - trigger: { type: "cost_exceeds", cents: 1000 }
        escalate_to: { type: "supervisor" }

mcp_bindings:
  - id: "github"
    server: { type: "reference", server_id: "github-mcp" }
    tools: { type: "all" }
    credentials: { type: "from_grants", grant_id: "github-token" }
  - id: "jira"
    server: { type: "reference", server_id: "jira-mcp" }
    tools: { type: "specific", tool_ids: ["jira_read", "jira_comment"] }
    credentials: { type: "from_grants", grant_id: "jira-token" }

secret_grants:
  - id: "github-token"
    secret_id: "github-pat-alice"
    permissions: { read: false, use_: true, rotate: false }
    conditions:
      - type: "for_tools"
        tool_ids: ["repo_read", "repo_write", "pr_create"]
  - id: "jira-token"
    secret_id: "jira-api-token"
    permissions: { read: false, use_: true, rotate: false }
    conditions: []

budgets:
  tokens:
    per_request: 100000
    per_day: 5000000
  cost:
    per_day_cents: 5000  # $50/day
  requests:
    per_minute: 30
  concurrency:
    max_concurrent_steps: 3
    max_concurrent_tools: 5

status: "active"
```

### Example: Manager Agent

```yaml
id: "eng-lead-bob"
name: "Bob (Engineering Lead)"
description: "Engineering team lead, approves deployments and architecture decisions"

archetype: "engineering-lead"

organization:
  team: "platform-team"
  reports_to: "vp-engineering"
  supervises: ["eng-alice", "eng-charlie", "eng-diana"]
  peers: ["eng-lead-eve", "eng-lead-frank"]

skills:
  base_skills:
    - skill_id: "code-review"
      source: { type: "builtin", name: "code-review" }
    - skill_id: "architecture-review"
      source: { type: "builtin", name: "architecture-review" }
    - skill_id: "incident-response"
      source: { type: "builtin", name: "incident-response" }
  discovery: "explicit"
  max_active_skills: 5

policies:
  allow:
    - id: "allow-all-repos"
      description: "Can access all repositories"
      scope: { type: "tools", tool_ids: ["repo_read", "repo_write"] }
      conditions: []
    - id: "allow-deploy"
      description: "Can deploy to staging and production"
      scope: { type: "tools", tool_ids: ["deploy"] }
      conditions: []

  deny:
    - id: "deny-direct-prod-hotfix"
      description: "Cannot hotfix production without approval"
      scope: { type: "resources", patterns: ["**/production/hotfix/**"] }
      conditions: []

  requires_approval:
    - id: "approve-prod-deploy"
      trigger: { type: "tools", tool_ids: ["deploy_production"] }
      approvers: { type: "board", board_id: "release-board" }
      timeout: "2h"
      timeout_action: "deny"

  delegation:
    can_delegate: true
    delegate_to:
      - agent_id: "eng-alice"
        scopes: [{ type: "all" }]
        requires_approval: false
      - agent_id: "eng-charlie"
        scopes: [{ type: "all" }]
        requires_approval: false
    delegatable_scopes:
      - { type: "all" }
    must_escalate:
      - trigger: { type: "cost_exceeds", cents: 10000 }
        escalate_to: { type: "human", person_id: "vp-engineering" }

budgets:
  tokens:
    per_day: 10000000
  cost:
    per_day_cents: 20000  # $200/day
  concurrency:
    max_concurrent_steps: 10

status: "active"
```

---

## Migration from Existing Types

### Mapping to Existing Concepts

| Old Concept | New Concept | Migration |
|-------------|-------------|-----------|
| `RoleSpec` | `Archetype` | Roles become archetypes that agents inherit from |
| `Person` | `Agent` | Persons with agent roles become Agents |
| `PolicySpec` | `AgentPolicies` | Policies attached to agents/archetypes |
| `RoleBudgets` | `AgentBudgets` | Expanded budget model |
| `McpServerConfig` | `McpBinding` | Per-agent tool bindings |

### Coexistence Strategy

During migration, both models can coexist:

1. Existing `RoleSpec` entries become `Archetype` entries
2. Existing `Person` entries with agent roles spawn `Agent` entries
3. `PolicySpec` rules are distributed to relevant archetypes/agents
4. New agents use the new model; legacy code uses adapters

---

## API Surface

### REST Endpoints

```
# Agents
GET    /api/agents                    # List agents
POST   /api/agents                    # Create agent
GET    /api/agents/{id}               # Get agent
PUT    /api/agents/{id}               # Update agent
DELETE /api/agents/{id}               # Archive agent
POST   /api/agents/{id}/activate      # Activate agent
POST   /api/agents/{id}/suspend       # Suspend agent

# Agent Skills
GET    /api/agents/{id}/skills                    # List agent's skills
POST   /api/agents/{id}/skills/{skill_id}/activate   # Activate skill
DELETE /api/agents/{id}/skills/{skill_id}/activate   # Deactivate skill
GET    /api/agents/{id}/skills/match?task=...     # Match skills to task

# Agent Policies
GET    /api/agents/{id}/policies              # Get effective policies
POST   /api/agents/{id}/policies/evaluate     # Evaluate action against policies

# Archetypes
GET    /api/archetypes                # List archetypes
POST   /api/archetypes                # Create archetype
GET    /api/archetypes/{id}           # Get archetype
PUT    /api/archetypes/{id}           # Update archetype

# Skills Library
GET    /api/skills                    # List available skills
GET    /api/skills/{id}               # Get skill metadata
GET    /api/skills/{id}/full          # Get full skill (instructions + resources)
POST   /api/skills                    # Register skill
```

### GraphQL Schema

```graphql
type Agent {
  id: ID!
  name: String!
  description: String!
  archetype: Archetype
  organization: AgentOrganization!
  skills: AgentSkills!
  policies: AgentPolicies!
  mcpBindings: [McpBinding!]!
  budgets: AgentBudgets!
  status: AgentStatus!
  createdAt: DateTime!
  updatedAt: DateTime!
}

type AgentSkills {
  baseSkills: [SkillRef!]!
  availableSkills: [SkillRef!]!
  activeSkills: [Skill!]!
  discovery: SkillDiscovery!
}

type Skill {
  name: String!
  description: String!
  allowedTools: [String!]!
  instructions: String!
  scripts: [SkillScript!]!
  references: [SkillReference!]!
}

type Query {
  agent(id: ID!): Agent
  agents(filter: AgentFilter): [Agent!]!
  archetype(id: ID!): Archetype
  skill(id: ID!): Skill

  # Policy evaluation
  evaluateAction(agentId: ID!, action: AgentActionInput!): PolicyDecision!
}

type Mutation {
  createAgent(input: CreateAgentInput!): Agent!
  updateAgent(id: ID!, input: UpdateAgentInput!): Agent!
  activateSkill(agentId: ID!, skillId: String!): Agent!
  deactivateSkill(agentId: ID!, skillId: String!): Agent!
}

type Subscription {
  agentStatusChanged(agentId: ID): AgentStatusEvent!
  skillActivated(agentId: ID): SkillActivationEvent!
  policyDecision(agentId: ID): PolicyDecisionEvent!
}
```

---

## Implementation Phases

### Phase 1: Core Types
- [ ] Define `Agent`, `AgentId`, `AgentStatus` types
- [ ] Define `AgentOrganization` type
- [ ] Define `AgentBudgets` type
- [ ] Add to `crates/core/src/types.rs` or new `agent.rs` module

### Phase 2: Skills Integration
- [ ] Define `Skill`, `SkillRef`, `SkillSource` types
- [ ] Implement SKILL.md parser
- [ ] Implement `SkillManager` trait
- [ ] Add skill loading from local/git/registry sources

### Phase 3: Policy Layer
- [ ] Define `AgentPolicies`, `PolicyRule`, `ApprovalRule` types
- [ ] Define `DelegationRules`, `EscalationRule` types
- [ ] Implement `AgentPolicyEngine` trait
- [ ] Implement policy inheritance resolution

### Phase 4: MCP Bindings
- [ ] Define `McpBinding`, `McpServerRef`, `CredentialRef` types
- [ ] Implement per-agent MCP server configuration
- [ ] Implement credential scoping

### Phase 5: Archetypes
- [ ] Define `Archetype`, `ArchetypeId` types
- [ ] Implement archetype inheritance
- [ ] Migrate existing `RoleSpec` to archetypes

### Phase 6: API & Storage
- [ ] Add REST endpoints
- [ ] Add GraphQL schema
- [ ] Add agent storage (events + index)

### Phase 7: Migration
- [ ] Create migration tooling
- [ ] Migrate existing data
- [ ] Update workflow executor to use new agent model

---

## Open Questions

1. **Skill versioning** — How do we handle skill updates? Pin versions or auto-update?

2. **Cross-org skills** — Can agents use skills from other organizations?

3. **Runtime skill loading** — Should agents be able to discover and load skills at runtime without explicit assignment?

4. **Policy conflicts** — How do we resolve conflicts between inherited policies? Last-write-wins, most-restrictive, or explicit priority?

5. **Agent identity** — Are agents immutable identities or can they be "reassigned" to different archetypes?

6. **Human-agent parity** — Should human users also be modeled as agents with the same primitives?
