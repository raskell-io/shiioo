//! HTTP client for LLM provider APIs.
//!
//! Currently supports the Anthropic Messages API. Additional providers
//! can be added by implementing new `call_*` functions.

use crate::types::{
    ContentBlockType, DeltaContent, LlmContentBlock, LlmError, LlmMessage, LlmMessagesRequest,
    LlmMessagesResponse, LlmUsage, StreamEvent,
};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use tokio::sync::mpsc;
use tokio::io::AsyncBufReadExt;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Call the Anthropic Messages API.
pub async fn call_anthropic(
    api_key: &str,
    request: &LlmMessagesRequest,
) -> Result<LlmMessagesResponse, LlmError> {
    let model = request
        .model
        .as_deref()
        .unwrap_or("claude-sonnet-4-20250514");

    let api_request = AnthropicRequest {
        model: model.to_string(),
        max_tokens: request.max_tokens,
        temperature: request.temperature,
        system: request.system.clone(),
        messages: request
            .messages
            .iter()
            .map(|m| AnthropicMessage::from_llm_message(m))
            .collect(),
        tools: if request.tools.is_empty() {
            None
        } else {
            Some(
                request
                    .tools
                    .iter()
                    .map(|t| AnthropicTool {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        input_schema: t.input_schema.clone(),
                    })
                    .collect(),
            )
        },
        stream: false,
    };

    let client = reqwest::Client::new();
    let response = client
        .post(ANTHROPIC_API_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(&api_request)
        .send()
        .await
        .map_err(|e| LlmError::Other {
            message: format!("HTTP request failed: {e}"),
        })?;

    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(match status.as_u16() {
            401 => LlmError::AuthenticationFailed,
            429 => {
                // Try to parse retry-after from error body
                let retry_after = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v["error"]["retry_after"].as_u64());
                LlmError::RateLimited { retry_after }
            }
            400 => LlmError::InvalidRequest {
                message: parse_error_message(&body),
            },
            529 | 503 => LlmError::ServiceUnavailable,
            408 => LlmError::TimeoutExceeded,
            _ => LlmError::Other {
                message: format!("API error {status}: {body}"),
            },
        });
    }

    let api_response: AnthropicResponse =
        response.json().await.map_err(|e| LlmError::Other {
            message: format!("Failed to parse response: {e}"),
        })?;

    Ok(LlmMessagesResponse {
        id: api_response.id,
        content: api_response
            .content
            .into_iter()
            .map(|b| b.into_llm_content_block())
            .collect(),
        stop_reason: api_response.stop_reason,
        usage: LlmUsage {
            input_tokens: api_response.usage.input_tokens,
            output_tokens: api_response.usage.output_tokens,
        },
        model: api_response.model,
    })
}

