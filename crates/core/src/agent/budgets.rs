//! Agent resource budgets.
//!
//! Budgets define resource limits for an agent including tokens, cost,
//! request rate, and concurrency limits.

use serde::{Deserialize, Serialize};

/// Resource budgets for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentBudgets {
    /// Token limits.
    pub tokens: TokenBudget,

    /// Cost limits.
    pub cost: CostBudget,

    /// Request limits.
    pub requests: RequestBudget,

    /// Concurrent execution limits.
    pub concurrency: ConcurrencyBudget,
}

impl Default for AgentBudgets {
    fn default() -> Self {
        Self {
            tokens: TokenBudget::default(),
            cost: CostBudget::default(),
            requests: RequestBudget::default(),
            concurrency: ConcurrencyBudget::default(),
        }
    }
}

impl AgentBudgets {
    /// Create new budgets with defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set token budgets.
    pub fn with_tokens(mut self, tokens: TokenBudget) -> Self {
        self.tokens = tokens;
        self
    }

    /// Set cost budgets.
    pub fn with_cost(mut self, cost: CostBudget) -> Self {
        self.cost = cost;
        self
    }

    /// Set request budgets.
    pub fn with_requests(mut self, requests: RequestBudget) -> Self {
        self.requests = requests;
        self
    }

    /// Set concurrency budgets.
    pub fn with_concurrency(mut self, concurrency: ConcurrencyBudget) -> Self {
        self.concurrency = concurrency;
        self
    }

    /// Create unlimited budgets (for testing or admin agents).
    pub fn unlimited() -> Self {
        Self {
            tokens: TokenBudget::unlimited(),
            cost: CostBudget::unlimited(),
            requests: RequestBudget::unlimited(),
            concurrency: ConcurrencyBudget::unlimited(),
        }
    }
}

/// Token limits for an agent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TokenBudget {
    /// Maximum tokens per request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_request: Option<u64>,

    /// Maximum tokens per hour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_hour: Option<u64>,

    /// Maximum tokens per day.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_day: Option<u64>,

    /// Maximum tokens per month.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_month: Option<u64>,
}

impl TokenBudget {
    /// Create a new token budget.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the per-request limit.
    pub fn with_per_request(mut self, limit: u64) -> Self {
        self.per_request = Some(limit);
        self
    }

    /// Set the per-hour limit.
    pub fn with_per_hour(mut self, limit: u64) -> Self {
        self.per_hour = Some(limit);
        self
    }

    /// Set the per-day limit.
    pub fn with_per_day(mut self, limit: u64) -> Self {
        self.per_day = Some(limit);
        self
    }

    /// Set the per-month limit.
    pub fn with_per_month(mut self, limit: u64) -> Self {
        self.per_month = Some(limit);
        self
    }

    /// Create an unlimited token budget.
    pub fn unlimited() -> Self {
        Self {
            per_request: None,
            per_hour: None,
            per_day: None,
            per_month: None,
        }
    }

    /// Check if any limits are set.
    pub fn has_limits(&self) -> bool {
        self.per_request.is_some()
            || self.per_hour.is_some()
            || self.per_day.is_some()
            || self.per_month.is_some()
    }
}

/// Cost limits for an agent (in cents).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CostBudget {
    /// Maximum cost per request (cents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_request_cents: Option<u64>,

    /// Maximum cost per hour (cents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_hour_cents: Option<u64>,

    /// Maximum cost per day (cents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_day_cents: Option<u64>,

    /// Maximum cost per month (cents).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_month_cents: Option<u64>,
}

impl CostBudget {
    /// Create a new cost budget.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the per-request limit.
    pub fn with_per_request(mut self, cents: u64) -> Self {
        self.per_request_cents = Some(cents);
        self
    }

    /// Set the per-hour limit.
    pub fn with_per_hour(mut self, cents: u64) -> Self {
        self.per_hour_cents = Some(cents);
        self
    }

    /// Set the per-day limit.
    pub fn with_per_day(mut self, cents: u64) -> Self {
        self.per_day_cents = Some(cents);
        self
    }

    /// Set the per-month limit.
    pub fn with_per_month(mut self, cents: u64) -> Self {
        self.per_month_cents = Some(cents);
        self
    }

    /// Create an unlimited cost budget.
    pub fn unlimited() -> Self {
        Self {
            per_request_cents: None,
            per_hour_cents: None,
            per_day_cents: None,
            per_month_cents: None,
        }
    }

