//! Model providers for Shiioo's native loop beyond the built-in Anthropic
//! one: any server speaking OpenAI's Chat Completions API (Ollama, LM
//! Studio, `mlx_lm.server`, vLLM, OpenAI itself), and a router that picks a
//! provider from a model-name prefix such as `ollama/qwen3:32b`.
//!
//! The recorded format stays Anthropic-shaped (`text` and `tool_use`
//! content blocks, see `shiioo_runtime::model`); this crate converts to and
//! from the Chat Completions shapes at the edge.

use serde_json::{Value, json};
use shiioo_runtime::{
    BoxFuture, ModelProvider, ModelRequest, ModelResponse, ProviderError, StopReason, ToolSpec,
    Transcript, Usage,
};
use std::sync::Arc;
use std::time::Duration;

const MAX_ATTEMPTS: u32 = 4;

pub const OLLAMA_URL: &str = "http://127.0.0.1:11434/v1";
pub const LM_STUDIO_URL: &str = "http://127.0.0.1:1234/v1";
pub const OPENAI_URL: &str = "https://api.openai.com/v1";

/// A Chat Completions client. Non-streaming: the whole response is recorded
/// at once, like every provider in Shiioo today.
pub struct OpenAiCompatProvider {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
    timeout: Duration,
    /// Sent as `max_tokens` when set. Local servers mostly ignore it.
    pub max_tokens: Option<u32>,
}

impl OpenAiCompatProvider {
    /// `base_url` is the API root including `/v1`, for example
    /// `http://127.0.0.1:11434/v1`. A trailing slash is tolerated.
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        let timeout = Duration::from_secs(600);
        Self {
            http: client(timeout),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            api_key,
            timeout,
            max_tokens: None,
        }
    }

    /// Ollama on this machine (`OLLAMA_HOST` is not consulted; pass a URL
    /// to `new` for a remote one).
    pub fn ollama() -> Self {
        Self::new(OLLAMA_URL, None)
    }

    /// LM Studio's local server.
    pub fn lm_studio() -> Self {
        Self::new(LM_STUDIO_URL, None)
    }

    /// OpenAI's API, billed to the key.
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self::new(OPENAI_URL, Some(api_key.into()))
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self.http = client(timeout);
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    fn body(&self, request: &ModelRequest<'_>) -> Value {
        let mut body = json!({
            "model": request.model,
            "messages": messages(request.transcript),
        });
        if !request.tools.is_empty() {
            body["tools"] = Value::Array(request.tools.iter().map(tool).collect());
            body["tool_choice"] = json!("auto");
        }
        if let Some(max) = self.max_tokens {
            body["max_tokens"] = json!(max);
        }
        body
    }

    async fn send(&self, body: &Value, model: &str) -> Result<ModelResponse, ProviderError> {
        let mut req = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .json(body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
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
        let raw: Value = serde_json::from_str(&text)
            .map_err(|e| ProviderError::Decode(format!("{e}: {}", truncate(&text, 300))))?;
        response(&raw, model)
    }
}

impl ModelProvider for OpenAiCompatProvider {
    fn complete<'a>(
        &'a self,
        request: ModelRequest<'a>,
    ) -> BoxFuture<'a, Result<ModelResponse, ProviderError>> {
        Box::pin(async move {
            let body = self.body(&request);
            let mut attempt = 0;
            loop {
                attempt += 1;
                match self.send(&body, request.model).await {
                    Ok(resp) => return Ok(resp),
                    Err(e) if attempt < MAX_ATTEMPTS && is_transient(&e) => {}
                    Err(e) => return Err(e),
                }
                tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt))).await;
            }
        })
    }
}

fn client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .unwrap_or_default()
}

/// The Chat Completions `messages` array for a transcript. Assistant
/// turns are rebuilt from the recorded Anthropic-shaped blocks: text
/// blocks become `content`, `tool_use` blocks become `tool_calls` with the
/// input re-encoded as a JSON string; each result becomes a `tool` message.
fn messages(transcript: &Transcript) -> Vec<Value> {
    let mut out = Vec::new();
    if !transcript.system.is_empty() {
        out.push(json!({ "role": "system", "content": transcript.system }));
    }
    out.push(json!({ "role": "user", "content": transcript.task }));
    for turn in &transcript.turns {
        let text: Vec<&str> = turn
            .assistant
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect();
        let tool_calls: Vec<Value> = turn
            .assistant
            .iter()
            .filter(|b| b["type"] == "tool_use")
            .map(|b| {
                json!({
                    "id": b["id"],
                    "type": "function",
                    "function": {
                        "name": b["name"],
                        "arguments": b["input"].to_string(),
                    }
                })
            })
            .collect();
        let mut assistant = json!({ "role": "assistant" });
        assistant["content"] = if text.is_empty() {
            Value::Null
        } else {
            Value::from(text.join("\n"))
        };
        if !tool_calls.is_empty() {
            assistant["tool_calls"] = Value::Array(tool_calls);
        }
        out.push(assistant);
        for result in &turn.results {
            // Chat Completions has no error flag on tool messages; say it
            // in the text so the model knows the call failed.
            let content = if result.is_error {
                format!("Error: {}", result.content)
            } else {
                result.content.clone()
            };
            out.push(json!({
                "role": "tool",
                "tool_call_id": result.call_id,
                "content": content,
            }));
        }
    }
    out
}