/// Call the Anthropic Messages API with streaming.
///
/// Returns a stream of `StreamEvent`s. The stream will emit `ContentBlockDelta`
/// events as tokens arrive, and a final `MessageStop` event when complete.
pub async fn call_anthropic_streaming(
    api_key: &str,
    request: &LlmMessagesRequest,
) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, LlmError>> + Send>>, LlmError> {
    let model = request
        .model
        .as_deref()
        .unwrap_or("claude-sonnet-4-20250514");

    let api_request = AnthropicRequest {
        model: model.to_string(),
        max_tokens: request.max_tokens,
        temperature: request.temperature,
        system: request.system.clone(),
        messages: request
            .messages
            .iter()
            .map(|m| AnthropicMessage::from_llm_message(m))
            .collect(),
        tools: if request.tools.is_empty() {
            None
        } else {
            Some(
                request
                    .tools
                    .iter()
                    .map(|t| AnthropicTool {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        input_schema: t.input_schema.clone(),
                    })
                    .collect(),
            )
        },
        stream: true,
    };

    let client = reqwest::Client::new();
    let response = client
        .post(ANTHROPIC_API_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .header("content-type", "application/json")
        .json(&api_request)
        .send()
        .await
        .map_err(|e| LlmError::Other {
            message: format!("HTTP request failed: {e}"),
        })?;

    let status = response.status();

    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(match status.as_u16() {
            401 => LlmError::AuthenticationFailed,
            429 => {
                let retry_after = serde_json::from_str::<serde_json::Value>(&body)
                    .ok()
                    .and_then(|v| v["error"]["retry_after"].as_u64());
                LlmError::RateLimited { retry_after }
            }
            400 => LlmError::InvalidRequest {
                message: parse_error_message(&body),
            },
            529 | 503 => LlmError::ServiceUnavailable,
            408 => LlmError::TimeoutExceeded,
            _ => LlmError::Other {
                message: format!("API error {status}: {body}"),
            },
        });
    }

    // Convert the response byte stream into SSE events via a channel
    let (tx, rx) = mpsc::channel::<Result<StreamEvent, LlmError>>(64);

    let byte_stream = response.bytes_stream();
    tokio::spawn(async move {
        use tokio_stream::StreamExt;
        use tokio_util::io::StreamReader;

        let reader = StreamReader::new(
            byte_stream.map(|r| r.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))),
        );
        let mut lines = reader.lines();

        let mut current_event_type = String::new();

        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    if line.starts_with("event: ") {
                        current_event_type = line[7..].to_string();
                    } else if line.starts_with("data: ") {
                        let data = &line[6..];
                        if let Some(event) =
                            parse_sse_event(&current_event_type, data)
                        {
                            let is_stop = matches!(event, StreamEvent::MessageStop);
                            if tx.send(Ok(event)).await.is_err() {
                                break;
                            }
                            if is_stop {
                                break;
                            }
                        }
                        current_event_type.clear();
                    }
                    // Empty lines (SSE separators) and other lines are ignored
                }
                Ok(None) => break, // Stream ended
                Err(e) => {
                    let _ = tx
                        .send(Err(LlmError::Other {
                            message: format!("Stream read error: {e}"),
                        }))
                        .await;
                    break;
                }
            }
        }
    });

    Ok(Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx)))
}

/// Parse a single SSE data payload into a StreamEvent.
fn parse_sse_event(event_type: &str, data: &str) -> Option<StreamEvent> {
    let json: serde_json::Value = serde_json::from_str(data).ok()?;

    match event_type {
        "message_start" => {
            let msg = &json["message"];
            Some(StreamEvent::MessageStart {
                id: msg["id"].as_str()?.to_string(),
                model: msg["model"].as_str()?.to_string(),
            })
        }
        "content_block_start" => {
            let index = json["index"].as_u64()? as u32;
            let cb = &json["content_block"];
            let block_type = match cb["type"].as_str()? {
                "tool_use" => ContentBlockType::ToolUse {
                    id: cb["id"].as_str()?.to_string(),
                    name: cb["name"].as_str()?.to_string(),
                },
                _ => ContentBlockType::Text,
            };
            Some(StreamEvent::ContentBlockStart { index, block_type })
        }
        "content_block_delta" => {
            let index = json["index"].as_u64()? as u32;
            let delta_type = json["delta"]["type"].as_str().unwrap_or("text_delta");
            let delta = match delta_type {
                "input_json_delta" => {
                    let partial = json["delta"]["partial_json"]
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    DeltaContent::InputJson(partial)
                }
                _ => {
                    let text = json["delta"]["text"].as_str().unwrap_or("").to_string();
                    DeltaContent::Text(text)
                }
            };
            Some(StreamEvent::ContentBlockDelta { index, delta })
        }
        "content_block_stop" => {
            let index = json["index"].as_u64()? as u32;
            Some(StreamEvent::ContentBlockStop { index })
        }
        "message_delta" => {
            let stop_reason = json["delta"]["stop_reason"]
                .as_str()
                .map(|s| s.to_string());
            // Usage info comes in the message_delta event
            let usage_input = json["usage"]["input_tokens"].as_u64().unwrap_or(0) as u32;
            let usage_output = json["usage"]["output_tokens"].as_u64().unwrap_or(0) as u32;
            // Emit usage first if present, then the delta
            if usage_input > 0 || usage_output > 0 {
                // We can only return one event — caller needs to handle usage from message_delta
            }
            Some(StreamEvent::MessageDelta { stop_reason })
        }
        "message_stop" => Some(StreamEvent::MessageStop),
        "ping" => None, // Heartbeat, ignore
        _ => None,      // Unknown event type, skip
    }
}

