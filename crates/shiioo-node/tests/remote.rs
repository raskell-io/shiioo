//! A node working for an office, end to end over HTTP: enrollment, work
//! polling, a run driven through the remote store, fencing across the wire.

use serde_json::{Value, json};
use shiioo_api::{ApiOptions, ApiState, Employee, EmployeeKind, router};
use shiioo_core::{EventStore, Payload, RunId, Seq, StoreError, SystemClock, WorkerId};
use shiioo_node::{NodeConfig, RemoteStore, enroll, run_node};
use shiioo_runtime::{AgentSpec, LocalMachine, ModelProvider, ScriptedProvider, Toolbox, Worker};
use shiioo_store::{MemoryStore, Notifying};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

const TOKEN: &str = "office-token";

struct Office {
    base: String,
    _dir: tempfile::TempDir,
}

/// An office with a scripted native employee and no repos; the office's
/// own workers would complete a run at once, so the test proves the node
/// did it by the worker id in the lease.
async fn serve() -> Office {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let store = Arc::new(Notifying::new(MemoryStore::new(SystemClock)));
    let machine = Arc::new(LocalMachine::in_memory(
        dir.path().join("ws"),
        Toolbox::builtin(),
    ));
    let provider: Arc<dyn ModelProvider> =
        Arc::new(ScriptedProvider::new(vec![ScriptedProvider::answer(
            "done by the office",
        )]));
    let factory_store = Arc::clone(&store);
    let state = ApiState::with_options(
        store,
        Arc::new(move || {
            Worker::new(
                Arc::clone(&factory_store),
                Arc::clone(&provider),
                Arc::clone(&machine),
                "office-worker",
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
            employees: vec![Employee {
                name: "scripted".into(),
                kind: EmployeeKind::Native {
                    model: "scripted".into(),
                },
                max_five_hour: 0.95,
                max_seven_day: 0.9,
            }],
            data_dir: Some(dir.path().join("data")),
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
    Office {
        base: format!("http://{addr}"),
        _dir: dir,
    }
}

async fn api(
    http: &reqwest::Client,
    method: reqwest::Method,
    url: String,
    token: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut req = http.request(method, url).bearer_auth(token);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await.unwrap_or_else(|e| panic!("{e}"));
    let status = resp.status().as_u16();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    (status, v)
}

#[tokio::test]
async fn a_node_enrolls_takes_its_runs_and_drives_them_over_http() {
    let office = serve().await;
    let http = reqwest::Client::new();

    // Nobody can enroll without a token; the owner makes one.
    let (status, _) = api(
        &http,
        reqwest::Method::POST,
        format!("{}/v1/machines/enroll", office.base),
        "",
        Some(json!({ "token": "nope", "name": "spark" })),
    )
    .await;
    assert_eq!(status, 400);
    let (status, created) = api(
        &http,
        reqwest::Method::POST,
        format!("{}/v1/machines/enrollments", office.base),
        TOKEN,
        Some(json!({ "labels": { "pool": "gpu" } })),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let enrollment = created["token"].as_str().unwrap_or_default().to_owned();

    let machine_token = enroll(
        &office.base,
        &enrollment,
        "spark",
        &BTreeMap::from([("gpu".to_owned(), "true".to_owned())]),
        &["harness.codex".to_owned()],
        "0.0.1-test",
    )
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        enroll(
            &office.base,
            &enrollment,
            "again",
            &BTreeMap::new(),
            &[],
            "x"
        )
        .await
        .is_err()
    );

    // The machine token works on the API; the office lists the machine.
    let (status, listed) = api(
        &http,
        reqwest::Method::GET,
        format!("{}/v1/machines", office.base),
        &machine_token,
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(listed["machines"][0]["name"], "spark");
    assert_eq!(listed["machines"][0]["labels"]["pool"], "gpu");
    assert_eq!(listed["machines"][0]["online"], true);

    // A run addressed to an unknown machine is refused; one for spark is
    // not driven by the office.
    let (status, _) = api(
        &http,
        reqwest::Method::POST,
        format!("{}/v1/runs", office.base),
        TOKEN,
        Some(json!({ "task": "x", "employee": "scripted", "machine": "mars" })),
    )
    .await;
    assert_eq!(status, 400);
    let (status, created) = api(
        &http,
        reqwest::Method::POST,
        format!("{}/v1/runs", office.base),
        TOKEN,
        Some(json!({ "task": "Say hello from the node.", "employee": "scripted", "machine": "gpu=true" })),
    )
    .await;
    assert_eq!(status, 201, "{created}");
    let run: RunId =
        serde_json::from_value(created["id"].clone()).unwrap_or_else(|e| panic!("{e}"));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (_, view) = api(
        &http,
        reqwest::Method::GET,
        format!("{}/v1/runs/{run}", office.base),
        TOKEN,
        None,
    )
    .await;
    assert_eq!(
        view["status"]["state"], "open",
        "the office must not drive it: {view}"
    );
    assert_eq!(view["machine"], "gpu=true");

    // The work poll names it (matching by label).
    let (status, work) = api(
        &http,
        reqwest::Method::GET,
        format!("{}/v1/machines/spark/work?wait=1", office.base),
        &machine_token,
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(work["runs"][0], json!(run));

    // The node drives it through the remote store.
    let store = Arc::new(RemoteStore::new(&office.base, &machine_token));
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let machine = Arc::new(LocalMachine::in_memory(
        dir.path().join("ws"),
        Toolbox::builtin(),
    ));
    let provider: Arc<dyn ModelProvider> =
        Arc::new(ScriptedProvider::new(vec![ScriptedProvider::answer(
            "hello from spark",
        )]));
    let worker_store = Arc::clone(&store);
    let workers: Arc<dyn Fn() -> Worker<RemoteStore> + Send + Sync> = Arc::new(move || {
        Worker::new(
            Arc::clone(&worker_store),
            Arc::clone(&provider),
            Arc::clone(&machine),
            "node-spark",
        )
    });
    let mut config = NodeConfig::new("spark");
    config.poll_wait_secs = 1;
    let node = tokio::spawn(run_node(config, Arc::clone(&store), workers));

    let mut view = Value::Null;
    for _ in 0..100 {
        view = api(
            &http,
            reqwest::Method::GET,
            format!("{}/v1/runs/{run}", office.base),
            TOKEN,
            None,
        )
        .await
        .1;
        if view["status"]["state"] == "completed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(view["status"]["state"], "completed", "{view}");
    assert_eq!(view["status"]["output"], "hello from spark");
    assert_eq!(view["worker"], "node-spark");
    node.abort();

    // The log as the node reads it is the office's log, chain intact.
    let log = store
        .read(run, Seq(0))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(shiioo_core::verify_chain(&log), Ok(()));
    assert!(
        matches!(&*log[1].payload, Payload::LeaseAcquired { worker, .. } if worker.0 == "node-spark")
    );

    // Nothing is left to do, and the heartbeat recorded the node.
    let (_, work) = api(
        &http,
        reqwest::Method::GET,
        format!("{}/v1/machines/spark/work?wait=0", office.base),
        &machine_token,
        None,
    )
    .await;
    assert_eq!(work["runs"].as_array().map(Vec::len), Some(0));
    let (_, listed) = api(
        &http,
        reqwest::Method::GET,
        format!("{}/v1/machines", office.base),
        TOKEN,
        None,
    )
    .await;
    assert_eq!(listed["machines"][0]["version"], "0.0.1");

    // Revoking the machine stops its token.
    let (status, _) = api(
        &http,
        reqwest::Method::DELETE,
        format!("{}/v1/machines/spark", office.base),
        TOKEN,
        None,
    )
    .await;
    assert_eq!(status, 204);
    let (status, _) = api(
        &http,
        reqwest::Method::GET,
        format!("{}/v1/machines", office.base),
        &machine_token,
        None,
    )
    .await;
    assert_eq!(status, 401);
}

#[tokio::test]
async fn fencing_errors_cross_the_wire() {
    let office = serve().await;
    let http = reqwest::Client::new();
    let (_, created) = api(
        &http,
        reqwest::Method::POST,
        format!("{}/v1/machines/enrollments", office.base),
        TOKEN,
        None,
    )
    .await;
    let machine_token = enroll(
        &office.base,
        created["token"].as_str().unwrap_or_default(),
        "n1",
        &BTreeMap::new(),
        &[],
        "t",
    )
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    let (_, created) = api(
        &http,
        reqwest::Method::POST,
        format!("{}/v1/runs", office.base),
        TOKEN,
        Some(json!({ "task": "fence me", "employee": "scripted", "machine": "n1" })),
    )
    .await;
    let run: RunId =
        serde_json::from_value(created["id"].clone()).unwrap_or_else(|e| panic!("{e}"));
    let store = RemoteStore::new(&office.base, &machine_token);

    let lease = store
        .acquire_lease(run, WorkerId("a".into()), 30_000)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    // Held: a second worker is refused with the holder.
    match store.acquire_lease(run, WorkerId("b".into()), 30_000).await {
        Err(StoreError::LeaseHeld { holder, .. }) => assert_eq!(holder.0, "a"),
        other => panic!("{other:?}"),
    }
    // A wrong expected_next is a SeqConflict with the real position.
    match store
        .append(
            &lease,
            Seq(99),
            vec![Payload::RunFailed { reason: "x".into() }],
        )
        .await
    {
        Err(StoreError::SeqConflict { expected, actual }) => {
            assert_eq!(expected, Seq(99));
            assert_eq!(actual, Seq(2));
        }
        other => panic!("{other:?}"),
    }
    // A stale epoch is fenced.
    let stale = shiioo_core::Lease {
        epoch: shiioo_core::Epoch(0),
        ..lease.clone()
    };
    match store
        .append(
            &stale,
            Seq(2),
            vec![Payload::RunFailed { reason: "x".into() }],
        )
        .await
    {
        Err(StoreError::Fenced { presented, current }) => {
            assert_eq!(presented, shiioo_core::Epoch(0));
            assert_eq!(current, lease.epoch);
        }
        other => panic!("{other:?}"),
    }
    // A good append lands, renews work, and a terminal event closes the run.
    let renewed = store
        .renew_lease(&lease, 30_000)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(renewed.epoch, lease.epoch);
    let events = store
        .append(
            &renewed,
            Seq(2),
            vec![Payload::RunFailed {
                reason: "fenced test".into(),
            }],
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(events.len(), 1);
    match store
        .append(
            &renewed,
            Seq(3),
            vec![Payload::RunFailed {
                reason: "again".into(),
            }],
        )
        .await
    {
        Err(StoreError::RunClosed(id)) => assert_eq!(id, run),
        other => panic!("{other:?}"),
    }
    let ids = store.list_runs().await.unwrap_or_else(|e| panic!("{e}"));
    assert!(ids.contains(&run));
}
