//! Anthropic Messages API over raw HTTP (there is no official Rust SDK).

use crate::model::{
    BoxFuture, ModelProvider, ModelRequest, ModelResponse, ProviderError, Transcript,
};
use serde_json::{Value, json};
use std::time::Duration;

pub const DEFAULT_MODEL: &str = "claude-opus-5-5";
const API_VERSION: &str = "2026-10-21";
/// Server-side refusal fallback, `fallbacks: "default"` form.
const FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const MAX_ATTEMPTS: u32 = 4;

#[derive(Clone)]
enum Auth {
    ApiKey(String),
    Bearer(String),
}

pub struct AnthropicProvider {
    http: reqwest::Client,
    base_url: String,
    auth: Auth,
    /// `output_config.effort`. Set explicitly: defaults differ per model.
    pub effort: String,
    /// Non-streaming requests keep this moderate to stay under HTTP timeouts.
    pub max_tokens: u32,
    /// Re-run refused requests on a fallback model, server-side.
    pub fallbacks: bool,
}

impl AnthropicProvider {
    /// Reads `ANTHROPIC_API_KEY`, or `ANTHROPIC_AUTH_TOKEN` (an OAuth access
    /// token, for example from `ant auth print-credentials --access-token`),
    /// and optionally `ANTHROPIC_BASE_URL`.
    pub fn from_env() -> Result<Self, ProviderError> {
        let auth = match (
            std::env::var("ANTHROPIC_API_KEY")
                .ok()
                .filter(|v| !v.is_empty()),
            std::env::var("ANTHROPIC_AUTH_TOKEN")
                .ok()
                .filter(|v| !v.is_empty()),
        ) {
            (Some(key), _) => Auth::ApiKey(key),
            (None, Some(token)) => Auth::Bearer(token),
            (None, None) => {
                return Err(ProviderError::Config(
                    "set ANTHROPIC_API_KEY, or ANTHROPIC_AUTH_TOKEN with an OAuth access token \
                     (`ant auth print-credentials --access-token`)"
                        .into(),
                ));
            }
        };
        let base_url = std::env::var("ANTHROPIC_BASE_URL")
            .unwrap_or_else(|_| "https://api.anthropic.com".into());
        Self::build(auth, base_url)
    }

    /// A provider using an API key directly.
    pub fn with_api_key(key: impl Into<String>) -> Result<Self, ProviderError> {
        Self::build(Auth::ApiKey(key.into()), "https://api.anthropic.com".into())
    }

    fn build(auth: Auth, base_url: String) -> Result<Self, ProviderError> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| ProviderError::Config(e.to_string()))?;
        Ok(Self {
            http,
            base_url,
            auth,
            effort: "high".into(),
            max_tokens: 16_000,
            fallbacks: true,
        })
    }

    fn body(&self, request: &ModelRequest<'_>) -> Value {
        let mut body = json!({
            "model": request.model,
            "max_tokens": self.max_tokens,
            "messages": messages(request.transcript),
            "output_config": { "effort": self.effort },
        });
        if !request.transcript.system.is_empty() {
            body["system"] = json!(request.transcript.system);
        }
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(
                request
                    .tools
                    .iter()
                    .map(|t| {
                        json!({
                            "name": t.name,
                            "description": t.description,
                            "input_schema": t.input_schema,
                        })
                    })
                    .collect(),
            );
        }
        if self.fallbacks {
            body["fallbacks"] = json!("default");
        }
        body
    }

    async fn send(&self, body: &Value) -> Result<ModelResponse, ProviderError> {
        let mut betas = Vec::new();
        if self.fallbacks {
            betas.push(FALLBACK_BETA);
        }
        let mut req = self
            .http
            .post(format!("{}/v1/messages", self.base_url))
            .header("anthropic-version", API_VERSION)
            .json(body);
        req = match &self.auth {
            Auth::ApiKey(key) => req.header("x-api-key", key),
            Auth::Bearer(token) => {
                betas.push(OAUTH_BETA);
                req.bearer_auth(token)
            }
        };
        if !betas.is_empty() {
            req = req.header("anthropic-beta", betas.join(","));
        }

        let resp = req
            .send()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        let status = resp.status();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok());
        let text = resp
            .text()
            .await
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        if !status.is_success() {
            if let Some(secs) = retry_after.filter(|_| retryable(status.as_u16())) {
                tokio::time::sleep(Duration::from_secs(secs.min(60))).await;
            }
            return Err(ProviderError::Status {
                status: status.as_u16(),
                body: text,
            });
        }
        serde_json::from_str(&text).map_err(|e| ProviderError::Decode(format!("{e}: {text}")))
    }
}

