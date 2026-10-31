//! The agent loop.
//!
//! A [`Worker`] leases a run, rebuilds its state from the event log,
//! recovers tool calls a previous worker left behind, and executes turns:
//! model call → record the response → for each tool call: record the intent,
//! execute it on the [`LocalMachine`], record the result. Everything a later
//! worker needs to continue is in the log; see `docs/design/execution.md`.

pub mod anthropic;
mod context;
pub mod harness;
pub mod machine;
pub mod model;
pub mod redact;
pub mod sandbox;
pub mod scripted;
pub mod secrets;
pub mod spec;
pub mod status;
pub mod tools;
mod unconfigured;
pub mod worker;
pub mod workspace;

pub use anthropic::AnthropicProvider;
pub use harness::claude_code::{HookCommand, HookEndpoint};
pub use harness::{HarnessError, HarnessProcess};
pub use machine::{CallStatus, Invocation, LocalMachine, MachineError};
pub use model::{
    BoxFuture, ModelProvider, ModelRequest, ModelResponse, ProviderError, StopReason, ToolCall,
    ToolResultBlock, ToolSpec, Transcript, Turn, Usage,
};
pub use redact::Redactor;
pub use sandbox::{Sandbox, SandboxError};
pub use scripted::ScriptedProvider;
pub use secrets::{NoSecrets, SecretSource};
pub use spec::{
    AgentSpec, ContainerSpec, Egress, HarnessBilling, HarnessKind, HarnessSpec, Isolation, Mount,
    OpenshellSpec, Origin, RunSpec, Runtime,
};
pub use status::{
    RunStatus, open_approval, parked, run_status, takeover_pending, taken_over, wake_reason,
};
pub use tools::{Tool, ToolOutput, Toolbox};
pub use unconfigured::UnconfiguredProvider;
pub use worker::{CrashPoint, Faults, RunOutcome, TakeoverControl, Worker, WorkerError};
pub use workspace::{Worktree, WorktreeError, branch_for};
