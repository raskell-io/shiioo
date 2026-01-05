use super::dag::WorkflowDag;
use super::step_executor::StepExecutor;
use crate::events::{Event, EventLog, EventType};
use crate::storage::{BlobStore, IndexStore};
use crate::types::{Run, RunId, RunStatus, StepExecution, StepId, StepStatus, WorkflowSpec};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Workflow executor that coordinates DAG execution
pub struct WorkflowExecutor {
    event_log: Arc<dyn EventLog>,
    blob_store: Arc<dyn BlobStore>,
    index_store: Arc<dyn IndexStore>,
    step_executor: Arc<StepExecutor>,
    // Track active runs for cancellation
    active_runs: Arc<RwLock<HashMap<RunId, tokio::sync::watch::Sender<bool>>>>,
}

impl WorkflowExecutor {
    pub fn new(
        event_log: Arc<dyn EventLog>,
        blob_store: Arc<dyn BlobStore>,
        index_store: Arc<dyn IndexStore>,
    ) -> Self {
        let step_executor = Arc::new(StepExecutor::new(
            event_log.clone(),
            blob_store.clone(),
        ));

        Self {
            event_log,
            blob_store,
            index_store,
            step_executor,
            active_runs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Execute a workflow and return the run
    pub async fn execute(&self, work_item_id: String, workflow: WorkflowSpec) -> Result<Run> {
        let run_id = RunId::new();
        let started_at = chrono::Utc::now();

        tracing::info!("Starting workflow execution: run_id={}", run_id);

        // Create cancellation channel
        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        self.active_runs.write().await.insert(run_id, cancel_tx);

        // Build DAG
        let dag = WorkflowDag::from_workflow(&workflow).context("Failed to build DAG")?;

        // Initialize run state
        let mut run = Run {
            id: run_id,
            work_item_id: work_item_id.clone(),
            status: RunStatus::Running,
            started_at,
            completed_at: None,
            steps: workflow
                .steps
                .iter()
                .map(|s| StepExecution {
                    id: s.id.clone(),
                    status: StepStatus::Pending,
                    started_at: None,
                    completed_at: None,
                    attempt: 0,
                    error: None,
                })
                .collect(),
        };

        // Emit RunStarted event
        self.event_log
            .append(Event::new(
                run_id,
                EventType::RunStarted {
                    work_item_id,
                    workflow_spec: workflow.clone(),
                },
            ))
            .await?;

        // Index the run
        self.index_store.index_run(&run)?;

        // Execute the workflow
        let result = self
            .execute_dag(run_id, &dag, &workflow, cancel_rx)
            .await;

        // Update run status
        let duration = started_at.elapsed_seconds_from(chrono::Utc::now());
        let completed_at = chrono::Utc::now();
        run.completed_at = Some(completed_at);

        match result {
            Ok(steps) => {
                run.status = RunStatus::Completed;
                run.steps = steps;

                self.event_log
                    .append(Event::new(
                        run_id,
                        EventType::RunCompleted {
                            duration_secs: duration as u64,
                        },
                    ))
                    .await?;

                tracing::info!("Workflow execution completed: run_id={}", run_id);
            }
            Err(e) => {
                run.status = RunStatus::Failed;

                self.event_log
                    .append(Event::new(
                        run_id,
                        EventType::RunFailed {
                            error: e.to_string(),
                            duration_secs: duration as u64,
                        },
                    ))
                    .await?;

                tracing::error!("Workflow execution failed: run_id={}, error={}", run_id, e);
            }
        }

        // Update index
        self.index_store.index_run(&run)?;

        // Clean up active runs
        self.active_runs.write().await.remove(&run_id);

        Ok(run)
    }

    /// Execute the DAG
    async fn execute_dag(
        &self,
        run_id: RunId,
        dag: &WorkflowDag,
        workflow: &WorkflowSpec,
        mut cancel_rx: tokio::sync::watch::Receiver<bool>,
    ) -> Result<Vec<StepExecution>> {
        let mut completed_steps: HashSet<StepId> = HashSet::new();
        let mut failed_steps: HashSet<StepId> = HashSet::new();
        let mut step_executions: HashMap<StepId, StepExecution> = HashMap::new();

        // Emit StepScheduled events for all steps
        for step in &workflow.steps {
            self.event_log
                .append(Event::new(
                    run_id,
                    EventType::StepScheduled {
                        step_id: step.id.clone(),
                        step_spec: step.clone(),
                    },
                ))
                .await?;

            step_executions.insert(
                step.id.clone(),
                StepExecution {
                    id: step.id.clone(),
                    status: StepStatus::Pending,
                    started_at: None,
                    completed_at: None,
                    attempt: 0,
                    error: None,
                },
            );
        }

        // Get topological order
        let topo_order = dag.topological_order();

        // Execute steps in order, respecting dependencies
        for step in topo_order {
            // Check for cancellation
            if *cancel_rx.borrow() {
                tracing::warn!("Workflow execution cancelled: run_id={}", run_id);
                return Err(anyhow::anyhow!("Workflow cancelled"));
            }

            // Skip if dependencies failed
            let deps = dag.dependencies(&step.id)?;
            if deps.iter().any(|d| failed_steps.contains(d)) {
                tracing::info!(
                    "Skipping step {} due to failed dependencies",
                    step.id
                );

                self.event_log
                    .append(Event::new(
                        run_id,
                        EventType::StepSkipped {
                            step_id: step.id.clone(),
                            reason: "Dependency failed".to_string(),
                        },
                    ))
                    .await?;

                if let Some(exec) = step_executions.get_mut(&step.id) {
                    exec.status = StepStatus::Skipped;
                }

                continue;
            }

            // Execute the step
            tracing::info!("Executing step: {}", step.id);

            let started_at = chrono::Utc::now();
            let result = self.step_executor.execute(run_id, &step, 1).await?;
            let completed_at = chrono::Utc::now();

            // Update execution state
            if let Some(exec) = step_executions.get_mut(&step.id) {
                exec.status = result.status;
                exec.started_at = Some(started_at);
                exec.completed_at = Some(completed_at);
                exec.attempt = 1;
                exec.error = result.error.clone();
            }

            match result.status {
                StepStatus::Completed => {
                    completed_steps.insert(step.id.clone());
                }
                StepStatus::Failed => {
                    failed_steps.insert(step.id.clone());
                    // For now, fail the entire workflow on any step failure
                    // In the future, we could make this configurable
                    return Err(anyhow::anyhow!(
                        "Step {} failed: {}",
                        step.id,
                        result.error.unwrap_or_else(|| "Unknown error".to_string())
                    ));
                }
                _ => {}
            }
        }

        // Convert executions to Vec
        let mut executions: Vec<StepExecution> = step_executions.into_values().collect();
        executions.sort_by(|a, b| a.id.0.cmp(&b.id.0));

        Ok(executions)
    }

    /// Get the status of a running workflow
    pub async fn get_run(&self, run_id: RunId) -> Result<Option<Run>> {
        self.index_store.get_run(&run_id)
    }

    /// Cancel a running workflow
    pub async fn cancel(&self, run_id: RunId) -> Result<()> {
        let active_runs = self.active_runs.read().await;

        if let Some(cancel_tx) = active_runs.get(&run_id) {
            cancel_tx.send(true).ok();
            tracing::info!("Cancellation signal sent for run {}", run_id);

            // Emit cancellation event
            self.event_log
                .append(Event::new(
                    run_id,
                    EventType::RunCancelled {
                        reason: "User requested cancellation".to_string(),
                    },
                ))
                .await?;

            Ok(())
        } else {
            Err(anyhow::anyhow!("Run {} is not active", run_id))
        }
    }
}

// Helper trait for duration calculation
trait ElapsedSeconds {
    fn elapsed_seconds_from(&self, other: chrono::DateTime<chrono::Utc>) -> i64;
}

impl ElapsedSeconds for chrono::DateTime<chrono::Utc> {
    fn elapsed_seconds_from(&self, other: chrono::DateTime<chrono::Utc>) -> i64 {
        (other - *self).num_seconds()
    }
}
