use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A tool as offered to the model.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// The conversation so far, rebuilt from the run's event log.
///
/// Assistant content is kept exactly as the provider returned it. Some
/// providers require their own blocks (for example thinking blocks) to be
/// sent back unchanged, and the log stores them verbatim.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Transcript {
    pub system: String,
    pub task: String,
    pub turns: Vec<Turn>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Turn {
    /// Content blocks of the model's response, verbatim.
    pub assistant: Vec<Value>,
    /// Results of the tool calls that response asked for, in call order.
    pub results: Vec<ToolResultBlock>,
    /// Messages from the owner that arrived after this response; they go
    /// to the model with the results, as the next user turn.
    pub messages: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolResultBlock {
    pub call_id: String,
    pub content: String,
    pub is_error: bool,
}

pub struct ModelRequest<'a> {
    pub model: &'a str,
    pub transcript: &'a Transcript,
    pub tools: &'a [ToolSpec],
}

/// A complete model response. Content blocks use the Anthropic Messages
/// shapes (`text`, `tool_use`, `thinking`, …) as the canonical format;
/// adapters for other providers convert to and from it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelResponse {
    /// The model that actually produced the response (may differ from the
    /// requested one after a server-side fallback).
    pub model: String,
    pub content: Vec<Value>,
    pub stop_reason: StopReason,
    #[serde(default)]
    pub usage: Usage,
}

impl ModelResponse {
    pub fn tool_calls(&self) -> Vec<ToolCall> {
        self.content
            .iter()
            .filter(|b| b["type"] == "tool_use")
            .filter_map(|b| {
                Some(ToolCall {
                    id: b["id"].as_str()?.to_owned(),
                    name: b["name"].as_str()?.to_owned(),
                    input: b["input"].clone(),
                })
            })
            .collect()
    }

    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Refusal,
    #[serde(untagged)]
    Other(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("provider not configured: {0}")]
    Config(String),
    #[error("request failed: {0}")]
    Http(String),
    #[error("provider returned {status}: {body}")]
    Status { status: u16, body: String },
    #[error("unexpected response: {0}")]
    Decode(String),
}

/// A model behind a uniform, non-streaming interface. Streaming token
/// deltas to subscribers comes later; only complete responses are recorded.
pub trait ModelProvider: Send + Sync {
    fn complete<'a>(
        &'a self,
        request: ModelRequest<'a>,
    ) -> BoxFuture<'a, Result<ModelResponse, ProviderError>>;
}
