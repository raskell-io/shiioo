//! Integration tests for the agent system.
//!
//! These tests verify that the agent runtime, orchestration, and policy systems
//! work together correctly.

mod common;

use chrono::Utc;
use common::{agent_with_budgets, simple_agent, AgentTestFixture, TestAgentBuilder};
use shiioo_core::agent::{
    AgentOrchestrator, AgentPolicies, AgentRuntime, AgentStatus, AgentTask, AgentTeam,
    BudgetConsumed, DelegationRequest, ExecutionContext, OrchestrationConfig, RuntimeConfig,
    TeamChannel,
};
use shiioo_core::storage::AgentStore;
use std::collections::HashMap;
use std::sync::Arc;

/// Test that an agent can execute a simple task successfully.
#[tokio::test]
async fn test_agent_executes_simple_task() {
    let fixture = AgentTestFixture::new();
    let agent = agent_with_budgets("agent-1", "Test Agent");

    let runtime = AgentRuntime::new(
        RuntimeConfig::default(),
        fixture.policy_engine.clone(),
        fixture.mcp_resolver.clone(),
        fixture.tool_executor.clone(),
        fixture.event_log.clone(),
    );

    let task = AgentTask {
        id: "task-1".to_string(),
        prompt: "Hello, world!".to_string(),
        skill: None,
        context: HashMap::new(),
        run_id: None,
        step_id: None,
    };

    let result = runtime
        .execute_task(&agent, task, ExecutionContext::default())
        .await
        .expect("Task execution should succeed");

    assert!(result.success, "Task should complete successfully");
    assert!(result.error.is_none(), "No error should be present");
}

/// Test that suspended agents cannot execute tasks.
#[tokio::test]
async fn test_suspended_agent_cannot_execute() {
    let fixture = AgentTestFixture::new();
    let agent = TestAgentBuilder::new("agent-suspended", "Suspended Agent")
        .status(AgentStatus::Suspended)
        .build();

    let runtime = AgentRuntime::new(
        RuntimeConfig::default(),
        fixture.policy_engine.clone(),
        fixture.mcp_resolver.clone(),
        fixture.tool_executor.clone(),
        fixture.event_log.clone(),
    );

    let task = AgentTask {
        id: "task-1".to_string(),
        prompt: "This should fail".to_string(),
        skill: None,
        context: HashMap::new(),
        run_id: None,
        step_id: None,
    };

    let result = runtime
        .execute_task(&agent, task, ExecutionContext::default())
        .await
        .expect("Should return result, not error");

    assert!(!result.success, "Task should fail for suspended agent");
    assert!(
        result.error.as_ref().unwrap().contains("not active"),
        "Error should mention agent not active: {:?}",
        result.error
    );
}

/// Test that archived agents cannot execute tasks.
#[tokio::test]
async fn test_archived_agent_cannot_execute() {
    let fixture = AgentTestFixture::new();
    let agent = TestAgentBuilder::new("agent-archived", "Archived Agent")
        .status(AgentStatus::Archived)
        .build();

    let runtime = AgentRuntime::new(
        RuntimeConfig::default(),
        fixture.policy_engine.clone(),
        fixture.mcp_resolver.clone(),
        fixture.tool_executor.clone(),
        fixture.event_log.clone(),
    );

    let task = AgentTask {
        id: "task-1".to_string(),
        prompt: "This should fail".to_string(),
        skill: None,
        context: HashMap::new(),
        run_id: None,
        step_id: None,
    };

    let result = runtime
        .execute_task(&agent, task, ExecutionContext::default())
        .await
        .expect("Should return result, not error");

    assert!(!result.success, "Task should fail for archived agent");
    assert!(
        result.error.as_ref().unwrap().contains("not active"),
        "Error should mention agent not active: {:?}",
        result.error
    );
}

