use crate::types::{Routine, RoutineExecution, RoutineId, RunId, RunStatus};
use crate::workflow::executor::WorkflowExecutor;
use anyhow::Result;
use chrono::{DateTime, Datelike, Timelike, Utc};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::task::JoinHandle;

/// Cron scheduler for recurring routines
pub struct RoutineScheduler {
    routines: Arc<Mutex<HashMap<RoutineId, Routine>>>,
    executions: Arc<Mutex<Vec<RoutineExecution>>>,
    executor: Arc<WorkflowExecutor>,
    running_tasks: Arc<Mutex<HashMap<RoutineId, JoinHandle<()>>>>,
}

impl RoutineScheduler {
    /// Create a new routine scheduler
    pub fn new(executor: Arc<WorkflowExecutor>) -> Self {
        Self {
            routines: Arc::new(Mutex::new(HashMap::new())),
            executions: Arc::new(Mutex::new(Vec::new())),
            executor,
            running_tasks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a routine
    pub fn register_routine(&self, routine: Routine) -> Result<()> {
        let routine_id = routine.id.clone();

        // Store the routine
        self.routines.lock().unwrap().insert(routine_id.clone(), routine.clone());

        // Start the scheduler task for this routine
        if routine.enabled {
            self.start_routine_task(routine)?;
        }

        Ok(())
    }

    /// Unregister a routine
    pub fn unregister_routine(&self, routine_id: &RoutineId) -> Result<()> {
        // Stop the running task
        if let Some(handle) = self.running_tasks.lock().unwrap().remove(routine_id) {
            handle.abort();
        }

        // Remove the routine
        self.routines.lock().unwrap().remove(routine_id);

        Ok(())
    }

    /// Get all routines
    pub fn list_routines(&self) -> Vec<Routine> {
        self.routines.lock().unwrap().values().cloned().collect()
    }

    /// Get a specific routine
    pub fn get_routine(&self, routine_id: &RoutineId) -> Option<Routine> {
        self.routines.lock().unwrap().get(routine_id).cloned()
    }

    /// Enable a routine
    pub fn enable_routine(&self, routine_id: &RoutineId) -> Result<()> {
        let mut routines = self.routines.lock().unwrap();
        if let Some(routine) = routines.get_mut(routine_id) {
            routine.enabled = true;
            routine.updated_at = Utc::now();

            // Start the task
            let routine_clone = routine.clone();
            drop(routines);
            self.start_routine_task(routine_clone)?;
            Ok(())
        } else {
            Err(anyhow::anyhow!("Routine not found"))
        }
    }

    /// Disable a routine
    pub fn disable_routine(&self, routine_id: &RoutineId) -> Result<()> {
        // Stop the running task
        if let Some(handle) = self.running_tasks.lock().unwrap().remove(routine_id) {
            handle.abort();
        }

        // Mark as disabled
        let mut routines = self.routines.lock().unwrap();
        if let Some(routine) = routines.get_mut(routine_id) {
            routine.enabled = false;
            routine.updated_at = Utc::now();
            Ok(())
        } else {
            Err(anyhow::anyhow!("Routine not found"))
        }
    }

    /// Start a scheduler task for a routine
    fn start_routine_task(&self, routine: Routine) -> Result<()> {
        let routine_id = routine.id.clone();
        let routine_id_for_task = routine_id.clone();
        let executor = self.executor.clone();
        let executions = self.executions.clone();
        let routines = self.routines.clone();

        let handle = tokio::spawn(async move {
            loop {
                // Calculate next run time based on cron expression
                let next_run = match calculate_next_run(&routine.schedule.cron) {
                    Ok(next) => next,
                    Err(e) => {
                        tracing::error!("Failed to calculate next run for routine {}: {}", routine_id_for_task.0, e);
                        break;
                    }
                };

                // Update next_run in the routine
                {
                    let mut routines_lock = routines.lock().unwrap();
                    if let Some(r) = routines_lock.get_mut(&routine_id_for_task) {
                        r.next_run = next_run;
                    }
                }

                // Wait until next run time
                let now = Utc::now();
                if next_run > now {
                    let wait_duration = (next_run - now).to_std().unwrap_or(std::time::Duration::from_secs(1));
                    tokio::time::sleep(wait_duration).await;
                }

                // Execute the routine
                tracing::info!("Executing routine: {}", routine.name);
                let scheduled_at = next_run;
                let executed_at = Utc::now();

                match executor.execute(routine.id.0.clone(), routine.workflow.clone()).await {
                    Ok(run) => {
                        let execution = RoutineExecution {
                            id: uuid::Uuid::new_v4().to_string(),
                            routine_id: routine_id_for_task.clone(),
                            run_id: run.id,
                            scheduled_at,
                            executed_at,
                            status: RunStatus::Running,
                            error: None,
                        };

                        executions.lock().unwrap().push(execution);

                        // Update last_run
                        let mut routines_lock = routines.lock().unwrap();
                        if let Some(r) = routines_lock.get_mut(&routine_id_for_task) {
                            r.last_run = Some(executed_at);
                        }
                    }
                    Err(e) => {
                        tracing::error!("Failed to execute routine {}: {}", routine.name, e);
                        let execution = RoutineExecution {
                            id: uuid::Uuid::new_v4().to_string(),
                            routine_id: routine_id_for_task.clone(),
                            run_id: RunId::new(),
                            scheduled_at,
                            executed_at,
                            status: RunStatus::Failed,
                            error: Some(e.to_string()),
                        };
                        executions.lock().unwrap().push(execution);
                    }
                }

                // Check if routine is still enabled
                let enabled = {
                    let routines_lock = routines.lock().unwrap();
                    routines_lock.get(&routine_id_for_task).map(|r| r.enabled).unwrap_or(false)
                };

                if !enabled {
                    tracing::info!("Routine {} has been disabled, stopping scheduler", routine.name);
                    break;
                }
            }
        });

        self.running_tasks.lock().unwrap().insert(routine_id, handle);
        Ok(())
    }

    /// Get execution history for a routine
    pub fn get_executions(&self, routine_id: &RoutineId) -> Vec<RoutineExecution> {
        self.executions
            .lock()
            .unwrap()
            .iter()
            .filter(|e| e.routine_id == *routine_id)
            .cloned()
            .collect()
    }

    /// Get all executions
    pub fn list_executions(&self) -> Vec<RoutineExecution> {
        self.executions.lock().unwrap().clone()
    }
}

/// Calculate next run time from cron expression (simplified)
/// In production, use a proper cron parsing library like `cron` or `tokio-cron-scheduler`
fn calculate_next_run(cron_expr: &str) -> Result<DateTime<Utc>> {
    // For MVP, we'll use a simple parser
    // Cron format: minute hour day month weekday
    // Examples:
    // "0 0 * * *" = daily at midnight
    // "0 */6 * * *" = every 6 hours
    // "*/15 * * * *" = every 15 minutes

    let parts: Vec<&str> = cron_expr.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(anyhow::anyhow!("Invalid cron expression: must have 5 parts"));
    }

    // Simple implementation: parse minutes and hours
    let minute = parts[0];
    let hour = parts[1];

    let now = Utc::now();
    let mut next = now;

    // Handle */N syntax for minutes
    if let Some(interval) = minute.strip_prefix("*/") {
        let interval: i64 = interval.parse()?;
        let minutes_to_add = interval - (now.minute() as i64 % interval);
        next = next + chrono::Duration::minutes(minutes_to_add);
    } else if minute != "*" {
        let target_minute: u32 = minute.parse()?;
        next = next
            .with_minute(target_minute)
            .ok_or_else(|| anyhow::anyhow!("Invalid minute"))?
            .with_second(0)
            .ok_or_else(|| anyhow::anyhow!("Invalid second"))?
            .with_nanosecond(0)
            .ok_or_else(|| anyhow::anyhow!("Invalid nanosecond"))?;

        // If the target minute has passed in this hour, move to next hour
        if next <= now {
            next = next + chrono::Duration::hours(1);
        }
    }

    // Handle hour
    if hour != "*" {
        let target_hour: u32 = hour.parse()?;
        next = next
            .with_hour(target_hour)
            .ok_or_else(|| anyhow::anyhow!("Invalid hour"))?;

        // If the target hour has passed today, move to tomorrow
        if next <= now {
            next = next + chrono::Duration::days(1);
        }
    }

    // Final check: ensure next run is in the future
    if next <= now {
        next = next + chrono::Duration::minutes(1);
    }

    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::event_log::JsonlEventLog;
    use crate::storage::blob::FilesystemBlobStore;
    use crate::storage::index::RedbIndexStore;
    use crate::types::{RoleId, RoutineSchedule, StepAction, StepId, StepSpec, WorkflowSpec};
    use tempfile::TempDir;

    #[test]
    fn test_calculate_next_run() {
        let now = Utc::now();

        // Every 15 minutes
        let next = calculate_next_run("*/15 * * * *").unwrap();
        assert!(next >= now, "next run should be >= now");

        // Daily at midnight
        let next = calculate_next_run("0 0 * * *").unwrap();
        assert!(next >= now, "next run should be >= now");

        // Every hour
        let next = calculate_next_run("0 * * * *").unwrap();
        assert!(next >= now, "next run should be >= now");
    }

    #[tokio::test]
    async fn test_register_routine() {
        let temp_dir = TempDir::new().unwrap();
        let index_path = temp_dir.path().join("index.redb");
        let event_dir = temp_dir.path().join("events");
        let blob_dir = temp_dir.path().join("blobs");

        std::fs::create_dir_all(&event_dir).unwrap();
        std::fs::create_dir_all(&blob_dir).unwrap();

        let index_store = Arc::new(RedbIndexStore::new(index_path).unwrap());
        let event_log = Arc::new(JsonlEventLog::new(event_dir).unwrap());
        let blob_store = Arc::new(FilesystemBlobStore::new(blob_dir).unwrap());
        let executor = Arc::new(WorkflowExecutor::new(event_log, blob_store, index_store));
        let scheduler = RoutineScheduler::new(executor);

        let routine = Routine {
            id: RoutineId::new("test_routine"),
            name: "Test Routine".to_string(),
            description: "A test routine".to_string(),
            schedule: RoutineSchedule {
                cron: "*/15 * * * *".to_string(),
                timezone: "UTC".to_string(),
            },
            workflow: WorkflowSpec {
                steps: vec![StepSpec {
                    id: StepId::new("step1"),
                    name: "Test Step".to_string(),
                    description: Some("A test step".to_string()),
                    role: RoleId::new("test"),
                    action: StepAction::AgentTask {
                        prompt: "Test prompt".to_string(),
                    },
                    timeout_secs: Some(60),
                    retry_policy: None,
                    requires_approval: false,
                }],
                dependencies: HashMap::new(),
            },
            enabled: false, // Disabled to avoid actual execution
            last_run: None,
            next_run: Utc::now(),
            created_at: Utc::now(),
            created_by: "test".to_string(),
            updated_at: Utc::now(),
        };

        scheduler.register_routine(routine.clone()).unwrap();

        let retrieved = scheduler.get_routine(&routine.id).unwrap();
        assert_eq!(retrieved.id, routine.id);
        assert_eq!(retrieved.name, routine.name);
    }

    #[tokio::test]
    async fn test_enable_disable_routine() {
        let temp_dir = TempDir::new().unwrap();
        let index_path = temp_dir.path().join("index.redb");
        let event_dir = temp_dir.path().join("events");
        let blob_dir = temp_dir.path().join("blobs");

        std::fs::create_dir_all(&event_dir).unwrap();
        std::fs::create_dir_all(&blob_dir).unwrap();

        let index_store = Arc::new(RedbIndexStore::new(index_path).unwrap());
        let event_log = Arc::new(JsonlEventLog::new(event_dir).unwrap());
        let blob_store = Arc::new(FilesystemBlobStore::new(blob_dir).unwrap());
        let executor = Arc::new(WorkflowExecutor::new(event_log, blob_store, index_store));
        let scheduler = RoutineScheduler::new(executor);

        let routine = Routine {
            id: RoutineId::new("test_routine"),
            name: "Test Routine".to_string(),
            description: "A test routine".to_string(),
            schedule: RoutineSchedule {
                cron: "*/15 * * * *".to_string(),
                timezone: "UTC".to_string(),
            },
            workflow: WorkflowSpec {
                steps: vec![],
                dependencies: HashMap::new(),
            },
            enabled: false,
            last_run: None,
            next_run: Utc::now(),
            created_at: Utc::now(),
            created_by: "test".to_string(),
            updated_at: Utc::now(),
        };

        scheduler.register_routine(routine.clone()).unwrap();

        // Enable
        scheduler.enable_routine(&routine.id).unwrap();
        let retrieved = scheduler.get_routine(&routine.id).unwrap();
        assert!(retrieved.enabled);

        // Disable
        scheduler.disable_routine(&routine.id).unwrap();
        let retrieved = scheduler.get_routine(&routine.id).unwrap();
        assert!(!retrieved.enabled);
    }
}
