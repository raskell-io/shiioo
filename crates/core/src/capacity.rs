use crate::types::{
    CapacitySource, CapacitySourceId, CapacityUsage, LlmError, LlmRequest, LlmResponse,
    PriorityRequest, RateLimitState, RoleId, RunId, StepId,
};
use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use std::collections::{BinaryHeap, HashMap};
use std::sync::{Arc, Mutex};
use tokio::time::sleep;

/// Capacity broker for multi-source LLM capacity pooling
pub struct CapacityBroker {
    sources: Arc<Mutex<HashMap<CapacitySourceId, CapacitySource>>>,
    rate_limits: Arc<Mutex<HashMap<CapacitySourceId, RateLimitState>>>,
    usage_history: Arc<Mutex<Vec<CapacityUsage>>>,
    priority_queue: Arc<Mutex<BinaryHeap<PriorityRequestWrapper>>>,
}

/// Wrapper for PriorityRequest to implement Ord for BinaryHeap
#[derive(Clone)]
struct PriorityRequestWrapper(PriorityRequest);

impl PartialEq for PriorityRequestWrapper {
    fn eq(&self, other: &Self) -> bool {
        self.0.priority == other.0.priority && self.0.id == other.0.id
    }
}

impl Eq for PriorityRequestWrapper {}

impl PartialOrd for PriorityRequestWrapper {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PriorityRequestWrapper {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Higher priority first, then older requests first
        self.0
            .priority
            .cmp(&other.0.priority)
            .then_with(|| other.0.created_at.cmp(&self.0.created_at))
    }
}

impl CapacityBroker {
    /// Create a new capacity broker
    pub fn new() -> Self {
        Self {
            sources: Arc::new(Mutex::new(HashMap::new())),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            usage_history: Arc::new(Mutex::new(Vec::new())),
            priority_queue: Arc::new(Mutex::new(BinaryHeap::new())),
        }
    }

    /// Register a capacity source
    pub fn register_source(&self, source: CapacitySource) -> Result<()> {
        let source_id = source.id.clone();

        // Initialize rate limit state
        let state = RateLimitState {
            source_id: source_id.clone(),
            window_start: Utc::now(),
            requests_in_window: 0,
            tokens_in_window: 0,
            daily_tokens: 0,
            daily_reset_at: Utc::now() + Duration::days(1),
            next_available: None,
            backoff_until: None,
        };

        self.sources
            .lock()
            .unwrap()
            .insert(source_id.clone(), source);

        self.rate_limits
            .lock()
            .unwrap()
            .insert(source_id, state);

        Ok(())
    }

    /// Remove a capacity source
    pub fn remove_source(&self, source_id: &CapacitySourceId) -> Result<()> {
        self.sources.lock().unwrap().remove(source_id);
        self.rate_limits.lock().unwrap().remove(source_id);
        Ok(())
    }

    /// Get all registered sources
    pub fn list_sources(&self) -> Vec<CapacitySource> {
        self.sources.lock().unwrap().values().cloned().collect()
    }

    /// Get a specific source
    pub fn get_source(&self, source_id: &CapacitySourceId) -> Option<CapacitySource> {
        self.sources.lock().unwrap().get(source_id).cloned()
    }

    /// Update source enabled status
    pub fn update_source_enabled(&self, source_id: &CapacitySourceId, enabled: bool) -> Result<()> {
        if let Some(source) = self.sources.lock().unwrap().get_mut(source_id) {
            source.enabled = enabled;
            source.updated_at = Utc::now();
            Ok(())
        } else {
            Err(anyhow::anyhow!("Source not found"))
        }
    }

