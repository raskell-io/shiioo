//! The OpenAI-compatible provider against a fake Chat Completions server
//! that records requests and replays canned responses.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};
use shiioo_providers::{OpenAiCompatProvider, ProviderRouter};
use shiioo_runtime::{
    ModelProvider, ModelRequest, StopReason, ToolResultBlock, ToolSpec, Transcript, Turn,
};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Fake {
    requests: Mutex<Vec<Value>>,
    replies: Mutex<VecDeque<(StatusCode, Value)>>,
}

impl Fake {
    fn reply(&self, status: StatusCode, body: Value) {
        self.replies
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push_back((status, body));
    }

    fn requests(&self) -> Vec<Value> {
        self.requests
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }
}

async fn chat(State(fake): State<Arc<Fake>>, Json(body): Json<Value>) -> (StatusCode, Json<Value>) {
    fake.requests
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .push(body);
    let next = fake
        .replies
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .pop_front();
    match next {
        Some((status, body)) => (status, Json(body)),
        None => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "no canned reply left" })),
        ),
    }
}

async fn serve(fake: Arc<Fake>) -> String {
    let app = Router::new()
        .route("/v1/chat/completions", post(chat))
        .with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|e| panic!("bind: {e}"));
    let addr = listener.local_addr().unwrap_or_else(|e| panic!("{e}"));
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    format!("http://{addr}/v1")
}

fn tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: "shell".into(),
        description: "Run a command".into(),
        input_schema: json!({
            "type": "object",
            "properties": { "command": { "type": "string" } },
            "required": ["command"]
        }),
    }]
}

fn tool_call_reply() -> Value {
    json!({
        "id": "chatcmpl-1",
        "model": "qwen3:32b",
        "choices": [{
            "index": 0,
            "finish_reason": "tool_calls",
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "call_abc",
                    "type": "function",
                    "function": { "name": "shell", "arguments": "{\"command\":\"ls -la\"}" }
                }]
            }
        }],
        "usage": { "prompt_tokens": 42, "completion_tokens": 7 }
    })
}

fn text_reply(text: &str) -> Value {
    json!({
        "model": "qwen3:32b",
        "choices": [{
            "index": 0,
            "finish_reason": "stop",
            "message": { "role": "assistant", "content": text }
        }],
        "usage": { "prompt_tokens": 50, "completion_tokens": 3 }
    })
}

#[tokio::test]
async fn a_tool_call_turn_round_trips() {
    let fake = Arc::new(Fake::default());
    let base = serve(Arc::clone(&fake)).await;
    let provider = OpenAiCompatProvider::new(&base, None);

    // First turn: the model asks for a tool.
    fake.reply(StatusCode::OK, tool_call_reply());
    let transcript = Transcript {
        system: "Be terse.".into(),
        task: "List the files.".into(),
        turns: vec![],
    };
    let tools = tools();
    let resp = provider
        .complete(ModelRequest {
            model: "qwen3:32b",
            transcript: &transcript,
            tools: &tools,
        })
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    let req = &fake.requests()[0];
    assert_eq!(req["model"], "qwen3:32b");
    assert_eq!(
        req["messages"][0],
        json!({ "role": "system", "content": "Be terse." })
    );
    assert_eq!(
        req["messages"][1],
        json!({ "role": "user", "content": "List the files." })
    );
    assert_eq!(req["tools"][0]["type"], "function");
    assert_eq!(req["tools"][0]["function"]["name"], "shell");
    assert_eq!(
        req["tools"][0]["function"]["parameters"]["required"][0],
        "command"
    );
    assert_eq!(req["tool_choice"], "auto");

    assert_eq!(resp.model, "qwen3:32b");
    assert_eq!(resp.stop_reason, StopReason::ToolUse);
    let calls = resp.tool_calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "call_abc");
    assert_eq!(calls[0].name, "shell");
    assert_eq!(calls[0].input, json!({ "command": "ls -la" }));
    assert_eq!(
        resp.content[0]["type"], "tool_use",
        "recorded in the canonical shape"
    );
    assert_eq!(resp.usage.input_tokens, 42);
    assert_eq!(resp.usage.output_tokens, 7);

    // Second turn: the recorded tool_use and its result go back as
    // Chat Completions messages; the model answers.
    fake.reply(StatusCode::OK, text_reply("Two files."));
    let transcript = Transcript {
        system: "Be terse.".into(),
        task: "List the files.".into(),
        turns: vec![Turn {
            assistant: resp.content.clone(),
            results: vec![ToolResultBlock {
                call_id: "call_abc".into(),
                content: "a.rs\nb.rs".into(),
                is_error: false,
            }],
        }],
    };
    let resp = provider
        .complete(ModelRequest {
            model: "qwen3:32b",
            transcript: &transcript,
            tools: &tools,
        })
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    let req = &fake.requests()[1];
    let messages = req["messages"].as_array().cloned().unwrap_or_default();
    assert_eq!(messages.len(), 4, "{messages:?}");
    assert_eq!(messages[2]["role"], "assistant");
    assert!(messages[2]["content"].is_null());
    assert_eq!(messages[2]["tool_calls"][0]["id"], "call_abc");
    assert_eq!(messages[2]["tool_calls"][0]["type"], "function");
    assert_eq!(messages[2]["tool_calls"][0]["function"]["name"], "shell");
    assert_eq!(
        messages[2]["tool_calls"][0]["function"]["arguments"], "{\"command\":\"ls -la\"}",
        "arguments are a JSON string"
    );
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], "call_abc");
    assert_eq!(messages[3]["content"], "a.rs\nb.rs");

    assert_eq!(resp.stop_reason, StopReason::EndTurn);
    assert_eq!(resp.text(), "Two files.");
    assert!(resp.tool_calls().is_empty());
}

