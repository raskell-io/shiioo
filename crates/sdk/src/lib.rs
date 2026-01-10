//! # Shiioo SDK
//!
//! Official Rust SDK for Shiioo - Enterprise LLM Agent Orchestrator.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use shiioo_sdk::{ShiiooClient, ShiiooResult};
//!
//! #[tokio::main]
//! async fn main() -> ShiiooResult<()> {
//!     // Build client
//!     let client = ShiiooClient::builder()
//!         .base_url("https://shiioo.example.com")
//!         .api_key("sk-your-api-key")
//!         .build()?;
//!
//!     // Check health
//!     let health = client.health().check().await?;
//!     println!("Server status: {}", health.status);
//!
//!     // List runs
//!     let runs = client.runs().list().await?;
//!     println!("Found {} runs", runs.len());
//!
//!     Ok(())
//! }
//! ```
//!
//! ## WebSocket Subscriptions
//!
//! ```rust,no_run
//! use shiioo_sdk::{ShiiooClient, stream::SubscriptionEvent};
//!
//! # async fn example() -> shiioo_sdk::ShiiooResult<()> {
//! let client = ShiiooClient::builder()
//!     .base_url("https://shiioo.example.com")
//!     .build()?;
//!
//! let mut sub = client.subscribe().await?;
//! sub.subscribe_all().await?;
//!
//! while let Some(event) = sub.next_event().await {
//!     match event? {
//!         SubscriptionEvent::WorkflowUpdate { run_id, status, .. } => {
//!             println!("Workflow {} is now {}", run_id, status);
//!         }
//!         _ => {}
//!     }
//! }
//! # Ok(())
//! # }
//! ```

pub mod api;
pub mod client;
pub mod config;
pub mod error;
pub mod stream;
pub mod transport;

// Re-export main client
pub use client::{ShiiooClient, ShiiooClientBuilder};
pub use config::{ClientConfig, RetryConfig};
pub use error::{ShiiooError, ShiiooResult};

// Re-export core types for convenience
pub use shiioo_core::{
    // IDs
    types::{
        ApprovalBoardId, ApprovalId, BlobHash, CapacitySourceId, ConfigChangeId, OrgId, PersonId,
        PolicyId, RoleId, RoutineId, RunId, StepId, TeamId, TemplateId,
    },
    // Status enums
    types::{ApprovalStatus, ConfigChangeStatus, RunStatus, StepStatus},
    // Workflow types
    types::{
        Job, RetryPolicy, Routine, RoutineExecution, RoutineSchedule, Run, StepAction,
        StepExecution, StepSpec, WorkflowSpec,
    },
    // Role & Policy
    types::{PolicyRule, PolicySpec, RoleBudgets, RoleSpec},
    // Approval
    types::{Approval, ApprovalBoard, ApprovalSubject, ApprovalVote, QuorumRule, VoteDecision},
    // Organization
    types::{OrgChart, Organization, Person, Team},
    // Capacity
    types::{CapacitySource, CapacityUsage, CostPerToken, LlmProvider, RateLimits},
    // Config
    types::{ClaudeConfig, ConfigChange, ConfigChangeType, ConfigDiff},
    // Templates
    types::{ProcessTemplate, TemplateInstance, TemplateParameter, TemplateParameterType},
};

// Re-export events
pub use shiioo_core::events::{Event, EventType};

// Re-export analytics types
pub use shiioo_core::analytics::{BottleneckReport, ExecutionTrace, StepStats, WorkflowStats};

// Re-export metrics types
pub use shiioo_core::metrics::{Counter, Gauge, Histogram};

// Re-export tenant types
pub use shiioo_core::tenant::{Tenant, TenantId, TenantQuota, TenantSettings, TenantStatus};

// Re-export cluster types
pub use shiioo_core::cluster::{ClusterNode, NodeId, NodeRole, NodeStatus};

// Re-export secrets types
pub use shiioo_core::secrets::{RotationPolicy, Secret, SecretId, SecretType, SecretVersion};

// Re-export audit types
pub use shiioo_core::audit::{
    AuditAction, AuditCategory, AuditEntry, AuditSeverity, AuditStatistics,
};

// Re-export RBAC types
pub use shiioo_core::rbac::{Action, Permission, RbacRole, RbacUser, Resource};

// Re-export compliance types
pub use shiioo_core::compliance::{
    ComplianceFramework, ComplianceReport, SecurityScanReport,
};