    /// Select the best available source for a request
    pub fn select_source(&self, required_tokens: u32) -> Option<CapacitySourceId> {
        let sources = self.sources.lock().unwrap();
        let mut rate_limits = self.rate_limits.lock().unwrap();
        let now = Utc::now();

        // Filter and sort sources by priority
        let mut candidates: Vec<_> = sources
            .values()
            .filter(|s| s.enabled)
            .collect();

        candidates.sort_by(|a, b| b.priority.cmp(&a.priority));

        // Find first available source
        for source in candidates {
            let state = rate_limits.get_mut(&source.id)?;

            // Check if in backoff period
            if let Some(backoff_until) = state.backoff_until {
                if now < backoff_until {
                    continue;
                }
                // Clear backoff
                state.backoff_until = None;
            }

            // Reset window if needed
            if now >= state.window_start + Duration::minutes(1) {
                state.window_start = now;
                state.requests_in_window = 0;
                state.tokens_in_window = 0;
            }

            // Reset daily counter if needed
            if now >= state.daily_reset_at {
                state.daily_tokens = 0;
                state.daily_reset_at = now + Duration::days(1);
            }

            // Check rate limits
            if state.requests_in_window >= source.rate_limits.requests_per_minute {
                continue;
            }
            if state.tokens_in_window + required_tokens > source.rate_limits.tokens_per_minute {
                continue;
            }
            if let Some(daily_limit) = source.rate_limits.tokens_per_day {
                if state.daily_tokens + required_tokens > daily_limit {
                    continue;
                }
            }

            return Some(source.id.clone());
        }

        None
    }

    /// Execute an LLM request with automatic retry and fallback
    pub async fn execute_request(
        &self,
        request: LlmRequest,
        run_id: RunId,
        step_id: StepId,
        role: RoleId,
        priority: u8,
    ) -> Result<LlmResponse> {
        let required_tokens = request.max_tokens;

        // Try to select a source
        if let Some(source_id) = self.select_source(required_tokens) {
            match self.execute_with_source(&source_id, &request, run_id, step_id.clone()).await {
                Ok(response) => return Ok(response),
                Err(LlmError::RateLimited { retry_after }) => {
                    // Apply backoff
                    self.apply_backoff(&source_id, retry_after);
                    // Fallback to queue
                }
                Err(err) => {
                    tracing::warn!("LLM request failed: {:?}", err);
                    // Try next source
                }
            }
        }

        // No source available, queue the request
        self.enqueue_request(PriorityRequest {
            id: uuid::Uuid::new_v4().to_string(),
            priority,
            run_id,
            step_id,
            role,
            prompt: request.prompt,
            max_tokens: request.max_tokens,
            created_at: Utc::now(),
            attempts: 0,
        });

        Err(anyhow::anyhow!("No capacity available, request queued"))
    }

    /// Execute a request with a specific source
    async fn execute_with_source(
        &self,
        source_id: &CapacitySourceId,
        request: &LlmRequest,
        run_id: RunId,
        step_id: StepId,
    ) -> Result<LlmResponse, LlmError> {
        let source = self.sources.lock().unwrap()
            .get(source_id)
            .cloned()
            .ok_or(LlmError::Other { message: "Source not found".to_string() })?;

        // Update rate limit state
        {
            let mut rate_limits = self.rate_limits.lock().unwrap();
            if let Some(state) = rate_limits.get_mut(source_id) {
                state.requests_in_window += 1;
                state.tokens_in_window += request.max_tokens;
                state.daily_tokens += request.max_tokens;
            }
        }

        // Simulate LLM API call (in production, this would call the actual API)
        let response = self.call_llm_api(&source, request).await?;

        // Track usage
        let usage = CapacityUsage {
            id: uuid::Uuid::new_v4().to_string(),
            source_id: source_id.clone(),
            timestamp: Utc::now(),
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
            total_tokens: response.input_tokens + response.output_tokens,
            cost: response.cost,
            request_count: 1,
            run_id: Some(run_id),
            step_id: Some(step_id),
        };

        self.usage_history.lock().unwrap().push(usage);

        Ok(response)
    }

    /// Call the LLM API (stub for MVP, would integrate with actual APIs)
    async fn call_llm_api(
        &self,
        source: &CapacitySource,
        request: &LlmRequest,
    ) -> Result<LlmResponse, LlmError> {
        // Simulate API latency
        sleep(tokio::time::Duration::from_millis(100)).await;

        // For MVP, return a mock response
        // In production, this would call the actual LLM API
        let input_tokens = request.prompt.split_whitespace().count() as u32 * 2;
        let output_tokens = request.max_tokens / 2; // Assume we use half the max
        let cost = (input_tokens as f64 * source.cost_per_token.input_cost
                  + output_tokens as f64 * source.cost_per_token.output_cost) / 1_000_000.0;

        Ok(LlmResponse {
            text: format!("Response from {} using {}", source.name, source.model),
            input_tokens,
            output_tokens,
            cost,
            model: source.model.clone(),
            source_id: source.id.clone(),
        })
    }