#[tokio::test]
async fn a_rate_limit_is_retried() {
    let fake = Arc::new(Fake::default());
    let base = serve(Arc::clone(&fake)).await;
    let provider = OpenAiCompatProvider::new(&base, Some("secret".into()));
    fake.reply(
        StatusCode::TOO_MANY_REQUESTS,
        json!({ "error": "slow down" }),
    );
    fake.reply(StatusCode::OK, text_reply("fine"));

    let transcript = Transcript {
        task: "hi".into(),
        ..Transcript::default()
    };
    let resp = tokio::time::timeout(
        Duration::from_secs(10),
        provider.complete(ModelRequest {
            model: "gpt-x",
            transcript: &transcript,
            tools: &[],
        }),
    )
    .await
    .unwrap_or_else(|_| panic!("retry took too long"))
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(resp.text(), "fine");
    let requests = fake.requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[0].get("tools").is_none(),
        "no tools, no tools field"
    );
    assert!(requests[0].get("tool_choice").is_none());
}

#[tokio::test]
async fn a_bad_request_is_not_retried() {
    let fake = Arc::new(Fake::default());
    let base = serve(Arc::clone(&fake)).await;
    let provider = OpenAiCompatProvider::new(&base, None);
    fake.reply(StatusCode::BAD_REQUEST, json!({ "error": "unknown model" }));
    let transcript = Transcript {
        task: "hi".into(),
        ..Transcript::default()
    };
    let err = provider
        .complete(ModelRequest {
            model: "nope",
            transcript: &transcript,
            tools: &[],
        })
        .await;
    assert!(
        matches!(
            err,
            Err(shiioo_runtime::ProviderError::Status { status: 400, .. })
        ),
        "{err:?}"
    );
    assert_eq!(fake.requests().len(), 1);
}

#[tokio::test]
async fn the_router_strips_the_prefix_before_sending() {
    let fake = Arc::new(Fake::default());
    let base = serve(Arc::clone(&fake)).await;
    fake.reply(StatusCode::OK, text_reply("hello"));
    let router = ProviderRouter::new(Arc::new(OpenAiCompatProvider::new(&base, None)))
        .route("ollama/", Arc::new(OpenAiCompatProvider::new(&base, None)));
    let transcript = Transcript {
        task: "hi".into(),
        ..Transcript::default()
    };
    router
        .complete(ModelRequest {
            model: "ollama/qwen3:32b",
            transcript: &transcript,
            tools: &[],
        })
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(fake.requests()[0]["model"], "qwen3:32b");
}
