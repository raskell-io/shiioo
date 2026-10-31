//! The approval loop over the API: a harness hook asks, the rules answer or
//! the owner does, and everything lands in the run's log.

use serde_json::{Value, json};
use shiioo_api::{ApiOptions, ApiState, router};
use shiioo_core::SystemClock;
use shiioo_rules::RuleSet;
use shiioo_runtime::{
    AgentSpec, BoxFuture, LocalMachine, ModelProvider, ModelRequest, ModelResponse, ProviderError,
    ScriptedProvider, Toolbox, Worker,
};
use shiioo_store::{MemoryStore, Notifying};
use std::sync::Arc;
use std::time::Duration;

const TOKEN: &str = "test-token";

/// A model that never answers: the run stays open for as long as the test
/// needs, so we can poke at it through the API.
struct Stalled;

impl ModelProvider for Stalled {
    fn complete<'a>(
        &'a self,
        _: ModelRequest<'a>,
    ) -> BoxFuture<'a, Result<ModelResponse, ProviderError>> {
        Box::pin(async {
            tokio::time::sleep(Duration::from_secs(3600)).await;
            Err(ProviderError::Config("unreachable".into()))
        })
    }
}

async fn serve(provider: Arc<dyn ModelProvider>) -> String {
    let store = Arc::new(Notifying::new(MemoryStore::new(SystemClock)));
    let dir = std::env::temp_dir().join(format!("shiioo-perm-{}", uuid::Uuid::new_v4()));
    let machine = Arc::new(LocalMachine::in_memory(dir, Toolbox::builtin()));
    let factory_store = Arc::clone(&store);
    let state = ApiState::with_options(
        store,
        Arc::new(move || {
            Worker::new(
                Arc::clone(&factory_store),
                Arc::clone(&provider),
                Arc::clone(&machine),
                "perm-test",
            )
        }),
        TOKEN,
        AgentSpec {
            model: "scripted".into(),
            system: String::new(),
            tools: vec![],
            max_turns: 5,
        },
        ApiOptions {
            rules: RuleSet::defaults(),
            ..ApiOptions::default()
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

async fn post(http: &reqwest::Client, url: String, body: Value) -> (u16, Value) {
    let resp = http
        .post(url)
        .bearer_auth(TOKEN)
        .json(&body)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let status = resp.status().as_u16();
    (status, resp.json().await.unwrap_or(Value::Null))
}

async fn get(http: &reqwest::Client, url: String) -> Value {
    http.get(url)
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
}

#[tokio::test]
async fn rules_answer_at_once_and_the_owner_answers_the_rest() {
    let base = serve(Arc::new(Stalled)).await;
    let http = reqwest::Client::new();
    let (_, created) = post(
        &http,
        format!("{base}/v1/runs"),
        json!({ "task": "do things" }),
    )
    .await;
    let id = created["id"].as_str().unwrap_or_default().to_owned();

    // A read-only tool: the rules allow it, no human involved.
    let (status, outcome) = post(
        &http,
        format!("{base}/v1/runs/{id}/permissions"),
        json!({ "tool_name": "Read", "tool_input": { "file_path": "/x/README.md" }, "tool_use_id": "t1" }),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(outcome["decision"], "allow");
    assert!(outcome["approval_id"].is_null());

    // A dangerous command: denied by the rules.
    let (_, outcome) = post(
        &http,
        format!("{base}/v1/runs/{id}/permissions"),
        json!({ "tool_name": "Bash", "tool_input": { "command": "sudo rm -rf /" }, "tool_use_id": "t2" }),
    )
    .await;
    assert_eq!(outcome["decision"], "deny");

    // A push: the rules say ask. The request waits; the run shows it.
    let waiting = tokio::spawn({
        let http = http.clone();
        let base = base.clone();
        let id = id.clone();
        async move {
            post(
                &http,
                format!("{base}/v1/runs/{id}/permissions"),
                json!({ "tool_name": "Bash", "tool_input": { "command": "git push origin shiioo/abc" }, "tool_use_id": "t3" }),
            )
            .await
        }
    });
    let mut approval_id = String::new();
    for _ in 0..50 {
        let run = get(&http, format!("{base}/v1/runs/{id}")).await;
        if run["status"]["state"] == "waiting_approval" {
            approval_id = run["status"]["approval_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            assert_eq!(run["status"]["summary"], "Bash: git push origin shiioo/abc");
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        !approval_id.is_empty(),
        "the run never showed the open approval"
    );
    assert!(!waiting.is_finished(), "the hook must wait for the owner");

    let (status, decided) = post(
        &http,
        format!("{base}/v1/approvals/{approval_id}"),
        json!({ "decision": "allow", "note": "fine" }),
    )
    .await;
    assert_eq!(status, 200, "{decided}");
    let (status, outcome) = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap_or_else(|_| panic!("hook still waiting after the decision"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(status, 200);
    assert_eq!(outcome["decision"], "allow");
    assert_eq!(outcome["approval_id"], approval_id);

    // Everything is in the log, in order.
    let events = get(&http, format!("{base}/v1/runs/{id}/events")).await;
    let kinds: Vec<&str> = events
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| e["payload"]["type"].as_str())
        .collect();
    let governance: Vec<&str> = kinds
        .iter()
        .copied()
        .filter(|k| {
            matches!(
                *k,
                "policy_decision" | "approval_requested" | "approval_decided"
            )
        })
        .collect();
    assert_eq!(
        governance,
        [
            "policy_decision",
            "policy_decision",
            "policy_decision",
            "approval_requested",
            "approval_decided"
        ]
    );
    let run = get(&http, format!("{base}/v1/runs/{id}")).await;
    assert_eq!(run["status"]["state"], "open");

    // Deciding twice is a 404: nothing is open any more.
    let (status, _) = post(
        &http,
        format!("{base}/v1/approvals/{approval_id}"),
        json!({ "decision": "deny" }),
    )
    .await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn cancel_stops_the_run_at_its_next_step() {
    // A scripted model that asks for one tool call, then answers; the
    // cancel lands before the worker's first step.
    let provider = Arc::new(ScriptedProvider::new(vec![
        ScriptedProvider::tool_use("t1", "write_file", json!({ "path": "a", "content": "b" })),
        ScriptedProvider::answer("done"),
    ]));
    let base = serve(provider).await;
    let http = reqwest::Client::new();
    let (_, created) = post(
        &http,
        format!("{base}/v1/runs"),
        json!({ "task": "write a" }),
    )
    .await;
    let id = created["id"].as_str().unwrap_or_default().to_owned();

    // Race the worker; either the cancel wins and nothing ran, or the run
    // finished first and the cancel is refused with a conflict. Both are
    // correct; what must not happen is a half-cancelled run.
    let (status, _) = post(
        &http,
        format!("{base}/v1/runs/{id}/cancel"),
        json!({ "reason": "test" }),
    )
    .await;
    assert!(status == 202 || status == 409, "{status}");

    let mut final_state = String::new();
    for _ in 0..50 {
        let run = get(&http, format!("{base}/v1/runs/{id}")).await;
        let state = run["status"]["state"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if state == "cancelled" || state == "completed" {
            final_state = state;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        final_state == "cancelled" || final_state == "completed",
        "{final_state}"
    );
    if status == 202 {
        assert_eq!(final_state, "cancelled");
    }

    let runs = get(&http, format!("{base}/v1/runs")).await;
    assert_eq!(runs.as_array().map_or(0, Vec::len), 1);
    assert_eq!(runs[0]["task"], "write a");
}

#[tokio::test]
async fn edits_outside_the_runs_workspace_ask_the_owner() {
    let base = serve(Arc::new(Stalled)).await;
    let http = reqwest::Client::new();
    let (_, created) = post(
        &http,
        format!("{base}/v1/runs"),
        json!({ "task": "do things" }),
    )
    .await;
    let id = created["id"].as_str().unwrap_or_default().to_owned();
    let hook = |path: &str, call: &str| {
        json!({ "tool_name": "Write", "tool_input": { "file_path": path, "content": "" },
                "tool_use_id": call, "cwd": "/work/tree" })
    };

    // Inside the working directory: allowed by the rules.
    let (_, outcome) = post(
        &http,
        format!("{base}/v1/runs/{id}/permissions"),
        hook("src/retry.rs", "w1"),
    )
    .await;
    assert_eq!(outcome["decision"], "allow");

    // Somewhere else (the agent guessed a path): the owner is asked.
    let waiting = tokio::spawn({
        let (http, base, id) = (http.clone(), base.clone(), id.clone());
        let body = hook("/home/me/repo/.shiioo/worktrees/x/src/retry.rs", "w2");
        async move { post(&http, format!("{base}/v1/runs/{id}/permissions"), body).await }
    });
    let mut approval_id = String::new();
    for _ in 0..50 {
        let run = get(&http, format!("{base}/v1/runs/{id}")).await;
        if run["status"]["state"] == "waiting_approval" {
            approval_id = run["status"]["approval_id"].as_str().unwrap_or_default().to_owned();
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!approval_id.is_empty(), "an edit outside the workspace was not asked about");
    post(
        &http,
        format!("{base}/v1/approvals/{approval_id}"),
        json!({ "decision": "deny" }),
    )
    .await;
    let (_, outcome) = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap_or_else(|_| panic!("hook still waiting"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(outcome["decision"], "deny");
}
