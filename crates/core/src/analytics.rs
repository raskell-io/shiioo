use crate::types::{RunId, StepId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Performance analytics for workflows and steps
pub struct PerformanceAnalytics {
    workflow_stats: Arc<Mutex<HashMap<String, WorkflowStats>>>,
    step_stats: Arc<Mutex<HashMap<String, StepStats>>>,
    execution_traces: Arc<Mutex<Vec<ExecutionTrace>>>,
}

/// Statistics for a workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStats {
    pub workflow_id: String,
    pub execution_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub total_duration_secs: f64,
    pub min_duration_secs: f64,
    pub max_duration_secs: f64,
    pub avg_duration_secs: f64,
    pub last_execution: Option<DateTime<Utc>>,
}

/// Statistics for a step
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepStats {
    pub step_id: String,
    pub execution_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub retry_count: u64,
    pub total_duration_secs: f64,
    pub min_duration_secs: f64,
    pub max_duration_secs: f64,
    pub avg_duration_secs: f64,
    pub p50_duration_secs: Option<f64>,
    pub p95_duration_secs: Option<f64>,
    pub p99_duration_secs: Option<f64>,
    pub durations: Vec<f64>, // Store for percentile calculation
}

/// Execution trace for a single workflow run
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionTrace {
    pub run_id: RunId,
    pub workflow_id: String,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<f64>,
    pub status: TraceStatus,
    pub steps: Vec<StepTrace>,
    pub bottleneck: Option<BottleneckInfo>,
}

/// Status of an execution trace
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Trace for a single step execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepTrace {
    pub step_id: StepId,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<f64>,
    pub status: TraceStatus,
    pub attempt: u32,
    pub error: Option<String>,
}

/// Information about detected bottlenecks
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BottleneckInfo {
    pub step_id: StepId,
    pub duration_secs: f64,
    pub percentage_of_total: f64,
}

/// Bottleneck detection result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BottleneckReport {
    pub workflow_id: String,
    pub total_executions: u64,
    pub avg_duration_secs: f64,
    pub bottlenecks: Vec<BottleneckStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BottleneckStep {
    pub step_id: String,
    pub avg_duration_secs: f64,
    pub percentage_of_workflow: f64,
    pub execution_count: u64,
}