fn parse_error_message(body: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
        .unwrap_or_else(|| body.to_string())
}

// --- Anthropic API wire types ---

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: serde_json::Value,
}

impl AnthropicMessage {
    fn from_llm_message(msg: &LlmMessage) -> Self {
        let role = match msg.role {
            crate::types::LlmRole::User => "user",
            crate::types::LlmRole::Assistant => "assistant",
        };

        // If single text block, use string shorthand; otherwise use array
        let content = if msg.content.len() == 1 {
            if let LlmContentBlock::Text { text } = &msg.content[0] {
                serde_json::Value::String(text.clone())
            } else {
                serde_json::to_value(&msg.content).unwrap_or_default()
            }
        } else {
            serde_json::to_value(&msg.content).unwrap_or_default()
        };

        Self {
            role: role.to_string(),
            content,
        }
    }
}

#[derive(Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    id: String,
    content: Vec<AnthropicContentBlock>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
    model: String,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum AnthropicContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
}

impl AnthropicContentBlock {
    fn into_llm_content_block(self) -> LlmContentBlock {
        match self {
            AnthropicContentBlock::Text { text } => LlmContentBlock::Text { text },
            AnthropicContentBlock::ToolUse { id, name, input } => {
                LlmContentBlock::ToolUse { id, name, input }
            }
        }
    }
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::LlmRole;

    #[test]
    fn test_anthropic_message_single_text() {
        let msg = LlmMessage::text(LlmRole::User, "Hello");
        let api_msg = AnthropicMessage::from_llm_message(&msg);
        assert_eq!(api_msg.role, "user");
        assert_eq!(api_msg.content, serde_json::Value::String("Hello".into()));
    }

    #[test]
    fn test_anthropic_message_multi_block() {
        let msg = LlmMessage {
            role: LlmRole::User,
            content: vec![
                LlmContentBlock::Text {
                    text: "Check this".into(),
                },
                LlmContentBlock::ToolResult {
                    tool_use_id: "t1".into(),
                    content: "result".into(),
                    is_error: None,
                },
            ],
        };
        let api_msg = AnthropicMessage::from_llm_message(&msg);
        assert_eq!(api_msg.role, "user");
        assert!(api_msg.content.is_array());
    }

    #[test]
    fn test_llm_message_text_content() {
        let msg = LlmMessage {
            role: LlmRole::Assistant,
            content: vec![
                LlmContentBlock::Text {
                    text: "Hello ".into(),
                },
                LlmContentBlock::Text {
                    text: "world".into(),
                },
            ],
        };
        assert_eq!(msg.text_content(), "Hello world");
    }

    #[test]
    fn test_parse_error_message_json() {
        let body = r#"{"error":{"type":"invalid_request","message":"max_tokens too large"}}"#;
        assert_eq!(parse_error_message(body), "max_tokens too large");
    }

    #[test]
    fn test_parse_error_message_plain() {
        let body = "Something went wrong";
        assert_eq!(parse_error_message(body), "Something went wrong");
    }

