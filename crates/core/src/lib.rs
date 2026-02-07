//! # Shiioo Core
//!
//! The core library for the Shiioo Virtual Company OS - a platform for
//! building and managing virtual companies of AI employees.
//!
//! ## Overview
//!
//! Shiioo enables organizations to deploy AI agents as virtual employees, complete with:
//! - **Skills and Capabilities** - Agents have defined skills following the Agent Skills format
//! - **Policies and Governance** - Fine-grained control over what agents can and cannot do
//! - **Budget Management** - Token, cost, and request budgets with enforcement
//! - **MCP Tool Integration** - Connect agents to external systems via Model Context Protocol
//! - **Multi-tenant Isolation** - Secure separation between different organizations
//!
//! ## Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                           SHIIOO ARCHITECTURE                               │
//! ├─────────────────────────────────────────────────────────────────────────────┤
//! │                                                                             │
//! │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐        │
//! │  │   AGENTS    │  │  WORKFLOWS  │  │  POLICIES   │  │  CAPACITY   │        │
//! │  │             │  │             │  │             │  │             │        │
//! │  │ • Skills    │  │ • DAG       │  │ • RBAC      │  │ • Pooling   │        │
//! │  │ • Budgets   │  │ • Steps     │  │ • Approval  │  │ • Rate Lim  │        │
//! │  │ • MCP Tools │  │ • Executor  │  │ • Audit     │  │ • Cost Track│        │
//! │  └─────────────┘  └─────────────┘  └─────────────┘  └─────────────┘        │
//! │         │                │                │                │               │
//! │         └────────────────┴────────────────┴────────────────┘               │
//! │                                   │                                        │
//! │                          ┌────────┴────────┐                               │
//! │                          │     STORAGE     │                               │
//! │                          │                 │                               │
//! │                          │ • Event Log    │                               │
//! │                          │ • Blob Store   │                               │
//! │                          │ • Agent Store  │                               │
//! │                          │ • Index Store  │                               │
//! │                          └─────────────────┘                               │
//! │                                                                             │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Module Guide
//!
//! ### Core Primitives
//!
//! - [`agent`] - The Agent primitive: skills, policies, MCP bindings, and budgets
//! - [`types`] - Core type definitions: IDs, specs, statuses, and domain models
//! - [`events`] - Event sourcing: event types, logging, and replay
//!
//! ### Execution
//!
//! - [`workflow`] - Workflow engine: DAG-based execution with step coordination
//! - [`capacity`] - Capacity management: LLM pooling, rate limiting, cost tracking
//! - [`scheduler`] - Task scheduling: cron-based routines and job management
//!
//! ### Governance
//!
//! - [`policy`] - Policy evaluation: allow/deny rules with budget enforcement
//! - [`approval`] - Approval workflows: human-in-the-loop with quorum rules
//! - [`rbac`] - Role-based access control: permissions and role management
//! - [`compliance`] - Compliance checking: SOX, HIPAA, PCI-DSS frameworks
//! - [`audit`] - Audit logging: immutable audit trail for all actions
//!
//! ### Organization
//!
//! - [`organization`] - Org structure: teams, people, reporting hierarchies
//! - [`tenant`] - Multi-tenancy: tenant isolation, quotas, and configuration
//! - [`secrets`] - Secret management: encrypted credential storage
//!
//! ### Infrastructure
//!
//! - [`storage`] - Storage layer: blobs, events, agents, and indexes
//! - [`cluster`] - Clustering: distributed coordination and leader election
//! - [`metrics`] - Metrics collection: counters, gauges, and histograms
//! - [`analytics`] - Analytics: workflow tracking and performance analysis
//!
//! ### Configuration
//!
//! - [`template`] - Process templates: parameterized workflow definitions
//! - [`config_change`] - Config management: proposals, diffs, and rollback
//! - [`claude_compiler`] - Claude compilation: system prompt generation
//!
//! ## Quick Start
//!
//! ### Creating an Agent
//!
//! ```rust,ignore
//! use shiioo_core::agent::{Agent, AgentId, AgentOrganization, AgentBudgets, TokenBudget};
//! use shiioo_core::types::TeamId;
//!
//! let agent = Agent::builder(AgentId::new("analyst-1"), "Data Analyst")
//!     .organization(AgentOrganization::new(TeamId::new("analytics-team")))
//!     .budgets(AgentBudgets::new()
//!         .with_tokens(TokenBudget::new()
//!             .with_per_hour(10000)
//!             .with_per_day(100000)))
//!     .build();
//! ```
//!
//! ### Executing a Task
//!
//! ```rust,ignore
//! use shiioo_core::agent::{AgentRuntime, AgentTask, RuntimeConfig, ExecutionContext};
//!
//! let runtime = AgentRuntime::new(
//!     RuntimeConfig::default(),
//!     policy_engine,
//!     mcp_resolver,
//!     tool_executor,
//!     event_log,
//! );
//!
//! let task = AgentTask {
//!     id: "task-1".to_string(),
//!     prompt: "Analyze the Q4 sales data".to_string(),
//!     skill: Some("data-analysis".to_string()),
//!     context: Default::default(),
//!     run_id: None,
//!     step_id: None,
//! };
//!
//! let result = runtime.execute_task(&agent, task, ExecutionContext::default()).await?;
//! ```
//!
//! ### Setting Up an Orchestrator
//!
//! ```rust,ignore
//! use shiioo_core::agent::{AgentOrchestrator, OrchestrationConfig};
//!
//! let orchestrator = AgentOrchestrator::new(
//!     OrchestrationConfig::default(),
//!     runtime,
//!     agent_store,
//!     event_log,
//! );
//!
//! // Create a team of agents
//! orchestrator.create_team(team).await?;
//!
//! // Delegate work between agents
//! orchestrator.delegate(delegation_request).await?;
//! ```
//!
//! ## Feature Flags
//!
//! This crate does not currently use feature flags. All functionality is included
//! by default.
//!
//! ## Thread Safety
//!
//! All public types in this crate are designed to be `Send + Sync` where appropriate.
//! The runtime components use `Arc` and `RwLock` for safe concurrent access.

pub mod types;
pub mod agent;
pub mod storage;
pub mod events;
pub mod workflow;
pub mod policy;
pub mod organization;
pub mod template;
pub mod claude_compiler;
pub mod capacity;
pub mod scheduler;
pub mod approval;
pub mod config_change;
pub mod metrics;
pub mod analytics;
pub mod tenant;
pub mod cluster;
pub mod secrets;
pub mod audit;
pub mod rbac;
pub mod compliance;

pub use types::*;
