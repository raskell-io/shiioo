use anyhow::Result;
use futures_core::Stream;
use shiioo_core::storage::AgentStore;
use shiioo_core::types::{
    CapacitySource, CapacitySourceId, CostPerToken, DeltaContent, LlmContentBlock, LlmError,
    LlmMessage, LlmMessagesRequest, LlmProvider, LlmRole, RateLimits, StreamEvent,
    ToolDefinition,
};
use std::pin::Pin;

use crate::config::AppState;

use super::renderer::StreamedResponse;

/// Tracks an ongoing conversation with the Chief of Staff.
pub struct Conversation {
    state: AppState,
    system_prompt: String,
    messages: Vec<LlmMessage>,
    model: String,
    session_input_tokens: u32,
    session_output_tokens: u32,
    session_cost: f64,
    has_api_key: bool,
}

impl Conversation {
    /// Create a new conversation backed by the given AppState.
    pub async fn new(state: AppState) -> Result<Self> {
        let system_prompt = build_system_prompt(&state).await?;
        let has_api_key = Self::register_capacity_source(&state);

        Ok(Self {
            state,
            system_prompt,
            messages: Vec::new(),
            model: "claude-sonnet-4-20250514".to_string(),
            session_input_tokens: 0,
            session_output_tokens: 0,
            session_cost: 0.0,
            has_api_key,
        })
    }

    /// Send a message with tools and get a streaming response.
    ///
    /// When `user_input` is `Some`, pushes a user text message first.
    /// When `None`, the messages already contain tool_result blocks — just re-send.
    pub async fn send_streaming_with_tools(
        &mut self,
        user_input: Option<&str>,
        tools: &[ToolDefinition],
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<StreamEvent, LlmError>> + Send>>,
        anyhow::Error,
    > {
        if let Some(input) = user_input {
            self.messages
                .push(LlmMessage::text(LlmRole::User, input));
        }

        if !self.has_api_key {
            let mock = self.mock_response();
            let (tx, rx) = tokio::sync::mpsc::channel(4);
            let _ = tx
                .send(Ok(StreamEvent::ContentBlockDelta {
                    index: 0,
                    delta: DeltaContent::Text(mock),
                }))
                .await;
            let _ = tx.send(Ok(StreamEvent::MessageStop)).await;
            return Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)));
        }

        let request = LlmMessagesRequest {
            system: Some(self.system_prompt.clone()),
            messages: self.messages.clone(),
            max_tokens: 4096,
            temperature: Some(0.7),
            model: Some(self.model.clone()),
            tools: tools.to_vec(),
        };

        match self
            .state
            .capacity_broker
            .execute_messages_request_streaming(request)
            .await
        {
            Ok(stream) => Ok(stream),
            Err(LlmError::AuthenticationFailed) => {
                // Remove the user message we just added (if we added one)
                if user_input.is_some() {
                    self.messages.pop();
                }
                Err(anyhow::anyhow!(
                    "Authentication failed. Check your ANTHROPIC_API_KEY."
                ))
            }
            Err(LlmError::RateLimited { retry_after }) => {
                if user_input.is_some() {
                    self.messages.pop();
                }
                let wait = retry_after.unwrap_or(60);
                Err(anyhow::anyhow!(
                    "Rate limited. Retry after {wait} seconds."
                ))
            }
            Err(e) => {
                if user_input.is_some() {
                    self.messages.pop();
                }
                Err(anyhow::anyhow!("LLM error: {:?}", e))
            }
        }
    }

    /// Record the assistant response after streaming completes.
    ///
    /// Pushes the full content blocks (text + tool_use) as the assistant message.
    pub fn finish_streaming_response(&mut self, response: &StreamedResponse) {
        let content_blocks: Vec<LlmContentBlock> = response
            .content_blocks
            .iter()
            .cloned()
            .collect();

        if !content_blocks.is_empty() {
            self.messages.push(LlmMessage {
                role: LlmRole::Assistant,
                content: content_blocks,
            });
        }

        self.session_input_tokens += response.input_tokens;
        self.session_output_tokens += response.output_tokens;

        let cost = (response.input_tokens as f64 * 3.0
            + response.output_tokens as f64 * 15.0)
            / 1_000_000.0;
        self.session_cost += cost;
    }

    /// Add tool results to the conversation.
    ///
    /// Pushes a user message with ToolResult blocks for each executed tool.
    pub fn add_tool_results(&mut self, results: Vec<(String, String, bool)>) {
        let content: Vec<LlmContentBlock> = results
            .into_iter()
            .map(|(tool_use_id, content, is_error)| LlmContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error: if is_error { Some(true) } else { None },
            })
            .collect();

        self.messages.push(LlmMessage {
            role: LlmRole::User,
            content,
        });
    }

    /// Clear conversation history (keep system prompt).
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Get session usage stats.
    pub fn session_usage(&self) -> (u32, u32, f64) {
        (
            self.session_input_tokens,
            self.session_output_tokens,
            self.session_cost,
        )
    }

    /// Switch model.
    pub fn set_model(&mut self, model: &str) {
        self.model = model.to_string();
    }

    /// Get current model.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Get a reference to the AppState.
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// Register the Anthropic capacity source from env. Returns true if key was found.
    fn register_capacity_source(state: &AppState) -> bool {
        if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
            let source_id = CapacitySourceId::new("anthropic-repl");
            if state.capacity_broker.get_source(&source_id).is_none() {
                let key_preview = format!(
                    "{}...{}",
                    &api_key[..4.min(api_key.len())],
                    &api_key[api_key.len().saturating_sub(4)..]
                );
                let source = CapacitySource {
                    id: source_id,
                    name: "Anthropic (REPL)".to_string(),
                    provider: LlmProvider::Anthropic,
                    api_key_hash: key_preview,
                    model: "claude-sonnet-4-20250514".to_string(),
                    rate_limits: RateLimits {
                        requests_per_minute: 60,
                        tokens_per_minute: 100_000,
                        tokens_per_day: None,
                    },
                    cost_per_token: CostPerToken {
                        input_cost: 3.0,
                        output_cost: 15.0,
                    },
                    priority: 100,
                    enabled: true,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                };
                let _ = state
                    .capacity_broker
                    .register_source_with_key(source, api_key);
            }
            true
        } else {
            false
        }
    }

    /// Generate a mock response when no API key is available.
    fn mock_response(&mut self) -> String {
        let last_user = self
            .messages
            .iter()
            .rev()
            .find(|m| m.role == LlmRole::User)
            .map(|m| m.text_content())
            .unwrap_or_default();

        self.session_input_tokens += last_user.split_whitespace().count() as u32 * 2;
        self.session_output_tokens += 50;

        format!(
            "[Mock response — no ANTHROPIC_API_KEY set]\n\n\
             I received your message: \"{}\"\n\n\
             To get real responses, set your ANTHROPIC_API_KEY environment variable \
             and restart shiioo.",
            truncate(&last_user, 100)
        )
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        &s[..s.floor_char_boundary(max)]
    }
}

