use crate::types::*;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An event in the system's event log
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: String,
    pub run_id: RunId,
    pub timestamp: DateTime<Utc>,
    pub event_type: EventType,
}

impl Event {
    pub fn new(run_id: RunId, event_type: EventType) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            run_id,
            timestamp: Utc::now(),
            event_type,
        }
    }
}

/// Types of events that can occur in the system
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EventType {
    // Run lifecycle events
    RunStarted {
        work_item_id: String,
        workflow_spec: WorkflowSpec,
    },
    RunCompleted {
        duration_secs: u64,
    },
    RunFailed {
        error: String,
        duration_secs: u64,
    },
    RunCancelled {
        reason: String,
    },

    // Step lifecycle events
    StepScheduled {
        step_id: StepId,
        step_spec: StepSpec,
    },
    StepStarted {
        step_id: StepId,
        attempt: u32,
    },
    StepCompleted {
        step_id: StepId,
        duration_secs: u64,
    },
    StepFailed {
        step_id: StepId,
        error: String,
        attempt: u32,
        will_retry: bool,
    },
    StepSkipped {
        step_id: StepId,
        reason: String,
    },

    // Agent interaction events
    AgentMessage {
        step_id: StepId,
        direction: MessageDirection,
        content_hash: BlobHash,
        tokens: Option<u64>,
    },

    // Tool call events
    ToolCallProposed {
        step_id: StepId,
        tool_id: String,
        parameters_hash: BlobHash,
    },
    ToolCallApproved {
        step_id: StepId,
        tool_id: String,
        approved_by: String,
    },
    ToolCallDenied {
        step_id: StepId,
        tool_id: String,
        denied_by: String,
        reason: String,
    },
    ToolCallExecuted {
        step_id: StepId,
        tool_id: String,
        result_hash: BlobHash,
        duration_ms: u64,
    },

    // Approval events
    ApprovalRequested {
        step_id: StepId,
        approvers: Vec<String>,
        context: String,
    },
    ApprovalGranted {
        step_id: StepId,
        approved_by: String,
        comment: Option<String>,
    },
    ApprovalRejected {
        step_id: StepId,
        rejected_by: String,
        reason: String,
    },

    // Artifact events
    ArtifactProduced {
        step_id: StepId,
        artifact_type: String,
        content_hash: BlobHash,
        metadata: serde_json::Value,
    },

    // Configuration change events
    ConfigProposalCreated {
        proposal_id: String,
        proposed_by: String,
        diff_hash: BlobHash,
    },
    ConfigDiffGenerated {
        proposal_id: String,
        diff_hash: BlobHash,
    },
    ConfigApplied {
        proposal_id: String,
        applied_by: String,
        previous_config_hash: BlobHash,
        new_config_hash: BlobHash,
    },
    ConfigRolledBack {
        reason: String,
        rolled_back_to_hash: BlobHash,
    },

    // Capacity events
    CapacitySourceUsed {
        step_id: StepId,
        source_id: String,
        model: String,
        tokens: u64,
        cost_cents: Option<u64>,
    },
    CapacitySourceThrottled {
        source_id: String,
        reason: String,
        retry_after_secs: Option<u64>,
    },
}

/// Direction of agent message
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageDirection {
    ToAgent,
    FromAgent,
}

/// Event log writer trait
#[async_trait::async_trait]
pub trait EventLog: Send + Sync {
    /// Append an event to the log
    async fn append(&self, event: Event) -> anyhow::Result<()>;

    /// Get all events for a run
    async fn get_run_events(&self, run_id: RunId) -> anyhow::Result<Vec<Event>>;

    /// Get events for a run within a time range
    async fn get_run_events_range(
        &self,
        run_id: RunId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> anyhow::Result<Vec<Event>>;
}