    /// Check if any limits are set.
    pub fn has_limits(&self) -> bool {
        self.per_request_cents.is_some()
            || self.per_hour_cents.is_some()
            || self.per_day_cents.is_some()
            || self.per_month_cents.is_some()
    }

    /// Get the daily limit in dollars.
    pub fn daily_dollars(&self) -> Option<f64> {
        self.per_day_cents.map(|c| c as f64 / 100.0)
    }

    /// Get the monthly limit in dollars.
    pub fn monthly_dollars(&self) -> Option<f64> {
        self.per_month_cents.map(|c| c as f64 / 100.0)
    }
}

/// Request rate limits for an agent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RequestBudget {
    /// Maximum requests per minute.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_minute: Option<u32>,

    /// Maximum requests per hour.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_hour: Option<u32>,

    /// Maximum requests per day.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_day: Option<u32>,
}

impl RequestBudget {
    /// Create a new request budget.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the per-minute limit.
    pub fn with_per_minute(mut self, limit: u32) -> Self {
        self.per_minute = Some(limit);
        self
    }

    /// Set the per-hour limit.
    pub fn with_per_hour(mut self, limit: u32) -> Self {
        self.per_hour = Some(limit);
        self
    }

    /// Set the per-day limit.
    pub fn with_per_day(mut self, limit: u32) -> Self {
        self.per_day = Some(limit);
        self
    }

    /// Create an unlimited request budget.
    pub fn unlimited() -> Self {
        Self {
            per_minute: None,
            per_hour: None,
            per_day: None,
        }
    }

    /// Check if any limits are set.
    pub fn has_limits(&self) -> bool {
        self.per_minute.is_some() || self.per_hour.is_some() || self.per_day.is_some()
    }
}

/// Concurrency limits for an agent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConcurrencyBudget {
    /// Maximum concurrent workflow steps.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent_steps: Option<u32>,

    /// Maximum concurrent tool calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent_tools: Option<u32>,

    /// Maximum concurrent workflows (runs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_concurrent_workflows: Option<u32>,
}

impl ConcurrencyBudget {
    /// Create a new concurrency budget.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the max concurrent steps.
    pub fn with_max_steps(mut self, limit: u32) -> Self {
        self.max_concurrent_steps = Some(limit);
        self
    }

    /// Set the max concurrent tools.
    pub fn with_max_tools(mut self, limit: u32) -> Self {
        self.max_concurrent_tools = Some(limit);
        self
    }

    /// Set the max concurrent workflows.
    pub fn with_max_workflows(mut self, limit: u32) -> Self {
        self.max_concurrent_workflows = Some(limit);
        self
    }

    /// Create an unlimited concurrency budget.
    pub fn unlimited() -> Self {
        Self {
            max_concurrent_steps: None,
            max_concurrent_tools: None,
            max_concurrent_workflows: None,
        }
    }

    /// Check if any limits are set.
    pub fn has_limits(&self) -> bool {
        self.max_concurrent_steps.is_some()
            || self.max_concurrent_tools.is_some()
            || self.max_concurrent_workflows.is_some()
    }
}

/// Budget usage tracking.
#[derive(Debug, Clone, Default)]
pub struct BudgetUsageStats {
    /// Tokens used in the current hour.
    pub tokens_this_hour: u64,

    /// Tokens used today.
    pub tokens_today: u64,

    /// Tokens used this month.
    pub tokens_this_month: u64,

    /// Cost incurred in the current hour (cents).
    pub cost_this_hour_cents: u64,

    /// Cost incurred today (cents).
    pub cost_today_cents: u64,

    /// Cost incurred this month (cents).
    pub cost_this_month_cents: u64,

    /// Requests made in the current minute.
    pub requests_this_minute: u32,

    /// Requests made in the current hour.
    pub requests_this_hour: u32,

    /// Requests made today.
    pub requests_today: u32,

    /// Current concurrent steps.
    pub current_concurrent_steps: u32,

    /// Current concurrent tool calls.
    pub current_concurrent_tools: u32,

    /// Current concurrent workflows.
    pub current_concurrent_workflows: u32,
}

