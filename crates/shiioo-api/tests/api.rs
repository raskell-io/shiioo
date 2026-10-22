use serde_json::{Value, json};
use shiioo_api::{ApiState, router};
use shiioo_core::SystemClock;
use shiioo_runtime::{
    AgentSpec, BoxFuture, LocalMachine, ModelProvider, ModelRequest, ModelResponse, ProviderError,
    ScriptedProvider, Toolbox, Worker,
};
use shiioo_store::{MemoryStore, Notifying};
use std::sync::Arc;

const TOKEN: &str = "test-token";

/// A scripted model that holds its answer for the second turn until the
/// test releases it, so the run stays open while a client is streaming.
struct Gated {
    script: ScriptedProvider,
    release: Arc<tokio::sync::Notify>,
}

impl ModelProvider for Gated {
    fn complete<'a>(
        &'a self,
        request: ModelRequest<'a>,
    ) -> BoxFuture<'a, Result<ModelResponse, ProviderError>> {
        Box::pin(async move {
            if !request.transcript.turns.is_empty() {
                self.release.notified().await;
            }
            self.script.complete(request).await
        })
    }
}

/// Starts the API on an ephemeral port with a scripted model that writes a
/// file and then answers.
async fn serve() -> String {
    serve_with(None).await
}

async fn serve_with(release: Option<Arc<tokio::sync::Notify>>) -> String {
    let store = Arc::new(Notifying::new(MemoryStore::new(SystemClock)));
    let dir = std::env::temp_dir().join(format!("shiioo-api-{}", uuid_like()));
    let machine = Arc::new(LocalMachine::in_memory(dir, Toolbox::builtin()));
    let script = ScriptedProvider::new(vec![
        ScriptedProvider::tool_use(
            "t1",
            "write_file",
            json!({ "path": "hello.txt", "content": "hi" }),
        ),
        ScriptedProvider::answer("Wrote hello.txt."),
    ]);
    let provider: Arc<dyn ModelProvider> = match release {
        Some(release) => Arc::new(Gated { script, release }),
        None => Arc::new(script),
    };
    let factory_store = Arc::clone(&store);
    let state = ApiState::new(
        store,
        Arc::new(move || {
            Worker::new(
                Arc::clone(&factory_store),
                Arc::clone(&provider),
                Arc::clone(&machine),
                "api-test",
            )
        }),
        TOKEN,
        AgentSpec {
            model: "scripted".into(),
            system: String::new(),
            tools: vec!["write_file".into()],
            max_turns: 5,
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|e| panic!("bind: {e}"));
    let addr = listener.local_addr().unwrap_or_else(|e| panic!("{e}"));
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.ok();
    });
    format!("http://{addr}")
}

fn uuid_like() -> String {
    format!("{:?}", std::time::SystemTime::now())
        .chars()
        .filter(char::is_ascii_digit)
        .collect()
}

/// Parses a finished SSE body into (id, event JSON) pairs.
fn parse_sse(body: &str) -> Vec<(u64, Value)> {
    body.split("\n\n")
        .filter_map(|frame| {
            let mut id = None;
            let mut data = None;
            for line in frame.lines() {
                if let Some(v) = line.strip_prefix("id:") {
                    id = v.trim().parse().ok();
                } else if let Some(v) = line.strip_prefix("data:") {
                    data = serde_json::from_str(v.trim()).ok();
                }
            }
            Some((id?, data?))
        })
        .collect()
}

