//! Agent Orchestration - multi-agent coordination and supervision.
//!
//! This module provides:
//! - Multi-agent coordination patterns (teams, hierarchies)
//! - Agent-to-agent delegation with policy propagation
//! - Supervision trees for fault tolerance
//! - Workflow integration (agents as workflow step executors)

use crate::agent::{
    runtime::{AgentRuntime, AgentTask, BudgetConsumed, ExecutionContext, McpResolver, TaskResult, ToolExecutor},
    policy_engine::AgentPolicyEngine,
    Agent, AgentId, AgentPolicies, AgentStatus,
};
use crate::events::EventLog;
use crate::storage::AgentStore;
use crate::types::{RunId, StepId};
use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Configuration for agent orchestration.
#[derive(Debug, Clone)]
pub struct OrchestrationConfig {
    /// Maximum delegation depth (to prevent infinite delegation).
    pub max_delegation_depth: usize,
    /// Maximum concurrent agents in a team.
    pub max_concurrent_agents: usize,
    /// Timeout for delegation responses (seconds).
    pub delegation_timeout_secs: u64,
    /// Whether to allow cross-team delegation.
    pub allow_cross_team_delegation: bool,
    /// Supervision restart policy.
    pub supervision_restart_policy: SupervisionRestartPolicy,
}

impl Default for OrchestrationConfig {
    fn default() -> Self {
        Self {
            max_delegation_depth: 5,
            max_concurrent_agents: 10,
            delegation_timeout_secs: 300,
            allow_cross_team_delegation: true,
            supervision_restart_policy: SupervisionRestartPolicy::default(),
        }
    }
}

/// Supervision restart policy.
#[derive(Debug, Clone)]
pub struct SupervisionRestartPolicy {
    /// Maximum restarts within the window.
    pub max_restarts: u32,
    /// Time window for counting restarts (seconds).
    pub window_secs: u64,
    /// Backoff multiplier for restarts.
    pub backoff_multiplier: f64,
    /// Initial backoff delay (milliseconds).
    pub initial_backoff_ms: u64,
    /// Maximum backoff delay (milliseconds).
    pub max_backoff_ms: u64,
}

impl Default for SupervisionRestartPolicy {
    fn default() -> Self {
        Self {
            max_restarts: 3,
            window_secs: 60,
            backoff_multiplier: 2.0,
            initial_backoff_ms: 100,
            max_backoff_ms: 30000,
        }
    }
}

/// A team of agents working together.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTeam {
    /// Team identifier.
    pub id: String,
    /// Team name.
    pub name: String,
    /// Lead agent (supervisor).
    pub lead: AgentId,
    /// Member agents.
    pub members: Vec<AgentId>,
    /// Team-level policies (applied to all members).
    pub policies: AgentPolicies,
    /// Communication channel configuration.
    pub channel: TeamChannel,
}

/// Team communication channel configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamChannel {
    /// Whether members can communicate directly.
    pub peer_to_peer: bool,
    /// Whether all messages go through the lead.
    pub hub_and_spoke: bool,
    /// Whether to broadcast messages to all members.
    pub broadcast_enabled: bool,
}

impl Default for TeamChannel {
    fn default() -> Self {
        Self {
            peer_to_peer: true,
            hub_and_spoke: false,
            broadcast_enabled: true,
        }
    }
}

/// A delegation request from one agent to another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationRequest {
    /// Unique delegation ID.
    pub id: String,
    /// Source agent making the delegation.
    pub from_agent: AgentId,
    /// Target agent receiving the delegation.
    pub to_agent: AgentId,
    /// The task being delegated.
    pub task: AgentTask,
    /// Reason for delegation.
    pub reason: String,
    /// Policies to propagate to the delegate.
    pub propagated_policies: Option<AgentPolicies>,
    /// Budget allocation for this delegation.
    pub budget_allocation: Option<BudgetConsumed>,
    /// Delegation depth (for tracking chain).
    pub depth: usize,
    /// Chain of agents in the delegation.
    pub chain: Vec<AgentId>,
    /// Created timestamp.
    pub created_at: DateTime<Utc>,
}

