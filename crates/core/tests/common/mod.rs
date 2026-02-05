//! Common test utilities for integration tests.
//!
//! This module provides shared fixtures, builders, and helpers for integration testing.

use shiioo_core::agent::{
    Agent, AgentBudgets, AgentId, AgentOrganization, AgentStatus, CostBudget,
    DefaultPolicyEngine, InMemoryMcpResolver, InMemoryPolicyStorage, InMemoryToolExecutor,
    RequestBudget, RuntimeToolInfo, TokenBudget,
};
use shiioo_core::events::InMemoryEventLog;
use shiioo_core::storage::InMemoryAgentStore;
use shiioo_core::types::TeamId;
use std::sync::Arc;

/// Test fixture for agent-related tests.
pub struct AgentTestFixture {
    pub policy_engine: Arc<DefaultPolicyEngine<InMemoryPolicyStorage>>,
    pub mcp_resolver: Arc<InMemoryMcpResolver>,
    pub tool_executor: Arc<InMemoryToolExecutor>,
    pub event_log: Arc<InMemoryEventLog>,
    pub agent_store: Arc<InMemoryAgentStore>,
}

impl AgentTestFixture {
    /// Create a new test fixture with default components.
    pub fn new() -> Self {
        let policy_storage = InMemoryPolicyStorage::new();
        Self {
            policy_engine: Arc::new(DefaultPolicyEngine::new(policy_storage)),
            mcp_resolver: Arc::new(InMemoryMcpResolver::new()),
            tool_executor: Arc::new(InMemoryToolExecutor::new()),
            event_log: Arc::new(InMemoryEventLog::new()),
            agent_store: Arc::new(InMemoryAgentStore::new()),
        }
    }

    /// Register a tool in the MCP resolver.
    pub async fn register_tool(&self, server_id: &str, name: &str, description: &str, tier: u8) {
        self.mcp_resolver
            .register_tool(RuntimeToolInfo {
                server_id: server_id.to_string(),
                name: name.to_string(),
                description: description.to_string(),
                tier,
            })
            .await;
    }
}

impl Default for AgentTestFixture {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for creating test agents with sensible defaults.
pub struct TestAgentBuilder {
    id: String,
    name: String,
    team_id: String,
    status: AgentStatus,
    token_budget: Option<TokenBudget>,
    cost_budget: Option<CostBudget>,
    request_budget: Option<RequestBudget>,
}

impl TestAgentBuilder {
    /// Create a new test agent builder.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            team_id: "test-team".to_string(),
            status: AgentStatus::Active,
            token_budget: None,
            cost_budget: None,
            request_budget: None,
        }
    }

    /// Set the team ID.
    pub fn team(mut self, team_id: impl Into<String>) -> Self {
        self.team_id = team_id.into();
        self
    }

    /// Set the agent status.
    pub fn status(mut self, status: AgentStatus) -> Self {
        self.status = status;
        self
    }

    /// Set token budget.
    pub fn token_budget(mut self, per_request: u64, per_hour: u64, per_day: u64) -> Self {
        self.token_budget = Some(
            TokenBudget::new()
                .with_per_request(per_request)
                .with_per_hour(per_hour)
                .with_per_day(per_day),
        );
        self
    }

    /// Set cost budget (in cents).
    pub fn cost_budget(mut self, per_hour: u64, per_day: u64) -> Self {
        self.cost_budget = Some(CostBudget::new().with_per_hour(per_hour).with_per_day(per_day));
        self
    }

    /// Set request budget.
    pub fn request_budget(mut self, per_hour: u32, per_day: u32) -> Self {
        self.request_budget = Some(
            RequestBudget::new()
                .with_per_hour(per_hour)
                .with_per_day(per_day),
        );
        self
    }

    /// Build the agent.
    pub fn build(self) -> Agent {
        let mut budgets = AgentBudgets::new();
        if let Some(token) = self.token_budget {
            budgets = budgets.with_tokens(token);
        }
        if let Some(cost) = self.cost_budget {
            budgets = budgets.with_cost(cost);
        }
        if let Some(request) = self.request_budget {
            budgets = budgets.with_requests(request);
        }

        Agent::builder(AgentId::new(&self.id), &self.name)
            .organization(AgentOrganization::new(TeamId::new(&self.team_id)))
            .status(self.status)
            .budgets(budgets)
            .build()
    }
}

/// Create a simple test agent with minimal configuration.
pub fn simple_agent(id: &str, name: &str) -> Agent {
    TestAgentBuilder::new(id, name).build()
}

/// Create an agent with generous budgets for testing.
pub fn agent_with_budgets(id: &str, name: &str) -> Agent {
    TestAgentBuilder::new(id, name)
        .token_budget(10000, 100000, 1000000)
        .cost_budget(1000, 10000)
        .request_budget(100, 1000)
        .build()
}