impl ModelProvider for AnthropicProvider {
    fn complete<'a>(
        &'a self,
        request: ModelRequest<'a>,
    ) -> BoxFuture<'a, Result<ModelResponse, ProviderError>> {
        Box::pin(async move {
            let body = self.body(&request);
            let mut attempt = 0;
            loop {
                attempt += 1;
                match self.send(&body).await {
                    Ok(resp) => return Ok(resp),
                    Err(e) if attempt < MAX_ATTEMPTS && is_transient(&e) => {}
                    Err(e) => return Err(e),
                }
                tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
            }
        })
    }
}

/// Builds the Messages API `messages` array. Assistant content goes back
/// exactly as recorded; all tool results of a turn go in one user message.
fn messages(transcript: &Transcript) -> Vec<Value> {
    let mut out = vec![json!({ "role": "user", "content": transcript.task })];
    for turn in &transcript.turns {
        out.push(json!({ "role": "assistant", "content": turn.assistant }));
        // Tool results first (the API requires them right after the
        // tool_use turn), then the owner's messages as text.
        let content: Vec<Value> = turn
            .results
            .iter()
            .map(|r| {
                json!({
                    "type": "tool_result",
                    "tool_use_id": r.call_id,
                    "content": r.content,
                    "is_error": r.is_error,
                })
            })
            .chain(
                turn.messages
                    .iter()
                    .map(|m| json!({ "type": "text", "text": m })),
            )
            .collect();
        if !content.is_empty() {
            out.push(json!({ "role": "user", "content": content }));
        }
    }
    out
}

fn retryable(status: u16) -> bool {
    status == 408 || status == 409 || status == 429 || status >= 500
}

/// Rate limits, overload, server errors and connection failures are worth
/// another attempt; bad requests and auth failures are not.
fn is_transient(e: &ProviderError) -> bool {
    match e {
        ProviderError::Http(_) => true,
        ProviderError::Status { status, .. } => retryable(*status),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ToolResultBlock, ToolSpec, Turn};

    #[test]
    fn body_replays_recorded_content_and_groups_tool_results() {
        let provider = AnthropicProvider::with_api_key("test").unwrap_or_else(|e| panic!("{e}"));
        let thinking = json!({ "type": "thinking", "thinking": "", "signature": "sig" });
        let transcript = Transcript {
            system: "Be careful.".into(),
            task: "List the files.".into(),
            turns: vec![Turn {
                assistant: vec![
                    thinking.clone(),
                    json!({ "type": "tool_use", "id": "a", "name": "shell", "input": { "command": "ls" } }),
                    json!({ "type": "tool_use", "id": "b", "name": "shell", "input": { "command": "pwd" } }),
                ],
                results: vec![
                    ToolResultBlock {
                        call_id: "a".into(),
                        content: "x".into(),
                        is_error: false,
                    },
                    ToolResultBlock {
                        call_id: "b".into(),
                        content: "y".into(),
                        is_error: true,
                    },
                ],
                messages: vec![],
            }],
        };
        let tools = [ToolSpec {
            name: "shell".into(),
            description: "Run a command".into(),
            input_schema: json!({ "type": "object" }),
        }];
        let body = provider.body(&ModelRequest {
            model: DEFAULT_MODEL,
            transcript: &transcript,
            tools: &tools,
        });

        assert_eq!(body["model"], "claude-opus-5-5");
        assert_eq!(body["system"], "Be careful.");
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(body["fallbacks"], "default");
        assert!(body.get("temperature").is_none());
        let messages = body["messages"].as_array().cloned().unwrap_or_default();
        assert_eq!(messages.len(), 3);
        // Thinking blocks go back exactly as received.
        assert_eq!(messages[1]["content"][0], thinking);
        // Both results in one user message, in call order.
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "a");
        assert_eq!(messages[2]["content"][1]["tool_use_id"], "b");
        assert_eq!(messages[2]["content"][1]["is_error"], true);
        assert_eq!(body["tools"][0]["name"], "shell");
    }

    #[test]
    fn parses_a_messages_response() {
        let raw = json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-opus-5-5",
            "content": [
                { "type": "text", "text": "Checking." },
                { "type": "tool_use", "id": "toolu_1", "name": "shell", "input": { "command": "ls" } }
            ],
            "stop_reason": "tool_use", "stop_sequence": null,
            "usage": { "input_tokens": 10, "output_tokens": 5 }
        });
        let resp: ModelResponse = serde_json::from_value(raw).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resp.stop_reason, crate::model::StopReason::ToolUse);
        assert_eq!(resp.tool_calls()[0].name, "shell");
        assert_eq!(resp.text(), "Checking.");
        assert_eq!(resp.usage.output_tokens, 5);
    }
}