    #[test]
    fn test_anthropic_request_serialization() {
        let request = AnthropicRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 1024,
            temperature: Some(0.7),
            system: Some("You are helpful.".to_string()),
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: serde_json::Value::String("Hello".into()),
            }],
            tools: None,
            stream: false,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["model"], "claude-sonnet-4-20250514");
        assert_eq!(json["max_tokens"], 1024);
        assert!(json.get("tools").is_none()); // skipped when None
        assert!(json.get("stream").is_none()); // skipped when false
    }

    #[test]
    fn test_anthropic_request_stream_field() {
        let request = AnthropicRequest {
            model: "claude-sonnet-4-20250514".to_string(),
            max_tokens: 1024,
            temperature: None,
            system: None,
            messages: vec![AnthropicMessage {
                role: "user".to_string(),
                content: serde_json::Value::String("Hello".into()),
            }],
            tools: None,
            stream: true,
        };
        let json = serde_json::to_value(&request).unwrap();
        assert_eq!(json["stream"], true);
    }

    #[test]
    fn test_anthropic_response_deserialization() {
        let json = r#"{
            "id": "msg_123",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello!"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5},
            "model": "claude-sonnet-4-20250514"
        }"#;
        let resp: AnthropicResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.id, "msg_123");
        assert_eq!(resp.content.len(), 1);
        assert_eq!(resp.usage.input_tokens, 10);
        assert_eq!(resp.usage.output_tokens, 5);
    }

    #[test]
    fn test_parse_sse_message_start() {
        let data = r#"{"type":"message_start","message":{"id":"msg_abc","type":"message","role":"assistant","content":[],"model":"claude-sonnet-4-20250514","stop_reason":null,"usage":{"input_tokens":25,"output_tokens":1}}}"#;
        let event = parse_sse_event("message_start", data).unwrap();
        match event {
            StreamEvent::MessageStart { id, model } => {
                assert_eq!(id, "msg_abc");
                assert_eq!(model, "claude-sonnet-4-20250514");
            }
            _ => panic!("Expected MessageStart"),
        }
    }

    #[test]
    fn test_parse_sse_content_block_delta() {
        let data = r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}"#;
        let event = parse_sse_event("content_block_delta", data).unwrap();
        match event {
            StreamEvent::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 0);
                match delta {
                    DeltaContent::Text(text) => assert_eq!(text, "Hello"),
                    _ => panic!("Expected Text delta"),
                }
            }
            _ => panic!("Expected ContentBlockDelta"),
        }
    }

    #[test]
    fn test_parse_sse_content_block_start() {
        let data = r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#;
        let event = parse_sse_event("content_block_start", data).unwrap();
        match event {
            StreamEvent::ContentBlockStart { index, block_type } => {
                assert_eq!(index, 0);
                assert!(matches!(block_type, ContentBlockType::Text));
            }
            _ => panic!("Expected ContentBlockStart"),
        }
    }

    #[test]
    fn test_parse_sse_tool_use_content_block_start() {
        let data = r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_abc","name":"list_employees"}}"#;
        let event = parse_sse_event("content_block_start", data).unwrap();
        match event {
            StreamEvent::ContentBlockStart { index, block_type } => {
                assert_eq!(index, 1);
                match block_type {
                    ContentBlockType::ToolUse { id, name } => {
                        assert_eq!(id, "toolu_abc");
                        assert_eq!(name, "list_employees");
                    }
                    _ => panic!("Expected ToolUse block type"),
                }
            }
            _ => panic!("Expected ContentBlockStart"),
        }
    }

    #[test]
    fn test_parse_sse_input_json_delta() {
        let data = r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"team\":"}}"#;
        let event = parse_sse_event("content_block_delta", data).unwrap();
        match event {
            StreamEvent::ContentBlockDelta { index, delta } => {
                assert_eq!(index, 1);
                match delta {
                    DeltaContent::InputJson(json) => assert_eq!(json, "{\"team\":"),
                    _ => panic!("Expected InputJson delta"),
                }
            }
            _ => panic!("Expected ContentBlockDelta"),
        }
    }

    #[test]
    fn test_parse_sse_message_delta() {
        let data = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":15}}"#;
        let event = parse_sse_event("message_delta", data).unwrap();
        match event {
            StreamEvent::MessageDelta { stop_reason } => {
                assert_eq!(stop_reason.as_deref(), Some("end_turn"));
            }
            _ => panic!("Expected MessageDelta"),
        }
    }

    #[test]
    fn test_parse_sse_message_stop() {
        let data = r#"{"type":"message_stop"}"#;
        let event = parse_sse_event("message_stop", data).unwrap();
        assert!(matches!(event, StreamEvent::MessageStop));
    }

    #[test]
    fn test_parse_sse_ping_ignored() {
        let data = r#"{"type":"ping"}"#;
        assert!(parse_sse_event("ping", data).is_none());
    }

    #[test]
    fn test_anthropic_tool_use_response() {
        let json = r#"{
            "id": "msg_456",
            "type": "message",
            "role": "assistant",
            "content": [
                {"type": "text", "text": "Let me check."},
                {"type": "tool_use", "id": "tu_1", "name": "list_employees", "input": {}}
            ],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 50, "output_tokens": 30},
            "model": "claude-sonnet-4-20250514"
        }"#;
        let resp: AnthropicResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.content.len(), 2);
        assert_eq!(resp.stop_reason.as_deref(), Some("tool_use"));
    }
}