    /// Apply exponential backoff to a source
    fn apply_backoff(&self, source_id: &CapacitySourceId, retry_after: Option<u64>) {
        let mut rate_limits = self.rate_limits.lock().unwrap();
        if let Some(state) = rate_limits.get_mut(source_id) {
            let backoff_secs = retry_after.unwrap_or(60); // Default 60s
            state.backoff_until = Some(Utc::now() + Duration::seconds(backoff_secs as i64));
            tracing::info!(
                "Applied backoff to source {}: retry in {}s",
                source_id.0,
                backoff_secs
            );
        }
    }

    /// Enqueue a request with priority
    fn enqueue_request(&self, request: PriorityRequest) {
        self.priority_queue
            .lock()
            .unwrap()
            .push(PriorityRequestWrapper(request));
    }

    /// Get the next request from the queue
    pub fn dequeue_request(&self) -> Option<PriorityRequest> {
        self.priority_queue.lock().unwrap().pop().map(|w| w.0)
    }

    /// Get queue length
    pub fn queue_length(&self) -> usize {
        self.priority_queue.lock().unwrap().len()
    }

    /// Get total usage for a source
    pub fn get_source_usage(&self, source_id: &CapacitySourceId, since: DateTime<Utc>) -> Vec<CapacityUsage> {
        self.usage_history
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.source_id == *source_id && u.timestamp >= since)
            .cloned()
            .collect()
    }

    /// Get total cost for a source
    pub fn get_source_cost(&self, source_id: &CapacitySourceId, since: DateTime<Utc>) -> f64 {
        self.usage_history
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.source_id == *source_id && u.timestamp >= since)
            .map(|u| u.cost)
            .sum()
    }

    /// Get all usage since a timestamp
    pub fn get_all_usage(&self, since: DateTime<Utc>) -> Vec<CapacityUsage> {
        self.usage_history
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.timestamp >= since)
            .cloned()
            .collect()
    }

    /// Get total cost across all sources
    pub fn get_total_cost(&self, since: DateTime<Utc>) -> f64 {
        self.usage_history
            .lock()
            .unwrap()
            .iter()
            .filter(|u| u.timestamp >= since)
            .map(|u| u.cost)
            .sum()
    }

    /// Get rate limit state for a source
    pub fn get_rate_limit_state(&self, source_id: &CapacitySourceId) -> Option<RateLimitState> {
        self.rate_limits.lock().unwrap().get(source_id).cloned()
    }
}