impl PerformanceAnalytics {
    /// Create a new performance analytics instance
    pub fn new() -> Self {
        Self {
            workflow_stats: Arc::new(Mutex::new(HashMap::new())),
            step_stats: Arc::new(Mutex::new(HashMap::new())),
            execution_traces: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Start tracking a workflow execution
    pub fn start_workflow(&self, run_id: RunId, workflow_id: String) {
        let mut traces = self.execution_traces.lock().unwrap();
        traces.push(ExecutionTrace {
            run_id,
            workflow_id,
            started_at: Utc::now(),
            completed_at: None,
            duration_secs: None,
            status: TraceStatus::Running,
            steps: Vec::new(),
            bottleneck: None,
        });
    }

    /// Start tracking a step execution
    pub fn start_step(&self, run_id: &RunId, step_id: StepId, attempt: u32) {
        let mut traces = self.execution_traces.lock().unwrap();
        if let Some(trace) = traces.iter_mut().find(|t| &t.run_id == run_id) {
            trace.steps.push(StepTrace {
                step_id,
                started_at: Utc::now(),
                completed_at: None,
                duration_secs: None,
                status: TraceStatus::Running,
                attempt,
                error: None,
            });
        }
    }

    /// Complete a step execution
    pub fn complete_step(&self, run_id: &RunId, step_id: &StepId, success: bool, error: Option<String>) {
        let mut traces = self.execution_traces.lock().unwrap();
        if let Some(trace) = traces.iter_mut().find(|t| &t.run_id == run_id) {
            if let Some(step) = trace.steps.iter_mut().rev().find(|s| &s.step_id == step_id) {
                let now = Utc::now();
                let duration = (now - step.started_at).num_milliseconds() as f64 / 1000.0;

                step.completed_at = Some(now);
                step.duration_secs = Some(duration);
                step.status = if success { TraceStatus::Completed } else { TraceStatus::Failed };
                step.error = error.clone();

                // Update step statistics
                let mut stats = self.step_stats.lock().unwrap();
                stats
                    .entry(step_id.0.clone())
                    .and_modify(|s| {
                        s.execution_count += 1;
                        if success {
                            s.success_count += 1;
                        } else {
                            s.failure_count += 1;
                        }
                        if step.attempt > 0 {
                            s.retry_count += 1;
                        }
                        s.total_duration_secs += duration;
                        s.min_duration_secs = s.min_duration_secs.min(duration);
                        s.max_duration_secs = s.max_duration_secs.max(duration);
                        s.avg_duration_secs = s.total_duration_secs / s.execution_count as f64;
                        s.durations.push(duration);
                        s.durations.sort_by(|a, b| a.partial_cmp(b).unwrap());

                        // Calculate percentiles
                        s.p50_duration_secs = Self::calculate_percentile(&s.durations, 0.50);
                        s.p95_duration_secs = Self::calculate_percentile(&s.durations, 0.95);
                        s.p99_duration_secs = Self::calculate_percentile(&s.durations, 0.99);
                    })
                    .or_insert_with(|| StepStats {
                        step_id: step_id.0.clone(),
                        execution_count: 1,
                        success_count: if success { 1 } else { 0 },
                        failure_count: if success { 0 } else { 1 },
                        retry_count: if step.attempt > 0 { 1 } else { 0 },
                        total_duration_secs: duration,
                        min_duration_secs: duration,
                        max_duration_secs: duration,
                        avg_duration_secs: duration,
                        p50_duration_secs: Some(duration),
                        p95_duration_secs: Some(duration),
                        p99_duration_secs: Some(duration),
                        durations: vec![duration],
                    });
            }
        }
    }

    /// Complete a workflow execution
    pub fn complete_workflow(&self, run_id: &RunId, success: bool) {
        let mut traces = self.execution_traces.lock().unwrap();
        if let Some(trace) = traces.iter_mut().find(|t| &t.run_id == run_id) {
            let now = Utc::now();
            let duration = (now - trace.started_at).num_milliseconds() as f64 / 1000.0;

            trace.completed_at = Some(now);
            trace.duration_secs = Some(duration);
            trace.status = if success { TraceStatus::Completed } else { TraceStatus::Failed };

            // Detect bottleneck
            if let Some(slowest_step) = trace.steps.iter().max_by(|a, b| {
                a.duration_secs
                    .unwrap_or(0.0)
                    .partial_cmp(&b.duration_secs.unwrap_or(0.0))
                    .unwrap()
            }) {
                if let Some(step_duration) = slowest_step.duration_secs {
                    let percentage = (step_duration / duration) * 100.0;
                    trace.bottleneck = Some(BottleneckInfo {
                        step_id: slowest_step.step_id.clone(),
                        duration_secs: step_duration,
                        percentage_of_total: percentage,
                    });
                }
            }

            // Update workflow statistics
            let mut stats = self.workflow_stats.lock().unwrap();
            stats
                .entry(trace.workflow_id.clone())
                .and_modify(|s| {
                    s.execution_count += 1;
                    if success {
                        s.success_count += 1;
                    } else {
                        s.failure_count += 1;
                    }
                    s.total_duration_secs += duration;
                    s.min_duration_secs = s.min_duration_secs.min(duration);
                    s.max_duration_secs = s.max_duration_secs.max(duration);
                    s.avg_duration_secs = s.total_duration_secs / s.execution_count as f64;
                    s.last_execution = Some(now);
                })
                .or_insert_with(|| WorkflowStats {
                    workflow_id: trace.workflow_id.clone(),
                    execution_count: 1,
                    success_count: if success { 1 } else { 0 },
                    failure_count: if success { 0 } else { 1 },
                    total_duration_secs: duration,
                    min_duration_secs: duration,
                    max_duration_secs: duration,
                    avg_duration_secs: duration,
                    last_execution: Some(now),
                });
        }
    }

    /// Get workflow statistics
    pub fn get_workflow_stats(&self, workflow_id: &str) -> Option<WorkflowStats> {
        self.workflow_stats.lock().unwrap().get(workflow_id).cloned()
    }

    /// Get all workflow statistics
    pub fn get_all_workflow_stats(&self) -> Vec<WorkflowStats> {
        self.workflow_stats.lock().unwrap().values().cloned().collect()
    }

    /// Get step statistics
    pub fn get_step_stats(&self, step_id: &str) -> Option<StepStats> {
        self.step_stats.lock().unwrap().get(step_id).cloned()
    }

    /// Get all step statistics
    pub fn get_all_step_stats(&self) -> Vec<StepStats> {
        self.step_stats.lock().unwrap().values().cloned().collect()
    }

    /// Get execution trace for a run
    pub fn get_trace(&self, run_id: &RunId) -> Option<ExecutionTrace> {
        self.execution_traces
            .lock()
            .unwrap()
            .iter()
            .find(|t| &t.run_id == run_id)
            .cloned()
    }

    /// Get all execution traces
    pub fn get_all_traces(&self) -> Vec<ExecutionTrace> {
        self.execution_traces.lock().unwrap().clone()
    }

    /// Get recent execution traces (last N)
    pub fn get_recent_traces(&self, limit: usize) -> Vec<ExecutionTrace> {
        let traces = self.execution_traces.lock().unwrap();
        traces.iter().rev().take(limit).cloned().collect()
    }

    /// Detect bottlenecks in a workflow
    pub fn detect_bottlenecks(&self, workflow_id: &str) -> Option<BottleneckReport> {
        let workflow_stats = self.get_workflow_stats(workflow_id)?;

        // Get all step stats for steps in this workflow
        let step_stats = self.step_stats.lock().unwrap();

        // Find steps that take the most time
        let mut bottlenecks: Vec<BottleneckStep> = step_stats
            .values()
            .filter(|s| s.execution_count > 0)
            .map(|s| {
                let percentage = (s.avg_duration_secs / workflow_stats.avg_duration_secs) * 100.0;
                BottleneckStep {
                    step_id: s.step_id.clone(),
                    avg_duration_secs: s.avg_duration_secs,
                    percentage_of_workflow: percentage,
                    execution_count: s.execution_count,
                }
            })
            .collect();

        // Sort by percentage of workflow time
        bottlenecks.sort_by(|a, b| b.percentage_of_workflow.partial_cmp(&a.percentage_of_workflow).unwrap());

        Some(BottleneckReport {
            workflow_id: workflow_id.to_string(),
            total_executions: workflow_stats.execution_count,
            avg_duration_secs: workflow_stats.avg_duration_secs,
            bottlenecks,
        })
    }

    /// Calculate percentile from sorted durations
    fn calculate_percentile(sorted_durations: &[f64], p: f64) -> Option<f64> {
        if sorted_durations.is_empty() {
            return None;
        }

        let index = ((sorted_durations.len() as f64) * p).ceil() as usize;
        let index = index.min(sorted_durations.len() - 1);
        Some(sorted_durations[index])
    }
}

impl Default for PerformanceAnalytics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_workflow_tracking() {
        let analytics = PerformanceAnalytics::new();
        let run_id = RunId::new();

        analytics.start_workflow(run_id.clone(), "test_workflow".to_string());
        analytics.complete_workflow(&run_id, true);

        let stats = analytics.get_workflow_stats("test_workflow").unwrap();
        assert_eq!(stats.execution_count, 1);
        assert_eq!(stats.success_count, 1);
        assert_eq!(stats.failure_count, 0);
    }

    #[test]
    fn test_step_tracking() {
        let analytics = PerformanceAnalytics::new();
        let run_id = RunId::new();
        let step_id = StepId::new("test_step");

        analytics.start_workflow(run_id.clone(), "workflow".to_string());
        analytics.start_step(&run_id, step_id.clone(), 0);

        std::thread::sleep(std::time::Duration::from_millis(100));

        analytics.complete_step(&run_id, &step_id, true, None);

        let stats = analytics.get_step_stats("test_step").unwrap();
        assert_eq!(stats.execution_count, 1);
        assert_eq!(stats.success_count, 1);
        assert!(stats.avg_duration_secs > 0.0);
    }

    #[test]
    fn test_retry_tracking() {
        let analytics = PerformanceAnalytics::new();
        let run_id = RunId::new();
        let step_id = StepId::new("flaky_step");

        analytics.start_workflow(run_id.clone(), "workflow".to_string());

        // First attempt fails
        analytics.start_step(&run_id, step_id.clone(), 0);
        analytics.complete_step(&run_id, &step_id, false, Some("Error".to_string()));

        // Second attempt succeeds
        analytics.start_step(&run_id, step_id.clone(), 1);
        analytics.complete_step(&run_id, &step_id, true, None);

        let stats = analytics.get_step_stats("flaky_step").unwrap();
        assert_eq!(stats.execution_count, 2);
        assert_eq!(stats.success_count, 1);
        assert_eq!(stats.failure_count, 1);
        assert_eq!(stats.retry_count, 1);
    }

    #[test]
    fn test_bottleneck_detection() {
        let analytics = PerformanceAnalytics::new();
        let run_id = RunId::new();

        analytics.start_workflow(run_id.clone(), "slow_workflow".to_string());

        let fast_step = StepId::new("fast_step");
        analytics.start_step(&run_id, fast_step.clone(), 0);
        std::thread::sleep(std::time::Duration::from_millis(10));
        analytics.complete_step(&run_id, &fast_step, true, None);

        let slow_step = StepId::new("slow_step");
        analytics.start_step(&run_id, slow_step.clone(), 0);
        std::thread::sleep(std::time::Duration::from_millis(100));
        analytics.complete_step(&run_id, &slow_step, true, None);

        analytics.complete_workflow(&run_id, true);

        let trace = analytics.get_trace(&run_id).unwrap();
        assert!(trace.bottleneck.is_some());

        let bottleneck = trace.bottleneck.unwrap();
        assert_eq!(bottleneck.step_id.0, "slow_step");
    }

    #[test]
    fn test_percentile_calculation() {
        let analytics = PerformanceAnalytics::new();
        let run_id = RunId::new();
        let step_id = StepId::new("test_step");

        analytics.start_workflow(run_id.clone(), "workflow".to_string());

        // Add multiple executions with different durations
        for _ in 0..10 {
            analytics.start_step(&run_id, step_id.clone(), 0);
            std::thread::sleep(std::time::Duration::from_millis(10));
            analytics.complete_step(&run_id, &step_id, true, None);
        }

        let stats = analytics.get_step_stats("test_step").unwrap();
        assert!(stats.p50_duration_secs.is_some());
        assert!(stats.p95_duration_secs.is_some());
        assert!(stats.p99_duration_secs.is_some());
    }
}