#[tokio::test]
async fn run_streams_to_completion_and_resumes_from_last_event_id() {
    let base = serve().await;
    let http = reqwest::Client::new();

    let created: Value = http
        .post(format!("{base}/v1/runs"))
        .bearer_auth(TOKEN)
        .header("idempotency-key", "k1")
        .json(&json!({ "task": "Say hi in a file." }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let id = created["id"].as_str().unwrap_or_default().to_owned();

    // The stream ends after the terminal event.
    let body = http
        .get(format!("{base}/v1/runs/{id}/events/stream"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .text()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let events = parse_sse(&body);
    let ids: Vec<u64> = events.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        ids,
        (0..ids.len() as u64).collect::<Vec<_>>(),
        "contiguous ids"
    );
    assert_eq!(
        events.last().map(|(_, e)| e["payload"]["type"].clone()),
        Some(json!("run_completed"))
    );

    // Reconnecting with Last-Event-ID replays exactly what came after it.
    let resumed = http
        .get(format!("{base}/v1/runs/{id}/events/stream"))
        .bearer_auth(TOKEN)
        .header("last-event-id", "2")
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .text()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let resumed_ids: Vec<u64> = parse_sse(&resumed).iter().map(|(id, _)| *id).collect();
    assert_eq!(resumed_ids, ids[3..].to_vec());

    let run: Value = http
        .get(format!("{base}/v1/runs/{id}"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(run["status"]["state"], "completed");
    assert_eq!(run["status"]["output"], "Wrote hello.txt.");

    // The same Idempotency-Key returns the same run instead of a new one.
    let again: Value = http
        .post(format!("{base}/v1/runs"))
        .bearer_auth(TOKEN)
        .header("idempotency-key", "k1")
        .json(&json!({ "task": "Say hi in a file." }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(again["id"], created["id"]);
}

#[tokio::test]
async fn requires_a_token_and_reports_problems() {
    let base = serve().await;
    let http = reqwest::Client::new();

    let health = http
        .get(format!("{base}/v1/health"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(health.status(), 200);

    let anonymous = http
        .post(format!("{base}/v1/runs"))
        .json(&json!({ "task": "x" }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(anonymous.status(), 401);
    assert_eq!(
        anonymous
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/problem+json")
    );

    let missing = http
        .get(format!(
            "{base}/v1/runs/00000000-0000-4000-8000-000000000000"
        ))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(missing.status(), 404);
}

/// A client streaming an open run is woken by new events right away (not
/// by the periodic re-read), and the stream ends with the terminal event.
#[tokio::test]
async fn stream_waits_for_and_delivers_live_events() {
    let release = Arc::new(tokio::sync::Notify::new());
    let base = serve_with(Some(Arc::clone(&release))).await;
    let http = reqwest::Client::new();

    let created: Value = http
        .post(format!("{base}/v1/runs"))
        .bearer_auth(TOKEN)
        .json(&json!({ "task": "Say hi in a file." }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let id = created["id"].as_str().unwrap_or_default().to_owned();

    let mut stream = http
        .get(format!("{base}/v1/runs/{id}/events/stream"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    // Read until the first turn's tool result has arrived: the run is now
    // waiting on the gated model.
    let mut body = String::new();
    while !body.contains("tool_result") {
        let chunk = tokio::time::timeout(std::time::Duration::from_secs(5), stream.chunk())
            .await
            .unwrap_or_else(|_| panic!("no event within 5s; got: {body}"))
            .unwrap_or_else(|e| panic!("{e}"));
        match chunk {
            Some(bytes) => body.push_str(&String::from_utf8_lossy(&bytes)),
            None => panic!("stream ended early: {body}"),
        }
    }
    assert!(!body.contains("run_completed"));

    // Release the model; the rest must arrive well before the 2s re-read.
    let released_at = std::time::Instant::now();
    release.notify_one();
    while let Some(bytes) = tokio::time::timeout(std::time::Duration::from_secs(5), stream.chunk())
        .await
        .unwrap_or_else(|_| panic!("stream stalled; got: {body}"))
        .unwrap_or_else(|e| panic!("{e}"))
    {
        body.push_str(&String::from_utf8_lossy(&bytes));
    }
    assert!(body.contains("run_completed"), "{body}");
    assert!(
        released_at.elapsed() < std::time::Duration::from_secs(1),
        "woken by notification, not by polling: {:?}",
        released_at.elapsed()
    );
    let ids: Vec<u64> = parse_sse(&body).iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, (0..ids.len() as u64).collect::<Vec<_>>());
}
