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

/// Unique identifier for a routine
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoutineId(pub String);

impl RoutineId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Recurring workflow with cron schedule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routine {
    pub id: RoutineId,
    pub name: String,
    pub description: String,
    pub schedule: RoutineSchedule,
    pub workflow: WorkflowSpec,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    pub next_run: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub updated_at: DateTime<Utc>,
}

/// Cron schedule configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineSchedule {
    pub cron: String, // Cron expression (e.g., "0 0 * * *" for daily at midnight)
    pub timezone: String, // IANA timezone (e.g., "America/New_York")
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

// === Phase 4: Capacity Broker ===

/// Unique identifier for a capacity source
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapacitySourceId(pub String);

impl CapacitySourceId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// LLM capacity source (API key, provider, model)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacitySource {
    pub id: CapacitySourceId,
    pub name: String,
    pub provider: LlmProvider,
    pub api_key_hash: String, // SHA-256 hash of API key (not plaintext)
    pub model: String,
    pub rate_limits: RateLimits,
    pub cost_per_token: CostPerToken,
    pub priority: u8, // 0-255, higher = preferred
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// LLM provider type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    Anthropic,
    OpenAI,
    Azure,
    Custom { endpoint: String },
}

/// Rate limits for a capacity source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimits {
    pub requests_per_minute: u32,
    pub tokens_per_minute: u32,
    pub tokens_per_day: Option<u32>,
}

/// Cost per token (per 1M tokens)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostPerToken {
    pub input_cost: f64,  // Cost per 1M input tokens (USD)
    pub output_cost: f64, // Cost per 1M output tokens (USD)
}

/// Usage tracking for a capacity source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapacityUsage {
    pub id: String, // Unique ID for this usage record
    pub source_id: CapacitySourceId,
    pub timestamp: DateTime<Utc>,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
    pub cost: f64,
    pub request_count: u32,
    pub run_id: Option<RunId>,
    pub step_id: Option<StepId>,
}

/// Rate limit state for a capacity source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitState {
    pub source_id: CapacitySourceId,
    pub window_start: DateTime<Utc>,
    pub requests_in_window: u32,
    pub tokens_in_window: u32,
    pub daily_tokens: u32,
    pub daily_reset_at: DateTime<Utc>,
    pub next_available: Option<DateTime<Utc>>, // When this source can be used again
    pub backoff_until: Option<DateTime<Utc>>, // Exponential backoff end time
}

/// Priority request in the queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriorityRequest {
    pub id: String,
    pub priority: u8, // 0-255, higher = more urgent
    pub run_id: RunId,
    pub step_id: StepId,
    pub role: RoleId,
    pub prompt: String,
    pub max_tokens: u32,
    pub created_at: DateTime<Utc>,
    pub attempts: u32,
}

/// LLM request sent to a capacity source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmRequest {
    pub prompt: String,
    pub max_tokens: u32,
    pub temperature: Option<f64>,
    pub model: Option<String>, // Override source model if needed
}

/// LLM response from a capacity source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cost: f64,
    pub model: String,
    pub source_id: CapacitySourceId,
}

/// Error from LLM API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum LlmError {
    RateLimited { retry_after: Option<u64> }, // Seconds until retry
    InvalidRequest { message: String },
    AuthenticationFailed,
    ServiceUnavailable,
    TimeoutExceeded,
    Other { message: String },
}

// === Phase 5: Routines + Approval Boards ===

/// Record of a routine execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutineExecution {
    pub id: String,
    pub routine_id: RoutineId,
    pub run_id: RunId,
    pub scheduled_at: DateTime<Utc>,
    pub executed_at: DateTime<Utc>,
    pub status: RunStatus,
    pub error: Option<String>,
}

/// Unique identifier for an approval board
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApprovalBoardId(pub String);

impl ApprovalBoardId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Approval board with quorum rules
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalBoard {
    pub id: ApprovalBoardId,
    pub name: String,
    pub description: String,
    pub approvers: Vec<PersonId>, // People who can approve
    pub quorum_rule: QuorumRule,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Quorum rules for approval
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuorumRule {
    Unanimous, // All approvers must approve
    Majority,  // More than 50% must approve
    MinCount { min: u32 }, // At least N approvers
    Percentage { percent: u8 }, // At least X% of approvers (0-100)
}

/// Unique identifier for an approval
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApprovalId(pub String);

impl ApprovalId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Pending approval request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub id: ApprovalId,
    pub board_id: ApprovalBoardId,
    pub subject: ApprovalSubject,
    pub status: ApprovalStatus,
    pub votes: Vec<ApprovalVote>,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// What is being approved
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalSubject {
    ConfigChange { change_id: ConfigChangeId },
    WorkflowRun { run_id: RunId },
    PolicyChange { policy_id: PolicyId },
    RoleChange { role_id: RoleId },
    Custom { subject_type: String, subject_id: String },
}

/// Individual vote on an approval
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalVote {
    pub voter: PersonId,
    pub vote: VoteDecision,
    pub comment: Option<String>,
    pub voted_at: DateTime<Utc>,
}

/// Vote decision
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoteDecision {
    Approve,
    Reject,
    Abstain,
}

/// Unique identifier for a config change
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConfigChangeId(pub String);

impl ConfigChangeId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Proposed configuration change
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigChange {
    pub id: ConfigChangeId,
    pub change_type: ConfigChangeType,
    pub description: String,
    pub proposed_by: String,
    pub approval_id: Option<ApprovalId>, // If approval is required
    pub status: ConfigChangeStatus,
    pub before: Option<String>, // JSON snapshot before change
    pub after: String, // JSON of proposed change
    pub applied_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Type of configuration change
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigChangeType {
    Role,
    Policy,
    Organization,
    Template,
    CapacitySource,
    Routine,
    ApprovalBoard,
}

/// Status of a config change
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigChangeStatus {
    Proposed,
    PendingApproval,
    Approved,
    Rejected,
    Applied,
    Failed,
}
