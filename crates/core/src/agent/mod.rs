//! Agent primitive for the virtual company.
//!
//! An Agent is the fundamental unit of work in Shiioo — a virtual employee with
//! defined skills, governed by policies, and connected to external systems via MCP tools.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                            AGENT                                │
//! ├─────────────────────────────────────────────────────────────────┤
//! │  Identity        │  who this agent is                           │
//! │  Skills          │  what this agent knows (Agent Skills format) │
//! │  Policies        │  what this agent can/cannot do               │
//! │  MCP Bindings    │  what systems this agent can access          │
//! │  Secrets         │  credentials this agent can use              │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Skills
//!
//! Skills follow the [Agent Skills](https://agentskills.io) open format:
//!
//! ```text
//! skill-name/
//! ├── SKILL.md          # Required: YAML frontmatter + markdown instructions
//! ├── scripts/          # Optional: executable code
//! ├── references/       # Optional: documentation
//! └── assets/           # Optional: templates, resources
//! ```
//!
//! Use [`SkillParser`] to parse SKILL.md files and [`SkillManager`] to manage
//! skill lifecycle for agents.

mod types;
mod skills;
mod policies;
mod mcp;
mod budgets;
mod archetype;
pub mod skill_parser;
pub mod skill_manager;
pub mod builtin_skills;
pub mod skill_registry;
pub mod format_converter;
pub mod git_import;
pub mod policy_engine;
pub mod mcp_manager;
pub mod archetype_manager;
pub mod migration;
pub mod runtime;
pub mod orchestration;

pub use types::*;
pub use skills::*;
pub use policies::*;
pub use mcp::*;
pub use budgets::*;
pub use archetype::*;
pub use skill_parser::SkillParser;
pub use skill_manager::{DefaultSkillManager, SkillManager, SkillManagerBuilder, SkillManagerConfig};
pub use skill_registry::{RegistryEntry, RegistryEntrySource, SkillRegistry};
pub use format_converter::{ContainsStudioConverter, FormatConverter, FormatConverterRegistry};
pub use git_import::{
    DiscoveredFile, GitImporter, ImportError, ImportMethod, ImportRequest, ImportResult,
    ImportedSkill, ScanResult,
};
pub use policy_engine::{
    ActionContext, ActionType, AgentAction, AgentPolicyEngine, DefaultPolicyEngine,
    EffectivePolicies, InMemoryPolicyStorage, PolicyDecision, PolicySource, PolicyStorage,
};
pub use mcp_manager::{
    CredentialType, InMemorySecretStore, InMemoryServerRegistry, McpBindingManager,
    McpServerConfig, McpServerRegistry, McpServerType, ResolvedCredential, ResolvedMcpConfig,
    ResolvedMcpServer, ResolvedTool, SecretStore, ToolInfo,
};
pub use archetype_manager::{
    ArchetypeError, ArchetypeRegistry, ArchetypeResolver, ArchetypeResolverConfig,
    InMemoryArchetypeRegistry, ResolvedArchetype, RoleSpecAdapter,
};
pub use migration::{
    FullMigrationReport, MigrationConfig, MigrationError, MigrationReport, MigrationResult,
    MigrationRunner, PersonAdapter, PolicySpecAdapter,
};
pub use runtime::{
    AgentRuntime, AgentTask, BudgetConsumed, ExecutionContext, InMemoryMcpResolver,
    InMemoryToolExecutor, McpResolver, RuntimeConfig, RuntimeToolInfo, TaskResult, ToolCallRecord,
    ToolExecutor,
};
pub use orchestration::{
    AgentOrchestrator, AgentSupervisor, AgentTeam, AgentWorkflowAdapter, DelegationRequest,
    DelegationResult, OrchestrationConfig, SupervisionAction, SupervisionEvent,
    SupervisionEventType, SupervisionRestartPolicy, SupervisionStrategy, TeamChannel,
};