impl BudgetUsageStats {
    /// Check if token usage exceeds the budget.
    pub fn exceeds_token_budget(&self, budget: &TokenBudget) -> Option<&'static str> {
        if let Some(limit) = budget.per_hour {
            if self.tokens_this_hour >= limit {
                return Some("hourly token limit");
            }
        }
        if let Some(limit) = budget.per_day {
            if self.tokens_today >= limit {
                return Some("daily token limit");
            }
        }
        if let Some(limit) = budget.per_month {
            if self.tokens_this_month >= limit {
                return Some("monthly token limit");
            }
        }
        None
    }

    /// Check if cost exceeds the budget.
    pub fn exceeds_cost_budget(&self, budget: &CostBudget) -> Option<&'static str> {
        if let Some(limit) = budget.per_hour_cents {
            if self.cost_this_hour_cents >= limit {
                return Some("hourly cost limit");
            }
        }
        if let Some(limit) = budget.per_day_cents {
            if self.cost_today_cents >= limit {
                return Some("daily cost limit");
            }
        }
        if let Some(limit) = budget.per_month_cents {
            if self.cost_this_month_cents >= limit {
                return Some("monthly cost limit");
            }
        }
        None
    }

    /// Check if request rate exceeds the budget.
    pub fn exceeds_request_budget(&self, budget: &RequestBudget) -> Option<&'static str> {
        if let Some(limit) = budget.per_minute {
            if self.requests_this_minute >= limit {
                return Some("per-minute request limit");
            }
        }
        if let Some(limit) = budget.per_hour {
            if self.requests_this_hour >= limit {
                return Some("hourly request limit");
            }
        }
        if let Some(limit) = budget.per_day {
            if self.requests_today >= limit {
                return Some("daily request limit");
            }
        }
        None
    }

    /// Check if concurrency exceeds the budget.
    pub fn exceeds_concurrency_budget(&self, budget: &ConcurrencyBudget) -> Option<&'static str> {
        if let Some(limit) = budget.max_concurrent_steps {
            if self.current_concurrent_steps >= limit {
                return Some("concurrent steps limit");
            }
        }
        if let Some(limit) = budget.max_concurrent_tools {
            if self.current_concurrent_tools >= limit {
                return Some("concurrent tools limit");
            }
        }
        if let Some(limit) = budget.max_concurrent_workflows {
            if self.current_concurrent_workflows >= limit {
                return Some("concurrent workflows limit");
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_budget() {
        let budget = TokenBudget::new()
            .with_per_request(100_000)
            .with_per_day(5_000_000);

        assert_eq!(budget.per_request, Some(100_000));
        assert_eq!(budget.per_day, Some(5_000_000));
        assert!(budget.has_limits());
    }

    #[test]
    fn test_cost_budget_dollars() {
        let budget = CostBudget::new()
            .with_per_day(5000) // $50
            .with_per_month(100_000); // $1000

        assert_eq!(budget.daily_dollars(), Some(50.0));
        assert_eq!(budget.monthly_dollars(), Some(1000.0));
    }

    #[test]
    fn test_agent_budgets_builder() {
        let budgets = AgentBudgets::new()
            .with_tokens(TokenBudget::new().with_per_day(1_000_000))
            .with_cost(CostBudget::new().with_per_day(5000))
            .with_requests(RequestBudget::new().with_per_minute(30))
            .with_concurrency(ConcurrencyBudget::new().with_max_steps(5));

        assert_eq!(budgets.tokens.per_day, Some(1_000_000));
        assert_eq!(budgets.cost.per_day_cents, Some(5000));
        assert_eq!(budgets.requests.per_minute, Some(30));
        assert_eq!(budgets.concurrency.max_concurrent_steps, Some(5));
    }

    #[test]
    fn test_unlimited_budgets() {
        let budgets = AgentBudgets::unlimited();

        assert!(!budgets.tokens.has_limits());
        assert!(!budgets.cost.has_limits());
        assert!(!budgets.requests.has_limits());
        assert!(!budgets.concurrency.has_limits());
    }

    #[test]
    fn test_budget_usage_check() {
        let budget = TokenBudget::new().with_per_day(1000);

        let mut usage = BudgetUsageStats::default();
        assert!(usage.exceeds_token_budget(&budget).is_none());

        usage.tokens_today = 1000;
        assert_eq!(
            usage.exceeds_token_budget(&budget),
            Some("daily token limit")
        );
    }

    #[test]
    fn test_cost_budget_usage_check() {
        let budget = CostBudget::new().with_per_hour(100);

        let mut usage = BudgetUsageStats::default();
        assert!(usage.exceeds_cost_budget(&budget).is_none());

        usage.cost_this_hour_cents = 100;
        assert_eq!(
            usage.exceeds_cost_budget(&budget),
            Some("hourly cost limit")
        );
    }
}
