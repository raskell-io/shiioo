use crate::events::{Event, EventLog, EventType, MessageDirection};
use crate::storage::BlobStore;
use crate::types::{BlobHash, RunId, StepAction, StepId, StepSpec, StepStatus};
use anyhow::{anyhow, Result};
use bytes::Bytes;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

/// Result of executing a step
#[derive(Debug, Clone)]
pub struct StepResult {
    pub status: StepStatus,
    pub error: Option<String>,
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, Clone)]
pub struct Artifact {
    pub artifact_type: String,
    pub content_hash: BlobHash,
    pub metadata: serde_json::Value,
}

/// Step executor with retry and timeout logic
pub struct StepExecutor {
    event_log: Arc<dyn EventLog>,
    blob_store: Arc<dyn BlobStore>,
}

impl StepExecutor {
    pub fn new(event_log: Arc<dyn EventLog>, blob_store: Arc<dyn BlobStore>) -> Self {
        Self {
            event_log,
            blob_store,
        }
    }

    /// Execute a step with retry and timeout logic
    pub async fn execute(
        &self,
        run_id: RunId,
        step: &StepSpec,
        attempt: u32,
    ) -> Result<StepResult> {
        tracing::info!(
            "Executing step {} (attempt {}) for run {}",
            step.id,
            attempt,
            run_id
        );

        // Emit StepStarted event
        self.event_log
            .append(Event::new(
                run_id,
                EventType::StepStarted {
                    step_id: step.id.clone(),
                    attempt,
                },
            ))
            .await?;

        let start = std::time::Instant::now();

        // Execute with timeout if configured
        let result = if let Some(timeout_secs) = step.timeout_secs {
            match timeout(
                Duration::from_secs(timeout_secs),
                self.execute_action(run_id, step, attempt),
            )
            .await
            {
                Ok(result) => result,
                Err(_) => {
                    let error = format!("Step timed out after {} seconds", timeout_secs);
                    tracing::warn!("Step {} timed out", step.id);
                    Err(anyhow!(error))
                }
            }
        } else {
            self.execute_action(run_id, step, attempt).await
        };

        let duration = start.elapsed();

        // Handle result and emit appropriate event
        match result {
            Ok(step_result) => {
                self.event_log
                    .append(Event::new(
                        run_id,
                        EventType::StepCompleted {
                            step_id: step.id.clone(),
                            duration_secs: duration.as_secs(),
                        },
                    ))
                    .await?;

                // Emit artifact events
                for artifact in &step_result.artifacts {
                    self.event_log
                        .append(Event::new(
                            run_id,
                            EventType::ArtifactProduced {
                                step_id: step.id.clone(),
                                artifact_type: artifact.artifact_type.clone(),
                                content_hash: artifact.content_hash.clone(),
                                metadata: artifact.metadata.clone(),
                            },
                        ))
                        .await?;
                }

                Ok(step_result)
            }
            Err(e) => {
                let error_msg = e.to_string();
                let will_retry = self.should_retry(step, attempt);

                self.event_log
                    .append(Event::new(
                        run_id,
                        EventType::StepFailed {
                            step_id: step.id.clone(),
                            error: error_msg.clone(),
                            attempt,
                            will_retry,
                        },
                    ))
                    .await?;

                if will_retry {
                    // Wait before retry (exponential backoff)
                    let backoff = step
                        .retry_policy
                        .as_ref()
                        .map(|p| p.backoff_secs)
                        .unwrap_or(1);
                    let backoff_duration = Duration::from_secs(backoff * 2_u64.pow(attempt - 1));

                    tracing::info!(
                        "Retrying step {} after {:?} (attempt {})",
                        step.id,
                        backoff_duration,
                        attempt + 1
                    );

                    tokio::time::sleep(backoff_duration).await;

                    // Retry (boxed to avoid infinite recursion size)
                    return Box::pin(self.execute(run_id, step, attempt + 1)).await;
                }

                Ok(StepResult {
                    status: StepStatus::Failed,
                    error: Some(error_msg),
                    artifacts: vec![],
                })
            }
        }
    }