fn tool(spec: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": spec.name,
            "description": spec.description,
            "parameters": spec.input_schema,
        }
    })
}

/// Maps a Chat Completions response onto the recorded format.
fn response(raw: &Value, requested_model: &str) -> Result<ModelResponse, ProviderError> {
    let choice = raw["choices"].get(0).ok_or_else(|| {
        ProviderError::Decode(format!("no choices in {}", truncate(&raw.to_string(), 300)))
    })?;
    let message = &choice["message"];
    let mut content = Vec::new();
    let text = content_text(&message["content"]);
    if !text.is_empty() {
        content.push(json!({ "type": "text", "text": text }));
    }
    for (i, call) in message["tool_calls"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        // Some local servers omit ids; the loop needs one per call.
        let id = call["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map_or_else(|| format!("call_{i}"), str::to_owned);
        let input = match &call["function"]["arguments"] {
            Value::String(s) => serde_json::from_str(s).unwrap_or_else(|_| json!({ "_raw": s })),
            Value::Null => json!({}),
            other => other.clone(),
        };
        content.push(json!({
            "type": "tool_use",
            "id": id,
            "name": call["function"]["name"].as_str().unwrap_or_default(),
            "input": input,
        }));
    }
    let stop_reason = match choice["finish_reason"].as_str() {
        Some("stop") | None => StopReason::EndTurn,
        Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
        Some("length") => StopReason::MaxTokens,
        Some("content_filter") => StopReason::Refusal,
        Some(other) => StopReason::Other(other.to_owned()),
    };
    Ok(ModelResponse {
        model: raw["model"]
            .as_str()
            .filter(|m| !m.is_empty())
            .unwrap_or(requested_model)
            .to_owned(),
        content,
        stop_reason,
        usage: Usage {
            input_tokens: raw["usage"]["prompt_tokens"].as_u64().unwrap_or(0),
            output_tokens: raw["usage"]["completion_tokens"].as_u64().unwrap_or(0),
        },
    })
}

/// `content` is a string, null, or an array of `{type: text, text}` parts.
fn content_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p["text"].as_str())
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn retryable(status: u16) -> bool {
    status == 408 || status == 409 || status == 429 || status >= 500
}

fn is_transient(e: &ProviderError) -> bool {
    match e {
        ProviderError::Http(_) => true,
        ProviderError::Status { status, .. } => retryable(*status),
        _ => false,
    }
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

/// Picks a provider from the model name's prefix and hands it the rest.
///
/// `ollama/qwen3:32b` goes to the provider registered for `ollama/` as
/// model `qwen3:32b`; a name without a known prefix goes to the default
/// provider unchanged. The longest matching prefix wins.
pub struct ProviderRouter {
    default: Arc<dyn ModelProvider>,
    routes: Vec<(String, Arc<dyn ModelProvider>)>,
}

impl ProviderRouter {
    pub fn new(default: Arc<dyn ModelProvider>) -> Self {
        Self {
            default,
            routes: Vec::new(),
        }
    }

    /// Registers `provider` for model names starting with `prefix` (a
    /// trailing `/` is added if missing).
    pub fn route(mut self, prefix: &str, provider: Arc<dyn ModelProvider>) -> Self {
        let prefix = if prefix.ends_with('/') {
            prefix.to_owned()
        } else {
            format!("{prefix}/")
        };
        self.routes.retain(|(p, _)| *p != prefix);
        self.routes.push((prefix, provider));
        self
    }

    pub fn prefixes(&self) -> Vec<&str> {
        self.routes.iter().map(|(p, _)| p.as_str()).collect()
    }

    /// The provider for `model` and the model name to send it.
    pub fn select<'m>(&self, model: &'m str) -> (&Arc<dyn ModelProvider>, &'m str) {
        self.routes
            .iter()
            .filter(|(prefix, _)| model.starts_with(prefix.as_str()))
            .max_by_key(|(prefix, _)| prefix.len())
            .map_or((&self.default, model), |(prefix, provider)| {
                (provider, &model[prefix.len()..])
            })
    }
}

impl ModelProvider for ProviderRouter {
    fn complete<'a>(
        &'a self,
        request: ModelRequest<'a>,
    ) -> BoxFuture<'a, Result<ModelResponse, ProviderError>> {
        let (provider, model) = self.select(request.model);
        provider.complete(ModelRequest {
            model,
            transcript: request.transcript,
            tools: request.tools,
        })
    }
}