impl Default for CapacityBroker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{CostPerToken, LlmProvider, RateLimits};

    fn create_test_source(id: &str, priority: u8) -> CapacitySource {
        CapacitySource {
            id: CapacitySourceId::new(id),
            name: format!("Test Source {}", id),
            provider: LlmProvider::Anthropic,
            api_key_hash: "hash123".to_string(),
            model: "claude-opus-4".to_string(),
            rate_limits: RateLimits {
                requests_per_minute: 60,
                tokens_per_minute: 100_000,
                tokens_per_day: Some(1_000_000),
            },
            cost_per_token: CostPerToken {
                input_cost: 15.0,
                output_cost: 75.0,
            },
            priority,
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_register_source() {
        let broker = CapacityBroker::new();
        let source = create_test_source("src1", 100);

        broker.register_source(source.clone()).unwrap();

        let sources = broker.list_sources();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].id, source.id);
    }

    #[test]
    fn test_select_source_by_priority() {
        let broker = CapacityBroker::new();

        broker.register_source(create_test_source("low", 10)).unwrap();
        broker.register_source(create_test_source("high", 100)).unwrap();
        broker.register_source(create_test_source("medium", 50)).unwrap();

        let selected = broker.select_source(1000).unwrap();
        assert_eq!(selected.0, "high");
    }

    #[test]
    fn test_rate_limit_enforcement() {
        let broker = CapacityBroker::new();

        let mut source = create_test_source("src1", 100);
        source.rate_limits.tokens_per_minute = 1000;
        broker.register_source(source).unwrap();

        // Should succeed
        let selected = broker.select_source(500);
        assert!(selected.is_some());

        // Consume tokens
        {
            let mut rate_limits = broker.rate_limits.lock().unwrap();
            if let Some(state) = rate_limits.get_mut(&CapacitySourceId::new("src1")) {
                state.tokens_in_window = 600;
            }
        }

        // Should fail (600 + 500 > 1000)
        let selected = broker.select_source(500);
        assert!(selected.is_none());

        // Should succeed with smaller request
        let selected = broker.select_source(300);
        assert!(selected.is_some());
    }

    #[test]
    fn test_priority_queue() {
        let broker = CapacityBroker::new();

        let req1 = PriorityRequest {
            id: "1".to_string(),
            priority: 10,
            run_id: RunId::new(),
            step_id: StepId::new("step1"),
            role: RoleId::new("analyst"),
            prompt: "Low priority".to_string(),
            max_tokens: 1000,
            created_at: Utc::now(),
            attempts: 0,
        };

        let req2 = PriorityRequest {
            id: "2".to_string(),
            priority: 100,
            run_id: RunId::new(),
            step_id: StepId::new("step2"),
            role: RoleId::new("engineer"),
            prompt: "High priority".to_string(),
            max_tokens: 1000,
            created_at: Utc::now(),
            attempts: 0,
        };

        broker.enqueue_request(req1);
        broker.enqueue_request(req2);

        assert_eq!(broker.queue_length(), 2);

        // High priority should come out first
        let dequeued = broker.dequeue_request().unwrap();
        assert_eq!(dequeued.priority, 100);
        assert_eq!(dequeued.id, "2");

        assert_eq!(broker.queue_length(), 1);
    }

    #[tokio::test]
    async fn test_execute_request() {
        let broker = CapacityBroker::new();
        broker.register_source(create_test_source("src1", 100)).unwrap();

        let request = LlmRequest {
            prompt: "Test prompt".to_string(),
            max_tokens: 1000,
            temperature: Some(0.7),
            model: None,
        };

        let response = broker
            .execute_request(
                request,
                RunId::new(),
                StepId::new("step1"),
                RoleId::new("analyst"),
                50,
            )
            .await
            .unwrap();

        assert!(response.input_tokens > 0);
        assert!(response.output_tokens > 0);
        assert!(response.cost > 0.0);
    }

    #[test]
    fn test_usage_tracking() {
        let broker = CapacityBroker::new();
        let source_id = CapacitySourceId::new("src1");

        let usage = CapacityUsage {
            id: uuid::Uuid::new_v4().to_string(),
            source_id: source_id.clone(),
            timestamp: Utc::now(),
            input_tokens: 100,
            output_tokens: 200,
            total_tokens: 300,
            cost: 0.05,
            request_count: 1,
            run_id: Some(RunId::new()),
            step_id: Some(StepId::new("step1")),
        };

        broker.usage_history.lock().unwrap().push(usage);

        let one_hour_ago = Utc::now() - Duration::hours(1);
        let cost = broker.get_source_cost(&source_id, one_hour_ago);
        assert_eq!(cost, 0.05);

        let total_cost = broker.get_total_cost(one_hour_ago);
        assert_eq!(total_cost, 0.05);
    }

    #[test]
    fn test_backoff() {
        let broker = CapacityBroker::new();
        let source_id = CapacitySourceId::new("src1");

        broker.register_source(create_test_source("src1", 100)).unwrap();
        broker.apply_backoff(&source_id, Some(120));

        let state = broker.get_rate_limit_state(&source_id).unwrap();
        assert!(state.backoff_until.is_some());

        let backoff_until = state.backoff_until.unwrap();
        assert!(backoff_until > Utc::now());
    }
}