/// Result of a delegation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationResult {
    /// The delegation request ID.
    pub delegation_id: String,
    /// Whether delegation was accepted and completed.
    pub success: bool,
    /// The task result if completed.
    pub task_result: Option<TaskResult>,
    /// Error if delegation failed.
    pub error: Option<String>,
    /// Time taken for delegation (including execution).
    pub duration_ms: u64,
}

/// Supervision strategy for agent failures.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SupervisionStrategy {
    /// Restart the failed agent.
    Restart,
    /// Stop the agent and notify supervisor.
    Stop,
    /// Escalate to human supervisor.
    Escalate,
    /// Resume with error logged.
    Resume,
    /// Delegate to another agent.
    Delegate,
}

/// Supervision event for tracking agent health.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisionEvent {
    /// Agent that triggered the event.
    pub agent_id: AgentId,
    /// Type of event.
    pub event_type: SupervisionEventType,
    /// Timestamp.
    pub timestamp: DateTime<Utc>,
    /// Additional details.
    pub details: Option<String>,
}

/// Types of supervision events.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SupervisionEventType {
    /// Agent started successfully.
    Started,
    /// Agent stopped normally.
    Stopped,
    /// Agent failed with error.
    Failed,
    /// Agent was restarted.
    Restarted,
    /// Agent was escalated.
    Escalated,
    /// Agent health check passed.
    HealthCheckPassed,
    /// Agent health check failed.
    HealthCheckFailed,
}

/// Supervisor that manages a group of agents.
pub struct AgentSupervisor {
    /// Configuration.
    config: OrchestrationConfig,
    /// Supervised agents and their strategies.
    supervised: RwLock<HashMap<AgentId, SupervisionStrategy>>,
    /// Restart counts per agent.
    restart_counts: RwLock<HashMap<AgentId, Vec<DateTime<Utc>>>>,
    /// Supervision events.
    events: RwLock<Vec<SupervisionEvent>>,
}

impl AgentSupervisor {
    pub fn new(config: OrchestrationConfig) -> Self {
        Self {
            config,
            supervised: RwLock::new(HashMap::new()),
            restart_counts: RwLock::new(HashMap::new()),
            events: RwLock::new(Vec::new()),
        }
    }

    /// Add an agent to supervision with a given strategy.
    pub async fn supervise(&self, agent_id: AgentId, strategy: SupervisionStrategy) {
        let mut supervised = self.supervised.write().await;
        supervised.insert(agent_id.clone(), strategy);

        self.record_event(SupervisionEvent {
            agent_id,
            event_type: SupervisionEventType::Started,
            timestamp: Utc::now(),
            details: Some(format!("Supervision started with strategy: {:?}", strategy)),
        })
        .await;
    }

    /// Remove an agent from supervision.
    pub async fn unsupervise(&self, agent_id: &AgentId) {
        let mut supervised = self.supervised.write().await;
        supervised.remove(agent_id);

        self.record_event(SupervisionEvent {
            agent_id: agent_id.clone(),
            event_type: SupervisionEventType::Stopped,
            timestamp: Utc::now(),
            details: Some("Supervision stopped".to_string()),
        })
        .await;
    }

    /// Handle an agent failure.
    pub async fn handle_failure(&self, agent_id: &AgentId, error: &str) -> Result<SupervisionAction> {
        let supervised = self.supervised.read().await;
        let strategy = supervised.get(agent_id).copied().unwrap_or(SupervisionStrategy::Stop);
        drop(supervised);

        self.record_event(SupervisionEvent {
            agent_id: agent_id.clone(),
            event_type: SupervisionEventType::Failed,
            timestamp: Utc::now(),
            details: Some(error.to_string()),
        })
        .await;

        match strategy {
            SupervisionStrategy::Restart => {
                if self.can_restart(agent_id).await {
                    self.record_restart(agent_id).await;
                    self.record_event(SupervisionEvent {
                        agent_id: agent_id.clone(),
                        event_type: SupervisionEventType::Restarted,
                        timestamp: Utc::now(),
                        details: None,
                    })
                    .await;
                    Ok(SupervisionAction::Restart {
                        backoff_ms: self.calculate_backoff(agent_id).await,
                    })
                } else {
                    self.record_event(SupervisionEvent {
                        agent_id: agent_id.clone(),
                        event_type: SupervisionEventType::Escalated,
                        timestamp: Utc::now(),
                        details: Some("Max restarts exceeded".to_string()),
                    })
                    .await;
                    Ok(SupervisionAction::Escalate {
                        reason: "Max restarts exceeded".to_string(),
                    })
                }
            }
            SupervisionStrategy::Stop => Ok(SupervisionAction::Stop),
            SupervisionStrategy::Escalate => {
                self.record_event(SupervisionEvent {
                    agent_id: agent_id.clone(),
                    event_type: SupervisionEventType::Escalated,
                    timestamp: Utc::now(),
                    details: Some(error.to_string()),
                })
                .await;
                Ok(SupervisionAction::Escalate {
                    reason: error.to_string(),
                })
            }
            SupervisionStrategy::Resume => Ok(SupervisionAction::Resume),
            SupervisionStrategy::Delegate => {
                Ok(SupervisionAction::Delegate { target: None })
            }
        }
    }