/// The router an office uses: the given default (normally the Anthropic
/// provider, or the unconfigured one) plus `ollama/` (URL from
/// `SHIIOO_OLLAMA_URL`), `lmstudio/` (`SHIIOO_LMSTUDIO_URL`), and `openai/`
/// when `OPENAI_API_KEY` is set.
pub fn from_env(default: Arc<dyn ModelProvider>) -> ProviderRouter {
    let url = |var: &str, fallback: &str| {
        std::env::var(var)
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| fallback.to_owned())
    };
    let mut router = ProviderRouter::new(default)
        .route(
            "ollama/",
            Arc::new(OpenAiCompatProvider::new(
                url("SHIIOO_OLLAMA_URL", OLLAMA_URL),
                None,
            )),
        )
        .route(
            "lmstudio/",
            Arc::new(OpenAiCompatProvider::new(
                url("SHIIOO_LMSTUDIO_URL", LM_STUDIO_URL),
                None,
            )),
        );
    if let Some(key) = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
    {
        router = router.route("openai/", Arc::new(OpenAiCompatProvider::openai(key)));
    }
    router
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiioo_runtime::{ToolResultBlock, Turn};

    struct Named(&'static str);

    impl ModelProvider for Named {
        fn complete<'a>(
            &'a self,
            request: ModelRequest<'a>,
        ) -> BoxFuture<'a, Result<ModelResponse, ProviderError>> {
            let model = format!("{}:{}", self.0, request.model);
            Box::pin(async move {
                Ok(ModelResponse {
                    model,
                    content: vec![],
                    stop_reason: StopReason::EndTurn,
                    usage: Usage::default(),
                })
            })
        }
    }

    #[test]
    fn router_picks_the_longest_prefix_and_strips_it() {
        let router = ProviderRouter::new(Arc::new(Named("default")))
            .route("ollama", Arc::new(Named("ollama")))
            .route("ollama/big/", Arc::new(Named("ollama-big")));
        let (_, model) = router.select("ollama/qwen3:32b");
        assert_eq!(model, "qwen3:32b");
        let (_, model) = router.select("ollama/big/llama");
        assert_eq!(model, "llama");
        let (_, model) = router.select("claude-opus-5-5");
        assert_eq!(model, "claude-opus-5-5");
        assert_eq!(router.prefixes(), ["ollama/", "ollama/big/"]);
    }

    #[tokio::test]
    async fn router_dispatches() {
        let router = ProviderRouter::new(Arc::new(Named("default")))
            .route("ollama/", Arc::new(Named("ollama")));
        let transcript = Transcript::default();
        let resp = router
            .complete(ModelRequest {
                model: "ollama/qwen3",
                transcript: &transcript,
                tools: &[],
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resp.model, "ollama:qwen3");
        let resp = router
            .complete(ModelRequest {
                model: "gpt-x",
                transcript: &transcript,
                tools: &[],
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resp.model, "default:gpt-x");
    }

    #[test]
    fn messages_follow_the_chat_completions_shape() {
        let transcript = Transcript {
            system: "Be terse.".into(),
            task: "List files.".into(),
            turns: vec![Turn {
                assistant: vec![
                    json!({ "type": "text", "text": "Listing." }),
                    json!({ "type": "tool_use", "id": "call_1", "name": "shell", "input": { "command": "ls" } }),
                ],
                results: vec![ToolResultBlock {
                    call_id: "call_1".into(),
                    content: "a b".into(),
                    is_error: true,
                }],
            }],
        };
        let m = messages(&transcript);
        assert_eq!(m[0]["role"], "system");
        assert_eq!(m[1], json!({ "role": "user", "content": "List files." }));
        assert_eq!(m[2]["role"], "assistant");
        assert_eq!(m[2]["content"], "Listing.");
        assert_eq!(m[2]["tool_calls"][0]["function"]["name"], "shell");
        assert_eq!(
            m[2]["tool_calls"][0]["function"]["arguments"],
            r#"{"command":"ls"}"#
        );
        assert_eq!(m[3]["role"], "tool");
        assert_eq!(m[3]["tool_call_id"], "call_1");
        assert_eq!(m[3]["content"], "Error: a b");
    }

    #[test]
    fn response_handles_missing_ids_and_object_arguments() {
        let raw = json!({
            "model": "qwen3",
            "choices": [{
                "finish_reason": "tool_calls",
                "message": {
                    "content": [{ "type": "text", "text": "ok" }],
                    "tool_calls": [
                        { "function": { "name": "read_file", "arguments": { "path": "x" } } },
                        { "id": "", "function": { "name": "shell", "arguments": "not json" } }
                    ]
                }
            }],
            "usage": { "prompt_tokens": 3, "completion_tokens": 4 }
        });
        let resp = response(&raw, "ollama/qwen3").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resp.text(), "ok");
        let calls = resp.tool_calls();
        assert_eq!(calls[0].id, "call_0");
        assert_eq!(calls[0].input, json!({ "path": "x" }));
        assert_eq!(calls[1].id, "call_1");
        assert_eq!(calls[1].input, json!({ "_raw": "not json" }));
        assert_eq!(resp.stop_reason, StopReason::ToolUse);
        assert_eq!(resp.usage.output_tokens, 4);
    }
}
