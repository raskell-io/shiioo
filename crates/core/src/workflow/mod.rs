//! Workflow execution engine.
//!
//! This module provides DAG-based workflow execution with support for parallel
//! steps, conditional branching, and human-in-the-loop approvals.
//!
//! # Concepts
//!
//! ## Workflow DAG
//!
//! Workflows are represented as Directed Acyclic Graphs (DAGs) where:
//! - **Nodes** are steps to execute
//! - **Edges** define dependencies between steps
//! - Steps only execute when all dependencies are satisfied
//!
//! ```text
//! ┌─────────┐     ┌─────────┐     ┌─────────┐
//! │  Step A │────▶│  Step B │────▶│  Step D │
//! └─────────┘     └─────────┘     └─────────┘
//!      │                               ▲
//!      │          ┌─────────┐          │
//!      └─────────▶│  Step C │──────────┘
//!                 └─────────┘
//! ```
//!
//! ## Step Types
//!
//! - **Agent Steps** - Executed by an AI agent with a specific skill
//! - **Approval Steps** - Require human approval before continuing
//! - **Conditional Steps** - Branch based on previous step outputs
//! - **Parallel Steps** - Execute multiple steps concurrently
//!
//! ## Execution Model
//!
//! 1. The [`WorkflowExecutor`] loads the workflow specification
//! 2. The [`WorkflowDag`] validates the graph structure (no cycles)
//! 3. Steps are executed in topological order
//! 4. The [`StepExecutor`] handles individual step execution
//! 5. Events are logged for each state transition
//!
//! # Components
//!
//! - [`WorkflowDag`] - DAG representation with dependency tracking
//! - [`WorkflowExecutor`] - Main execution orchestrator
//! - [`StepExecutor`] - Individual step execution logic
//! - [`WorkflowVersionManager`] - Version control for workflow definitions
//!
//! # Advanced Features
//!
//! The [`advanced`] module provides:
//! - [`ParallelForEachBuilder`] - Dynamic parallel step generation
//! - [`evaluate_condition`] - Conditional expression evaluation
//! - [`WorkflowVersion`] - Immutable workflow snapshots
//!
//! # Example
//!
//! ```rust,ignore
//! use shiioo_core::workflow::{WorkflowDag, WorkflowExecutor};
//! use shiioo_core::types::WorkflowSpec;
//!
//! // Create a workflow DAG from a specification
//! let dag = WorkflowDag::from_spec(&workflow_spec)?;
//!
//! // Validate no cycles exist
//! dag.validate()?;
//!
//! // Get execution order
//! let order = dag.topological_order()?;
//!
//! // Execute with the workflow executor
//! let executor = WorkflowExecutor::new(event_log, blob_store);
//! executor.execute(run_id, workflow_spec).await?;
//! ```

pub mod dag;
pub mod executor;
pub mod step_executor;
pub mod advanced;

pub use dag::WorkflowDag;
pub use executor::WorkflowExecutor;
pub use step_executor::StepExecutor;
pub use advanced::{
    AdvancedPattern, ParallelForEachBuilder, WorkflowVersion, WorkflowVersionManager,
    evaluate_condition, expand_parallel_foreach,
};