    /// Check if an agent can be restarted (within restart limits).
    async fn can_restart(&self, agent_id: &AgentId) -> bool {
        let counts = self.restart_counts.read().await;
        if let Some(restarts) = counts.get(agent_id) {
            let window = chrono::Duration::seconds(self.config.supervision_restart_policy.window_secs as i64);
            let cutoff = Utc::now() - window;
            let recent_restarts = restarts.iter().filter(|t| **t > cutoff).count();
            recent_restarts < self.config.supervision_restart_policy.max_restarts as usize
        } else {
            true
        }
    }

    /// Record a restart attempt.
    async fn record_restart(&self, agent_id: &AgentId) {
        let mut counts = self.restart_counts.write().await;
        counts
            .entry(agent_id.clone())
            .or_insert_with(Vec::new)
            .push(Utc::now());
    }

    /// Calculate backoff delay for restart.
    async fn calculate_backoff(&self, agent_id: &AgentId) -> u64 {
        let counts = self.restart_counts.read().await;
        let restart_count = counts.get(agent_id).map(|v| v.len()).unwrap_or(0);

        let policy = &self.config.supervision_restart_policy;
        let backoff = policy.initial_backoff_ms as f64
            * policy.backoff_multiplier.powi(restart_count as i32);
        backoff.min(policy.max_backoff_ms as f64) as u64
    }

    /// Record a supervision event.
    async fn record_event(&self, event: SupervisionEvent) {
        let mut events = self.events.write().await;
        events.push(event);
        // Keep only recent events (last 1000)
        if events.len() > 1000 {
            events.drain(0..100);
        }
    }

    /// Get supervision events for an agent.
    pub async fn get_events(&self, agent_id: &AgentId) -> Vec<SupervisionEvent> {
        let events = self.events.read().await;
        events
            .iter()
            .filter(|e| &e.agent_id == agent_id)
            .cloned()
            .collect()
    }

    /// Get all supervision events.
    pub async fn get_all_events(&self) -> Vec<SupervisionEvent> {
        let events = self.events.read().await;
        events.clone()
    }

    /// Check if an agent is supervised.
    pub async fn is_supervised(&self, agent_id: &AgentId) -> bool {
        let supervised = self.supervised.read().await;
        supervised.contains_key(agent_id)
    }

    /// Get the supervision strategy for an agent.
    pub async fn get_strategy(&self, agent_id: &AgentId) -> Option<SupervisionStrategy> {
        let supervised = self.supervised.read().await;
        supervised.get(agent_id).copied()
    }
}

/// Action to take after supervision decision.
#[derive(Debug, Clone)]
pub enum SupervisionAction {
    /// Restart the agent after backoff.
    Restart { backoff_ms: u64 },
    /// Stop the agent permanently.
    Stop,
    /// Escalate to human/higher authority.
    Escalate { reason: String },
    /// Resume without action.
    Resume,
    /// Delegate to another agent.
    Delegate { target: Option<AgentId> },
}

