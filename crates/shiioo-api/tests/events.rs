//! The office-wide stream, `GET /v1/events/stream`.

use serde_json::{Value, json};
use shiioo_api::{ApiState, router};
use shiioo_core::SystemClock;
use shiioo_runtime::{AgentSpec, LocalMachine, ScriptedProvider, Toolbox, Worker};
use shiioo_store::{MemoryStore, Notifying};
use std::sync::Arc;
use std::time::Duration;

const TOKEN: &str = "test-token";

async fn serve() -> String {
    let store = Arc::new(Notifying::new(MemoryStore::new(SystemClock)));
    let dir = std::env::temp_dir().join(format!("shiioo-events-{}", uuid::Uuid::new_v4()));
    let machine = Arc::new(LocalMachine::in_memory(dir, Toolbox::builtin()));
    let provider = Arc::new(ScriptedProvider::new(vec![
        ScriptedProvider::answer("one"),
        ScriptedProvider::answer("two"),
    ]));
    let factory_store = Arc::clone(&store);
    let state = ApiState::new(
        store,
        Arc::new(move || {
            Worker::new(
                Arc::clone(&factory_store),
                Arc::clone(&provider) as _,
                Arc::clone(&machine),
                "events-test",
            )
        }),
        TOKEN,
        AgentSpec {
            model: "scripted".into(),
            system: String::new(),
            tools: vec![],
            max_turns: 3,
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

/// One SSE frame: its `event:`, `id:` and parsed `data:`.
#[derive(Debug)]
struct Frame {
    event: String,
    id: Option<u64>,
    data: Value,
}

/// Reads frames from a live response until `done` says stop.
struct Reader {
    resp: reqwest::Response,
    buf: String,
}

impl Reader {
    async fn open(http: &reqwest::Client, base: &str, last_id: Option<u64>) -> Self {
        let mut req = http.get(format!("{base}/v1/events/stream")).bearer_auth(TOKEN);
        if let Some(id) = last_id {
            req = req.header("last-event-id", id.to_string());
        }
        let resp = req.send().await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resp.status(), 200);
        Self {
            resp,
            buf: String::new(),
        }
    }

    async fn until(&mut self, done: impl Fn(&Frame) -> bool) -> Vec<Frame> {
        let mut frames = Vec::new();
        loop {
            while let Some(cut) = self.buf.find("\n\n") {
                let text: String = self.buf.drain(..cut + 2).collect();
                let mut frame = Frame {
                    event: String::new(),
                    id: None,
                    data: Value::Null,
                };
                for line in text.lines() {
                    if let Some(v) = line.strip_prefix("event:") {
                        frame.event = v.trim().to_owned();
                    } else if let Some(v) = line.strip_prefix("id:") {
                        frame.id = v.trim().parse().ok();
                    } else if let Some(v) = line.strip_prefix("data:") {
                        frame.data = serde_json::from_str(v.trim()).unwrap_or(Value::Null);
                    }
                }
                if frame.event.is_empty() {
                    continue; // keep-alive comment
                }
                let stop = done(&frame);
                frames.push(frame);
                if stop {
                    return frames;
                }
            }
            let chunk = tokio::time::timeout(Duration::from_secs(5), self.resp.chunk())
                .await
                .unwrap_or_else(|_| panic!("no frame within 5s; got {frames:?}"))
                .unwrap_or_else(|e| panic!("{e}"))
                .unwrap_or_else(|| panic!("stream ended; got {frames:?}"));
            self.buf
                .push_str(&String::from_utf8_lossy(&chunk).replace("\r\n", "\n"));
        }
    }
}

async fn start(http: &reqwest::Client, base: &str, task: &str) -> String {
    let created: Value = http
        .post(format!("{base}/v1/runs"))
        .bearer_auth(TOKEN)
        .json(&json!({ "task": task }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    created["id"].as_str().unwrap_or_default().to_owned()
}

async fn wait_finished(http: &reqwest::Client, base: &str, id: &str) {
    for _ in 0..100 {
        let run: Value = http
            .get(format!("{base}/v1/runs/{id}"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .json()
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        if run["status"]["state"] == "completed" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("run {id} did not finish");
}

fn is_completed(frame: &Frame, id: &str) -> bool {
    frame.event == "run" && frame.data["id"] == id && frame.data["status"]["state"] == "completed"
}

#[tokio::test]
async fn snapshot_then_live_changes_then_resume_from_last_id() {
    let base = serve().await;
    let http = reqwest::Client::new();

    let first = start(&http, &base, "first").await;
    wait_finished(&http, &base, &first).await;
    // Let the next run's events land in a later millisecond.
    tokio::time::sleep(Duration::from_millis(5)).await;

    // The first pass covers every run, then says it is done.
    let mut stream = Reader::open(&http, &base, None).await;
    let snapshot = stream.until(|f| f.event == "ready").await;
    assert!(
        snapshot.iter().any(|f| is_completed(f, &first)),
        "{snapshot:?}"
    );
    assert_eq!(snapshot.iter().filter(|f| f.event == "run").count(), 1);

    // A new run arrives live, without polling, and finishes on the stream.
    let second = start(&http, &base, "second").await;
    let live = stream.until(|f| is_completed(f, &second)).await;
    assert!(
        live.iter()
            .all(|f| f.event == "run" && f.data["id"] == second)
    );
    let ids: Vec<u64> = live.iter().filter_map(|f| f.id).collect();
    assert!(
        ids.windows(2).all(|w| w[0] <= w[1]),
        "ids never go back: {ids:?}"
    );
    let last = ids.last().copied();

    // Reconnecting with the last id skips the runs that did not change
    // since, and repeats at most the latest view of the rest.
    let mut resumed = Reader::open(&http, &base, last).await;
    let again = resumed.until(|f| f.event == "ready").await;
    let runs: Vec<&Value> = again
        .iter()
        .filter(|f| f.event == "run")
        .map(|f| &f.data["id"])
        .collect();
    assert_eq!(runs, vec![&json!(second)], "{again:?}");
}

#[tokio::test]
async fn needs_the_token_and_a_numeric_last_id() {
    let base = serve().await;
    let http = reqwest::Client::new();
    let anonymous = http
        .get(format!("{base}/v1/events/stream"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(anonymous.status(), 401);
    let bad = http
        .get(format!("{base}/v1/events/stream"))
        .bearer_auth(TOKEN)
        .header("last-event-id", "yesterday")
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(bad.status(), 400);
}
