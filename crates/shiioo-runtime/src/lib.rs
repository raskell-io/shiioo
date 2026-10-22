//! The agent loop.
//!
//! A [`Worker`] leases a run, rebuilds its state from the event log,
//! recovers tool calls a previous worker left behind, and executes turns:
//! model call → record the response → for each tool call: record the intent,
//! execute it on the [`LocalMachine`], record the result. Everything a later
//! worker needs to continue is in the log; see `docs/design/execution.md`.

pub mod anthropic;
mod context;
pub mod machine;
pub mod model;
pub mod scripted;
pub mod spec;
pub mod status;
pub mod tools;
pub mod worker;

pub use anthropic::AnthropicProvider;
pub use machine::{CallStatus, Invocation, LocalMachine, MachineError};
pub use model::{
    BoxFuture, ModelProvider, ModelRequest, ModelResponse, ProviderError, StopReason, ToolCall,
    ToolResultBlock, ToolSpec, Transcript, Turn, Usage,
};
pub use scripted::ScriptedProvider;
pub use spec::{AgentSpec, RunSpec};
pub use status::{RunStatus, run_status};
pub use tools::{Tool, ToolOutput, Toolbox};
pub use worker::{CrashPoint, Faults, RunOutcome, Worker, WorkerError};