/// Test agent orchestration with multiple agents.
#[tokio::test]
async fn test_orchestrator_manages_multiple_agents() {
    let fixture = AgentTestFixture::new();

    // Store agents
    let agent1 = simple_agent("agent-1", "Agent One");
    let agent2 = simple_agent("agent-2", "Agent Two");

    fixture.agent_store.store_agent(&agent1).await.expect("Store agent 1");
    fixture.agent_store.store_agent(&agent2).await.expect("Store agent 2");

    // Create runtime
    let runtime = Arc::new(AgentRuntime::new(
        RuntimeConfig::default(),
        fixture.policy_engine.clone(),
        fixture.mcp_resolver.clone(),
        fixture.tool_executor.clone(),
        fixture.event_log.clone(),
    ));

    let orchestrator = AgentOrchestrator::new(
        OrchestrationConfig::default(),
        runtime,
        fixture.agent_store.clone(),
        fixture.event_log.clone(),
    );

    // Create a team with both agents
    let team = AgentTeam {
        id: "test-team".to_string(),
        name: "Test Team".to_string(),
        lead: agent1.id.clone(),
        members: vec![agent1.id.clone(), agent2.id.clone()],
        policies: AgentPolicies::default(),
        channel: TeamChannel::default(),
    };

    orchestrator
        .create_team(team)
        .await
        .expect("Team creation should succeed");

    // Verify team was created
    let retrieved_team = orchestrator.get_team("test-team").await;
    assert!(retrieved_team.is_some(), "Team should be retrievable");
    assert_eq!(
        retrieved_team.unwrap().members.len(),
        2,
        "Team should have 2 members"
    );
}

/// Test agent delegation.
#[tokio::test]
async fn test_agent_delegation() {
    let fixture = AgentTestFixture::new();

    let delegator = simple_agent("delegator", "Delegator Agent");
    let delegate = simple_agent("delegate", "Delegate Agent");

    fixture.agent_store.store_agent(&delegator).await.expect("Store delegator");
    fixture.agent_store.store_agent(&delegate).await.expect("Store delegate");

    // Create runtime
    let runtime = Arc::new(AgentRuntime::new(
        RuntimeConfig::default(),
        fixture.policy_engine.clone(),
        fixture.mcp_resolver.clone(),
        fixture.tool_executor.clone(),
        fixture.event_log.clone(),
    ));

    let orchestrator = AgentOrchestrator::new(
        OrchestrationConfig::default(),
        runtime,
        fixture.agent_store.clone(),
        fixture.event_log.clone(),
    );

    let request = DelegationRequest {
        id: uuid::Uuid::new_v4().to_string(),
        from_agent: delegator.id.clone(),
        to_agent: delegate.id.clone(),
        task: AgentTask {
            id: "delegated-task".to_string(),
            prompt: "Please handle this".to_string(),
            skill: None,
            context: HashMap::new(),
            run_id: None,
            step_id: None,
        },
        reason: "Specialized handling required".to_string(),
        propagated_policies: None,
        budget_allocation: Some(BudgetConsumed {
            tokens_input: 500,
            tokens_output: 500,
            cost_cents: 10,
            requests: 1,
            tool_calls: 0,
        }),
        depth: 0,
        chain: vec![delegator.id.clone()],
        created_at: Utc::now(),
    };

    let result = orchestrator
        .delegate(request)
        .await
        .expect("Delegation should succeed");

    // Note: With default policy configuration, delegation is not allowed
    // This tests the policy enforcement for delegation
    if !result.success {
        assert!(
            result.error.as_ref().unwrap().contains("policy"),
            "Should fail due to policy: {:?}",
            result.error
        );
    } else {
        // If policies allow it, verify the task result
        assert!(result.task_result.is_some(), "Should have task result");
        assert!(
            result.task_result.as_ref().unwrap().success,
            "Delegated task should succeed"
        );
    }
}

/// Test that budget tracking accumulates correctly across multiple tasks.
#[tokio::test]
async fn test_budget_accumulation() {
    let fixture = AgentTestFixture::new();
    let agent = agent_with_budgets("accumulator", "Accumulator Agent");

    let runtime = AgentRuntime::new(
        RuntimeConfig::default(),
        fixture.policy_engine.clone(),
        fixture.mcp_resolver.clone(),
        fixture.tool_executor.clone(),
        fixture.event_log.clone(),
    );

    // Execute multiple tasks
    for i in 1..=5 {
        let task = AgentTask {
            id: format!("task-{}", i),
            prompt: format!("Task {}", i),
            skill: None,
            context: HashMap::new(),
            run_id: None,
            step_id: None,
        };

        let _ = runtime
            .execute_task(&agent, task, ExecutionContext::default())
            .await
            .expect("Task should execute");
    }

    // Check that budget stats have been updated
    let stats = runtime
        .get_budget_stats(&agent.id)
        .await
        .expect("Should have budget stats");

    assert!(
        stats.requests_this_hour >= 5,
        "Should have at least 5 requests tracked"
    );
}

