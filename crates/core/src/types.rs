use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Unique identifier for a workflow run
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(pub Uuid);

impl RunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a workflow step
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StepId(pub String);

impl StepId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl std::fmt::Display for StepId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a role
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoleId(pub String);

impl RoleId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Unique identifier for a policy
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PolicyId(pub String);

/// Content-addressed blob hash (SHA-256)
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlobHash(pub String);

impl BlobHash {
    pub fn from_bytes(data: &[u8]) -> Self {
        use sha2::{Digest, Sha256};
        let hash = Sha256::digest(data);
        Self(hex::encode(hash))
    }
}

/// Status of a workflow run
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Status of a workflow step
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Skipped,
}

/// Approval status for a pending action
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
}

/// A unit of work - can be a Job (one-time) or Routine (recurring)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WorkItem {
    Job(Job),
    Routine(Routine),
}

/// A one-time job with a workflow to execute
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub workflow: WorkflowSpec,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

/// A recurring routine with a schedule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routine {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub workflow: WorkflowSpec,
    pub schedule: CronSchedule,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub enabled: bool,
}

/// Cron-like schedule specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronSchedule {
    pub expression: String,
    pub timezone: Option<String>,
}

/// Specification for a workflow (DAG of steps)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub steps: Vec<StepSpec>,
    pub dependencies: HashMap<StepId, Vec<StepId>>,
}

/// Specification for a single workflow step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepSpec {
    pub id: StepId,
    pub name: String,
    pub description: Option<String>,
    pub role: RoleId,
    pub action: StepAction,
    pub timeout_secs: Option<u64>,
    pub retry_policy: Option<RetryPolicy>,
    pub requires_approval: bool,
}

/// Action to perform in a step
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StepAction {
    /// Execute an agent with a prompt
    AgentTask { prompt: String },
    /// Execute a sequence of tool calls
    ToolSequence { tools: Vec<ToolCallSpec> },
    /// Wait for manual approval
    ManualApproval { approvers: Vec<String> },
    /// Run a subprocess/script
    Script { command: String, args: Vec<String> },
}

/// Specification for a tool call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallSpec {
    pub tool_id: String,
    pub parameters: serde_json::Value,
}

/// Retry policy for a step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff_secs: u64,
}

/// A specific execution of a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: RunId,
    pub work_item_id: String,
    pub status: RunStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub steps: Vec<StepExecution>,
}

/// Execution state of a workflow step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepExecution {
    pub id: StepId,
    pub status: StepStatus,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub attempt: u32,
    pub error: Option<String>,
}

/// Role specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleSpec {
    pub id: RoleId,
    pub name: String,
    pub description: String,
    pub prompt_template: String,
    pub allowed_tools: Vec<String>,
    pub budgets: RoleBudgets,
    pub requires_approval_for: Vec<String>, // Tool IDs or tiers
}

/// Budget limits for a role
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleBudgets {
    pub daily_tokens: Option<u64>,
    pub daily_cost_cents: Option<u64>,
}

/// Policy specification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicySpec {
    pub id: PolicyId,
    pub name: String,
    pub description: String,
    pub rules: Vec<PolicyRule>,
}

/// Individual policy rule
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PolicyRule {
    DenyPath { patterns: Vec<String> },
    AllowDomain { domains: Vec<String> },
    RequireApproval { tool_ids: Vec<String> },
    EnforceEnvironment { environment: String },
}

/// Configuration change proposal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigProposal {
    pub id: String,
    pub title: String,
    pub description: String,
    pub proposed_by: String,
    pub proposed_at: DateTime<Utc>,
    pub diff: ConfigDiff,
    pub approval_status: ApprovalStatus,
}

/// Diff of configuration changes
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDiff {
    pub roles_added: Vec<RoleSpec>,
    pub roles_modified: Vec<RoleSpec>,
    pub roles_removed: Vec<RoleId>,
    pub policies_added: Vec<PolicySpec>,
    pub policies_modified: Vec<PolicySpec>,
    pub policies_removed: Vec<PolicyId>,
}

// === Phase 3: Organization & Templates ===

/// Unique identifier for an organization
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct OrgId(pub String);

impl OrgId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Unique identifier for a team
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TeamId(pub String);

impl TeamId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Unique identifier for a person in the organization
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersonId(pub String);

impl PersonId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Unique identifier for a process template
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TemplateId(pub String);

impl TemplateId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Organization definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Organization {
    pub id: OrgId,
    pub name: String,
    pub description: String,
    pub teams: Vec<Team>,
    pub people: Vec<Person>,
    pub org_chart: OrgChart,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Team within an organization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Team {
    pub id: TeamId,
    pub name: String,
    pub description: String,
    pub lead: Option<PersonId>,
    pub members: Vec<PersonId>,
    pub parent_team: Option<TeamId>,
}

/// Person in the organization
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Person {
    pub id: PersonId,
    pub name: String,
    pub email: String,
    pub role: RoleId,
    pub team: TeamId,
    pub reports_to: Option<PersonId>,
    pub can_approve: Vec<String>, // List of approval types this person can approve
}

/// Organizational chart structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrgChart {
    pub root_team: TeamId,
    pub reporting_structure: HashMap<PersonId, PersonId>, // person -> manager
}

/// Process template for reusable workflows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProcessTemplate {
    pub id: TemplateId,
    pub name: String,
    pub description: String,
    pub category: String, // e.g., "code_review", "deployment", "analysis"
    pub parameters: Vec<TemplateParameter>,
    pub workflow_template: WorkflowSpec,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

/// Parameter for a process template
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateParameter {
    pub name: String,
    pub description: String,
    pub param_type: TemplateParameterType,
    pub default_value: Option<String>,
    pub required: bool,
}

/// Type of template parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateParameterType {
    String,
    Number,
    Boolean,
    RoleId,
    TeamId,
    PersonId,
}

/// Instantiation of a template with specific parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateInstance {
    pub template_id: TemplateId,
    pub parameters: HashMap<String, String>,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
}

/// Claude Code configuration (for .claude/config.json generation)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeConfig {
    pub mcp_servers: HashMap<String, McpServerConfig>,
    pub tools: Vec<ToolConfig>,
    pub settings: ClaudeSettings,
}

/// MCP server configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

/// Tool configuration for Claude
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    pub name: String,
    pub enabled: bool,
    pub tier: u8,
    pub requires_approval: bool,
}

/// Claude settings
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeSettings {
    pub max_tokens: Option<u64>,
    pub temperature: Option<f64>,
    pub model: Option<String>,
}