    /// Execute the actual step action
    async fn execute_action(
        &self,
        run_id: RunId,
        step: &StepSpec,
        _attempt: u32,
    ) -> Result<StepResult> {
        match &step.action {
            StepAction::AgentTask { prompt } => {
                self.execute_agent_task(run_id, &step.id, prompt).await
            }
            StepAction::ToolSequence { tools } => {
                self.execute_tool_sequence(run_id, &step.id, tools).await
            }
            StepAction::ManualApproval { approvers } => {
                self.execute_manual_approval(run_id, &step.id, approvers).await
            }
            StepAction::Script { command, args } => {
                self.execute_script(run_id, &step.id, command, args).await
            }
        }
    }

    /// Execute an agent task (stub for now - will integrate with MCP in Phase 2)
    async fn execute_agent_task(
        &self,
        run_id: RunId,
        step_id: &StepId,
        prompt: &str,
    ) -> Result<StepResult> {
        // Store prompt as blob
        let prompt_bytes = Bytes::from(prompt.to_string());
        let prompt_hash = self.blob_store.put(prompt_bytes).await?;

        // Emit agent message event
        self.event_log
            .append(Event::new(
                run_id,
                EventType::AgentMessage {
                    step_id: step_id.clone(),
                    direction: MessageDirection::ToAgent,
                    content_hash: prompt_hash,
                    tokens: None,
                },
            ))
            .await?;

        // For MVP: simulate agent response
        let response = format!("Agent response to: {}", prompt);
        let response_bytes = Bytes::from(response);
        let response_hash = self.blob_store.put(response_bytes).await?;

        // Emit response event
        self.event_log
            .append(Event::new(
                run_id,
                EventType::AgentMessage {
                    step_id: step_id.clone(),
                    direction: MessageDirection::FromAgent,
                    content_hash: response_hash.clone(),
                    tokens: Some(100), // Simulated
                },
            ))
            .await?;

        Ok(StepResult {
            status: StepStatus::Completed,
            error: None,
            artifacts: vec![Artifact {
                artifact_type: "agent_response".to_string(),
                content_hash: response_hash,
                metadata: serde_json::json!({
                    "tokens": 100,
                }),
            }],
        })
    }

    /// Execute a sequence of tool calls (stub for Phase 2)
    async fn execute_tool_sequence(
        &self,
        _run_id: RunId,
        _step_id: &StepId,
        tools: &[crate::types::ToolCallSpec],
    ) -> Result<StepResult> {
        // For MVP: just log that we would execute tools
        tracing::info!("Would execute {} tool calls", tools.len());

        Ok(StepResult {
            status: StepStatus::Completed,
            error: None,
            artifacts: vec![],
        })
    }

    /// Execute manual approval (stub for Phase 2)
    async fn execute_manual_approval(
        &self,
        run_id: RunId,
        step_id: &StepId,
        approvers: &[String],
    ) -> Result<StepResult> {
        // Emit approval request event
        self.event_log
            .append(Event::new(
                run_id,
                EventType::ApprovalRequested {
                    step_id: step_id.clone(),
                    approvers: approvers.to_vec(),
                    context: "Manual approval required".to_string(),
                },
            ))
            .await?;

        // For MVP: auto-approve
        tracing::info!("Auto-approving step {} (MVP mode)", step_id);

        self.event_log
            .append(Event::new(
                run_id,
                EventType::ApprovalGranted {
                    step_id: step_id.clone(),
                    approved_by: "system".to_string(),
                    comment: Some("Auto-approved in MVP mode".to_string()),
                },
            ))
            .await?;

        Ok(StepResult {
            status: StepStatus::Completed,
            error: None,
            artifacts: vec![],
        })
    }

    /// Execute a script (stub for Phase 2)
    async fn execute_script(
        &self,
        _run_id: RunId,
        _step_id: &StepId,
        command: &str,
        args: &[String],
    ) -> Result<StepResult> {
        // For MVP: just log
        tracing::info!("Would execute script: {} {:?}", command, args);

        Ok(StepResult {
            status: StepStatus::Completed,
            error: None,
            artifacts: vec![],
        })
    }

    /// Check if we should retry a failed step
    fn should_retry(&self, step: &StepSpec, attempt: u32) -> bool {
        if let Some(retry_policy) = &step.retry_policy {
            attempt < retry_policy.max_attempts
        } else {
            false
        }
    }
}