/// Orchestrator that coordinates multiple agents.
pub struct AgentOrchestrator<P, M, T, E, A>
where
    P: AgentPolicyEngine,
    M: McpResolver,
    T: ToolExecutor,
    E: EventLog,
    A: AgentStore,
{
    config: OrchestrationConfig,
    runtime: Arc<AgentRuntime<P, M, T, E>>,
    agent_store: Arc<A>,
    #[allow(dead_code)]
    event_log: Arc<E>,
    /// Active teams.
    teams: RwLock<HashMap<String, AgentTeam>>,
    /// Pending delegations.
    pending_delegations: RwLock<HashMap<String, DelegationRequest>>,
    /// Supervisor for managed agents.
    supervisor: AgentSupervisor,
}

impl<P, M, T, E, A> AgentOrchestrator<P, M, T, E, A>
where
    P: AgentPolicyEngine + 'static,
    M: McpResolver + 'static,
    T: ToolExecutor + 'static,
    E: EventLog + 'static,
    A: AgentStore + 'static,
{
    pub fn new(
        config: OrchestrationConfig,
        runtime: Arc<AgentRuntime<P, M, T, E>>,
        agent_store: Arc<A>,
        event_log: Arc<E>,
    ) -> Self {
        let supervisor = AgentSupervisor::new(config.clone());
        Self {
            config,
            runtime,
            agent_store,
            event_log,
            teams: RwLock::new(HashMap::new()),
            pending_delegations: RwLock::new(HashMap::new()),
            supervisor,
        }
    }

    /// Create a new team.
    pub async fn create_team(&self, team: AgentTeam) -> Result<()> {
        // Validate lead exists
        self.agent_store
            .get_agent(&team.lead)
            .await?
            .ok_or_else(|| anyhow!("Lead employee not found: {}", team.lead.0))?;

        // Validate all members exist
        for member in &team.members {
            self.agent_store
                .get_agent(member)
                .await?
                .ok_or_else(|| anyhow!("Team member not found: {}", member.0))?;
        }

        let mut teams = self.teams.write().await;
        teams.insert(team.id.clone(), team);
        Ok(())
    }

    /// Get a team by ID.
    pub async fn get_team(&self, team_id: &str) -> Option<AgentTeam> {
        let teams = self.teams.read().await;
        teams.get(team_id).cloned()
    }

    /// List all teams.
    pub async fn list_teams(&self) -> Vec<AgentTeam> {
        let teams = self.teams.read().await;
        teams.values().cloned().collect()
    }

    /// Delete a team.
    pub async fn delete_team(&self, team_id: &str) -> bool {
        let mut teams = self.teams.write().await;
        teams.remove(team_id).is_some()
    }

    /// Delegate a task from one agent to another.
    pub async fn delegate(&self, request: DelegationRequest) -> Result<DelegationResult> {
        let start_time = Utc::now();

        // Check delegation depth
        if request.depth >= self.config.max_delegation_depth {
            return Ok(DelegationResult {
                delegation_id: request.id,
                success: false,
                task_result: None,
                error: Some(format!(
                    "Max delegation depth exceeded: {}",
                    self.config.max_delegation_depth
                )),
                duration_ms: 0,
            });
        }

        // Load source and target agents
        let from_agent = self
            .agent_store
            .get_agent(&request.from_agent)
            .await?
            .ok_or_else(|| anyhow!("Source employee not found"))?;

        let target_agent = self
            .agent_store
            .get_agent(&request.to_agent)
            .await?
            .ok_or_else(|| anyhow!("Target employee not found"))?;

        // Check if target agent is active
        if target_agent.status != AgentStatus::Active {
            return Ok(DelegationResult {
                delegation_id: request.id,
                success: false,
                task_result: None,
                error: Some(format!(
                    "Target employee is not active: {:?}",
                    target_agent.status
                )),
                duration_ms: 0,
            });
        }

        // Check delegation rules
        if !self.check_delegation_allowed(&from_agent, &target_agent, &request).await? {
            return Ok(DelegationResult {
                delegation_id: request.id,
                success: false,
                task_result: None,
                error: Some("Delegation not allowed by policy".to_string()),
                duration_ms: 0,
            });
        }

        // Store pending delegation
        {
            let mut pending = self.pending_delegations.write().await;
            pending.insert(request.id.clone(), request.clone());
        }

        // Build execution context with delegation chain
        let mut chain = request.chain.clone();
        chain.push(request.from_agent.clone());

        let context = ExecutionContext {
            run_id: request.task.run_id.clone(),
            step_id: request.task.step_id.clone(),
            environment: "production".to_string(),
            parent_agent: Some(request.from_agent.clone()),
            delegation_chain: chain,
        };

        // Execute the task with the target agent
        let task_result = self
            .runtime
            .execute_task(&target_agent, request.task.clone(), context)
            .await?;

        // Remove from pending
        {
            let mut pending = self.pending_delegations.write().await;
            pending.remove(&request.id);
        }

        let end_time = Utc::now();
        let duration_ms = (end_time - start_time).num_milliseconds() as u64;

        Ok(DelegationResult {
            delegation_id: request.id,
            success: task_result.success,
            task_result: Some(task_result),
            error: None,
            duration_ms,
        })
    }

    /// Check if delegation is allowed by policies.
    async fn check_delegation_allowed(
        &self,
        from: &Agent,
        to: &Agent,
        _request: &DelegationRequest,
    ) -> Result<bool> {
        // Check source agent's delegation rules
        let rules = &from.policies.delegation;

        // Check if delegation is enabled
        if !rules.can_delegate {
            return Ok(false);
        }

        // Check if target is in the allowed delegation targets
        if !rules.delegate_to.is_empty() {
            let allowed = rules.delegate_to.iter().any(|t| t.agent_id.0 == to.id.0);
            if !allowed {
                return Ok(false);
            }
        }

        // Check cross-team delegation
        if !self.config.allow_cross_team_delegation {
            let from_team = self.find_agent_team(&from.id).await;
            let to_team = self.find_agent_team(&to.id).await;

            if from_team != to_team {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Find which team an agent belongs to.
    async fn find_agent_team(&self, agent_id: &AgentId) -> Option<String> {
        let teams = self.teams.read().await;
        for (team_id, team) in teams.iter() {
            if &team.lead == agent_id || team.members.contains(agent_id) {
                return Some(team_id.clone());
            }
        }
        None
    }

    /// Execute a task with the best matching agent from a team.
    pub async fn execute_with_team(
        &self,
        team_id: &str,
        task: AgentTask,
    ) -> Result<TaskResult> {
        let team = self
            .get_team(team_id)
            .await
            .ok_or_else(|| anyhow!("Team not found: {}", team_id))?;

        // Start with lead agent
        let lead = self
            .agent_store
            .get_agent(&team.lead)
            .await?
            .ok_or_else(|| anyhow!("Lead employee not found"))?;

        let context = ExecutionContext {
            run_id: task.run_id.clone(),
            step_id: task.step_id.clone(),
            environment: "production".to_string(),
            parent_agent: None,
            delegation_chain: Vec::new(),
        };

        // Execute with lead first
        let result = self.runtime.execute_task(&lead, task.clone(), context.clone()).await?;

        // If lead succeeds, return
        if result.success {
            return Ok(result);
        }

        // Try team members
        for member_id in &team.members {
            if let Some(member) = self.agent_store.get_agent(member_id).await? {
                if member.status == AgentStatus::Active {
                    let member_context = ExecutionContext {
                        parent_agent: Some(team.lead.clone()),
                        ..context.clone()
                    };

                    let member_result = self
                        .runtime
                        .execute_task(&member, task.clone(), member_context)
                        .await?;

                    if member_result.success {
                        return Ok(member_result);
                    }
                }
            }
        }

        // Return the original failure
        Ok(result)
    }

    /// Execute a workflow step using an agent.
    pub async fn execute_workflow_step(
        &self,
        agent_id: &AgentId,
        step_id: &StepId,
        run_id: &RunId,
        prompt: String,
    ) -> Result<TaskResult> {
        let agent = self
            .agent_store
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| anyhow!("Employee not found: {}", agent_id.0))?;

        let task = AgentTask {
            id: format!("{}:{}", run_id.0, step_id.0),
            prompt,
            skill: None,
            context: HashMap::new(),
            run_id: Some(run_id.clone()),
            step_id: Some(step_id.0.clone()),
        };

        let context = ExecutionContext {
            run_id: Some(run_id.clone()),
            step_id: Some(step_id.0.clone()),
            environment: "production".to_string(),
            parent_agent: None,
            delegation_chain: Vec::new(),
        };

        // Add to supervision
        self.supervisor
            .supervise(agent_id.clone(), SupervisionStrategy::Restart)
            .await;

        // Execute
        let result = self.runtime.execute_task(&agent, task, context).await;

        // Handle failure with supervision
        match &result {
            Ok(r) if !r.success => {
                if let Some(ref error) = r.error {
                    let action = self.supervisor.handle_failure(agent_id, error).await?;
                    tracing::info!("Supervision action for {}: {:?}", agent_id.0, action);
                }
            }
            Err(e) => {
                let action = self.supervisor.handle_failure(agent_id, &e.to_string()).await?;
                tracing::info!("Supervision action for {}: {:?}", agent_id.0, action);
            }
            _ => {}
        }

        result
    }

    /// Broadcast a task to all members of a team.
    pub async fn broadcast_to_team(
        &self,
        team_id: &str,
        task: AgentTask,
    ) -> Result<Vec<TaskResult>> {
        let team = self
            .get_team(team_id)
            .await
            .ok_or_else(|| anyhow!("Team not found: {}", team_id))?;

        if !team.channel.broadcast_enabled {
            return Err(anyhow!("Broadcast not enabled for team"));
        }

        let mut results = Vec::new();
        let mut all_agents = vec![team.lead.clone()];
        all_agents.extend(team.members.clone());

        // Execute concurrently with bounded concurrency
        let semaphore = Arc::new(tokio::sync::Semaphore::new(
            self.config.max_concurrent_agents,
        ));

        let runtime = self.runtime.clone();
        let agent_store = self.agent_store.clone();

        let handles: Vec<_> = all_agents
            .into_iter()
            .map(|agent_id| {
                let runtime = runtime.clone();
                let store = agent_store.clone();
                let task = task.clone();
                let sem = semaphore.clone();

                tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();

                    if let Some(agent) = store.get_agent(&agent_id).await.ok().flatten() {
                        if agent.status == AgentStatus::Active {
                            let context = ExecutionContext {
                                run_id: task.run_id.clone(),
                                step_id: task.step_id.clone(),
                                environment: "production".to_string(),
                                parent_agent: None,
                                delegation_chain: Vec::new(),
                            };
                            return runtime.execute_task(&agent, task, context).await.ok();
                        }
                    }
                    None
                })
            })
            .collect();

        for handle in handles {
            if let Ok(Some(result)) = handle.await {
                results.push(result);
            }
        }

        Ok(results)
    }

    /// Get the supervisor for this orchestrator.
    pub fn supervisor(&self) -> &AgentSupervisor {
        &self.supervisor
    }

    /// Get pending delegations.
    pub async fn get_pending_delegations(&self) -> Vec<DelegationRequest> {
        let pending = self.pending_delegations.read().await;
        pending.values().cloned().collect()
    }
}