/// Test concurrent task execution on the same agent.
#[tokio::test]
async fn test_concurrent_task_execution() {
    let fixture = AgentTestFixture::new();
    let agent = agent_with_budgets("concurrent-agent", "Concurrent Agent");

    let runtime = Arc::new(AgentRuntime::new(
        RuntimeConfig::default(),
        fixture.policy_engine.clone(),
        fixture.mcp_resolver.clone(),
        fixture.tool_executor.clone(),
        fixture.event_log.clone(),
    ));

    // Spawn multiple tasks concurrently
    let mut handles = Vec::new();
    for i in 1..=10 {
        let runtime = runtime.clone();
        let agent = agent.clone();
        let handle = tokio::spawn(async move {
            let task = AgentTask {
                id: format!("concurrent-task-{}", i),
                prompt: format!("Concurrent task {}", i),
                skill: None,
                context: HashMap::new(),
                run_id: None,
                step_id: None,
            };
            runtime
                .execute_task(&agent, task, ExecutionContext::default())
                .await
        });
        handles.push(handle);
    }

    // Wait for all tasks and verify they all succeeded
    let results: Vec<_> = futures::future::join_all(handles).await;
    for result in results {
        let task_result = result
            .expect("Task should not panic")
            .expect("Task should execute");
        assert!(task_result.success, "Concurrent task should succeed");
    }
}

/// Test that agent store operations work correctly.
#[tokio::test]
async fn test_agent_store_operations() {
    let fixture = AgentTestFixture::new();

    // Create and store agent
    let agent = simple_agent("store-test", "Store Test Agent");
    fixture.agent_store.store_agent(&agent).await.expect("Store agent");

    // Retrieve agent
    let retrieved = fixture
        .agent_store
        .get_agent(&agent.id)
        .await
        .expect("Get agent");
    assert!(retrieved.is_some(), "Agent should be found");
    assert_eq!(retrieved.unwrap().name, "Store Test Agent");

    // List agents
    let agents = fixture.agent_store.list_agents().await.expect("List agents");
    assert!(!agents.is_empty(), "Should have at least one agent");

    // Delete agent
    fixture
        .agent_store
        .delete_agent(&agent.id)
        .await
        .expect("Delete agent");

    let deleted = fixture
        .agent_store
        .get_agent(&agent.id)
        .await
        .expect("Get deleted agent");
    assert!(deleted.is_none(), "Deleted agent should not be found");
}

/// Test orchestrator team lifecycle.
#[tokio::test]
async fn test_orchestrator_team_lifecycle() {
    let fixture = AgentTestFixture::new();

    // Create runtime
    let runtime = Arc::new(AgentRuntime::new(
        RuntimeConfig::default(),
        fixture.policy_engine.clone(),
        fixture.mcp_resolver.clone(),
        fixture.tool_executor.clone(),
        fixture.event_log.clone(),
    ));

    let orchestrator = AgentOrchestrator::new(
        OrchestrationConfig::default(),
        runtime,
        fixture.agent_store.clone(),
        fixture.event_log.clone(),
    );

    // Create agents for the team
    let agent1 = simple_agent("team-a1", "Team Agent 1");
    let agent2 = simple_agent("team-a2", "Team Agent 2");

    fixture.agent_store.store_agent(&agent1).await.unwrap();
    fixture.agent_store.store_agent(&agent2).await.unwrap();

    // Create team
    let team = AgentTeam {
        id: "lifecycle-team".to_string(),
        name: "Lifecycle Team".to_string(),
        lead: agent1.id.clone(),
        members: vec![agent1.id.clone(), agent2.id.clone()],
        policies: AgentPolicies::default(),
        channel: TeamChannel::default(),
    };

    orchestrator.create_team(team).await.expect("Create team");

    // List teams
    let teams = orchestrator.list_teams().await;
    assert_eq!(teams.len(), 1, "Should have one team");

    // Delete team
    let deleted = orchestrator.delete_team("lifecycle-team").await;
    assert!(deleted, "Team should be deleted");

    // Verify deletion
    let teams = orchestrator.list_teams().await;
    assert!(teams.is_empty(), "Should have no teams after deletion");
}