/// Build the system prompt from company state.
async fn build_system_prompt(state: &AppState) -> Result<String> {
    let mut parts = Vec::new();

    parts.push(
        "You are the Chief of Staff at this virtual company. \
         You are the CEO's right hand — an analyst, strategist, and executor. \
         The CEO talks to you; you coordinate everything else.\n\
         \n\
         Your responsibilities:\n\
         - Translate the CEO's vision into actionable work\n\
         - Report on company status, employee activity, and progress\n\
         - Delegate tasks to employees (AI agents)\n\
         - Escalate decisions that need CEO approval\n\
         - Maintain the company metaphor at all times\n\
         \n\
         Always refer to agents as \"employees\", agent creation as \"hiring\", \
         archetypes as \"job titles\", and use professional company language.\n\
         \n\
         You have tools available to manage the company. Use them to look up real data \
         rather than guessing. When the CEO asks about employees, teams, budgets, or \
         company status, always use the appropriate tool to get accurate information."
            .to_string(),
    );

    // Add employee roster
    let agents = state.agent_store.list_agents().await.unwrap_or_default();
    if agents.is_empty() {
        parts.push(
            "\nCompany status: No employees have been hired yet. \
             You should suggest the CEO hire some employees to get started."
                .to_string(),
        );
    } else {
        let mut roster = String::from("\nCurrent employees:\n");
        for agent in &agents {
            roster.push_str(&format!(
                "- {} ({}): {} [{}]\n",
                agent.name, agent.id, agent.description, agent.status,
            ));
        }
        parts.push(roster);
    }

    // Add skill registry summary
    let registry = state.skill_registry.read().await;
    let skill_count = registry.list_entries().len();
    if skill_count > 0 {
        parts.push(format!("\nAvailable skills in registry: {skill_count}"));
    }

    Ok(parts.join("\n"))
}