/// Workflow adapter for executing workflow steps with agents.
pub struct AgentWorkflowAdapter<P, M, T, E, A>
where
    P: AgentPolicyEngine,
    M: McpResolver,
    T: ToolExecutor,
    E: EventLog,
    A: AgentStore,
{
    orchestrator: Arc<AgentOrchestrator<P, M, T, E, A>>,
    /// Mapping from role IDs to agent IDs.
    role_agent_mapping: RwLock<HashMap<String, AgentId>>,
}

impl<P, M, T, E, A> AgentWorkflowAdapter<P, M, T, E, A>
where
    P: AgentPolicyEngine + 'static,
    M: McpResolver + 'static,
    T: ToolExecutor + 'static,
    E: EventLog + 'static,
    A: AgentStore + 'static,
{
    pub fn new(orchestrator: Arc<AgentOrchestrator<P, M, T, E, A>>) -> Self {
        Self {
            orchestrator,
            role_agent_mapping: RwLock::new(HashMap::new()),
        }
    }

    /// Register a mapping from role ID to agent ID.
    pub async fn register_role_agent(&self, role_id: &str, agent_id: AgentId) {
        let mut mapping = self.role_agent_mapping.write().await;
        mapping.insert(role_id.to_string(), agent_id);
    }

    /// Get the agent for a role.
    pub async fn get_agent_for_role(&self, role_id: &str) -> Option<AgentId> {
        let mapping = self.role_agent_mapping.read().await;
        mapping.get(role_id).cloned()
    }

    /// Execute a workflow step, finding the appropriate agent by role.
    pub async fn execute_step(
        &self,
        role_id: &str,
        step_id: &StepId,
        run_id: &RunId,
        prompt: String,
    ) -> Result<TaskResult> {
        let agent_id = self
            .get_agent_for_role(role_id)
            .await
            .ok_or_else(|| anyhow!("No agent registered for role: {}", role_id))?;

        self.orchestrator
            .execute_workflow_step(&agent_id, step_id, run_id, prompt)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{
        runtime::{InMemoryToolExecutor, InMemoryMcpResolver, RuntimeConfig},
        policy_engine::{DefaultPolicyEngine, InMemoryPolicyStorage},
        AgentOrganization, DelegationRules,
    };
    use crate::events::InMemoryEventLog;
    use crate::storage::InMemoryAgentStore;
    use crate::types::TeamId;

    type TestOrchestrator = AgentOrchestrator<
        DefaultPolicyEngine<InMemoryPolicyStorage>,
        InMemoryMcpResolver,
        InMemoryToolExecutor,
        InMemoryEventLog,
        InMemoryAgentStore,
    >;

    fn test_agent(id: &str, name: &str) -> Agent {
        Agent::builder(AgentId::new(id), name)
            .organization(AgentOrganization::new(TeamId::new("test-team")))
            .build()
    }

    fn test_agent_with_delegation(id: &str, name: &str, delegation: DelegationRules) -> Agent {
        Agent::builder(AgentId::new(id), name)
            .organization(AgentOrganization::new(TeamId::new("test-team")))
            .policies(AgentPolicies::new().with_delegation(delegation))
            .build()
    }

    async fn create_test_orchestrator() -> TestOrchestrator {
        let policy_storage = InMemoryPolicyStorage::new();
        let policy_engine = DefaultPolicyEngine::new(policy_storage);

        let mcp_resolver = InMemoryMcpResolver::new();

        let tool_executor = InMemoryToolExecutor::new();
        let event_log = Arc::new(InMemoryEventLog::new());
        let agent_store = Arc::new(InMemoryAgentStore::new());

        let runtime = Arc::new(AgentRuntime::new(
            RuntimeConfig::default(),
            Arc::new(policy_engine),
            Arc::new(mcp_resolver),
            Arc::new(tool_executor),
            event_log.clone(),
        ));

        AgentOrchestrator::new(
            OrchestrationConfig::default(),
            runtime,
            agent_store,
            event_log,
        )
    }

    #[tokio::test]
    async fn test_create_team() {
        let orchestrator = create_test_orchestrator().await;

        // Create agents
        let lead = test_agent("lead", "Team Lead");
        let member1 = test_agent("member1", "Member 1");
        let member2 = test_agent("member2", "Member 2");

        orchestrator.agent_store.store_agent(&lead).await.unwrap();
        orchestrator.agent_store.store_agent(&member1).await.unwrap();
        orchestrator.agent_store.store_agent(&member2).await.unwrap();

        // Create team
        let team = AgentTeam {
            id: "team1".to_string(),
            name: "Test Team".to_string(),
            lead: lead.id.clone(),
            members: vec![member1.id, member2.id],
            policies: AgentPolicies::default(),
            channel: TeamChannel::default(),
        };

        orchestrator.create_team(team).await.unwrap();

        let retrieved = orchestrator.get_team("team1").await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "Test Team");
    }

    #[tokio::test]
    async fn test_delegation() {
        let orchestrator = create_test_orchestrator().await;

        // Create agents with delegation rules
        let from_agent = test_agent_with_delegation(
            "from",
            "From Agent",
            DelegationRules::allowing(),
        );

        let to_agent = test_agent("to", "To Agent");

        orchestrator.agent_store.store_agent(&from_agent).await.unwrap();
        orchestrator.agent_store.store_agent(&to_agent).await.unwrap();

        // Create delegation request
        let request = DelegationRequest {
            id: "del1".to_string(),
            from_agent: from_agent.id,
            to_agent: to_agent.id,
            task: AgentTask {
                id: "task1".to_string(),
                prompt: "Do something".to_string(),
                skill: None,
                context: HashMap::new(),
                run_id: None,
                step_id: None,
            },
            reason: "Test delegation".to_string(),
            propagated_policies: None,
            budget_allocation: None,
            depth: 0,
            chain: Vec::new(),
            created_at: Utc::now(),
        };

        let result = orchestrator.delegate(request).await.unwrap();
        assert!(result.success);
        assert!(result.task_result.is_some());
    }

    #[tokio::test]
    async fn test_delegation_depth_exceeded() {
        let orchestrator = create_test_orchestrator().await;

        let from_agent = test_agent_with_delegation(
            "from",
            "From Agent",
            DelegationRules::allowing(),
        );

        let to_agent = test_agent("to", "To Agent");

        orchestrator.agent_store.store_agent(&from_agent).await.unwrap();
        orchestrator.agent_store.store_agent(&to_agent).await.unwrap();

        // Create request with depth at max
        let request = DelegationRequest {
            id: "del1".to_string(),
            from_agent: from_agent.id,
            to_agent: to_agent.id,
            task: AgentTask {
                id: "task1".to_string(),
                prompt: "Do something".to_string(),
                skill: None,
                context: HashMap::new(),
                run_id: None,
                step_id: None,
            },
            reason: "Test".to_string(),
            propagated_policies: None,
            budget_allocation: None,
            depth: 10, // Exceeds default max of 5
            chain: Vec::new(),
            created_at: Utc::now(),
        };

        let result = orchestrator.delegate(request).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("depth exceeded"));
    }

    #[tokio::test]
    async fn test_supervision() {
        let supervisor = AgentSupervisor::new(OrchestrationConfig::default());

        let agent_id = AgentId::new("test-agent");

        // Add to supervision
        supervisor.supervise(agent_id.clone(), SupervisionStrategy::Restart).await;
        assert!(supervisor.is_supervised(&agent_id).await);

        // Handle failure
        let action = supervisor.handle_failure(&agent_id, "Test error").await.unwrap();
        match action {
            SupervisionAction::Restart { .. } => {}
            _ => panic!("Expected restart action"),
        }

        // Check events
        let events = supervisor.get_events(&agent_id).await;
        assert!(events.len() >= 2); // Started + Failed + Restarted
    }

    #[tokio::test]
    async fn test_supervision_max_restarts() {
        let mut config = OrchestrationConfig::default();
        config.supervision_restart_policy.max_restarts = 2;
        config.supervision_restart_policy.window_secs = 3600;

        let supervisor = AgentSupervisor::new(config);

        let agent_id = AgentId::new("test-agent");
        supervisor.supervise(agent_id.clone(), SupervisionStrategy::Restart).await;

        // Trigger multiple failures
        for i in 0..3 {
            let action = supervisor.handle_failure(&agent_id, &format!("Error {}", i)).await.unwrap();
            if i < 2 {
                match action {
                    SupervisionAction::Restart { .. } => {}
                    _ => panic!("Expected restart on attempt {}", i),
                }
            } else {
                match action {
                    SupervisionAction::Escalate { .. } => {}
                    _ => panic!("Expected escalation after max restarts"),
                }
            }
        }
    }

    #[tokio::test]
    async fn test_workflow_adapter() {
        let orchestrator = Arc::new(create_test_orchestrator().await);
        let adapter = AgentWorkflowAdapter::new(orchestrator.clone());

        // Create and store agent
        let agent = test_agent("workflow-agent", "Workflow Agent");
        orchestrator.agent_store.store_agent(&agent).await.unwrap();

        // Register role mapping
        adapter.register_role_agent("developer", agent.id.clone()).await;

        // Verify mapping
        let retrieved = adapter.get_agent_for_role("developer").await;
        assert_eq!(retrieved, Some(agent.id));
    }
}
