use crate::model::{
    BoxFuture, ModelProvider, ModelRequest, ModelResponse, ProviderError, StopReason, Usage,
};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU32, Ordering};

/// A deterministic provider for tests and simulation: the response for turn
/// `n` is the `n`-th scripted response, where `n` is the number of turns
/// already in the transcript. Asking again for the same turn (for example
/// after a crash before the response was recorded) returns the same answer.
pub struct ScriptedProvider {
    responses: Vec<ModelResponse>,
    calls: AtomicU32,
}

impl ScriptedProvider {
    pub fn new(responses: Vec<ModelResponse>) -> Self {
        Self {
            responses,
            calls: AtomicU32::new(0),
        }
    }

    /// How many times the model was called, including repeats.
    pub fn calls(&self) -> u32 {
        self.calls.load(Ordering::SeqCst)
    }

    /// A response asking for one tool call.
    pub fn tool_use(id: &str, name: &str, input: Value) -> ModelResponse {
        response(
            vec![json!({ "type": "tool_use", "id": id, "name": name, "input": input })],
            StopReason::ToolUse,
        )
    }

    /// A final answer.
    pub fn answer(text: &str) -> ModelResponse {
        response(
            vec![json!({ "type": "text", "text": text })],
            StopReason::EndTurn,
        )
    }
}

fn response(content: Vec<Value>, stop_reason: StopReason) -> ModelResponse {
    ModelResponse {
        model: "scripted".into(),
        content,
        stop_reason,
        usage: Usage::default(),
    }
}

impl ModelProvider for ScriptedProvider {
    fn complete<'a>(
        &'a self,
        request: ModelRequest<'a>,
    ) -> BoxFuture<'a, Result<ModelResponse, ProviderError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let turn = request.transcript.turns.len();
        let result = self.responses.get(turn).cloned().ok_or_else(|| {
            ProviderError::Decode(format!("script has no response for turn {turn}"))
        });
        Box::pin(async move { result })
    }
}
