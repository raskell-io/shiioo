//! Taking over a run through the API, the way the desk does: a ticket from
//! `POST …/takeover`, the terminal over `GET …/terminal?ticket=…`, and
//! `POST …/hand-back`.

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use shiioo_api::{ApiOptions, ApiState, router};
use shiioo_core::{EventStore, Payload, RunId, Seq, SystemClock};
use shiioo_runtime::harness::transcript::Homes;
use shiioo_runtime::{
    AgentSpec, HarnessSpec, LocalMachine, RunSpec, Runtime, ScriptedProvider, Toolbox, Worker,
};
use shiioo_store::{MemoryStore, Notifying};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

const TOKEN: &str = "test-token";

type Store = Notifying<MemoryStore<SystemClock>>;

struct Office {
    base: String,
    store: Arc<Store>,
    machine: Arc<LocalMachine>,
    dir: tempfile::TempDir,
    http: reqwest::Client,
}

async fn office(detach: Duration) -> Office {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let store: Arc<Store> = Arc::new(Notifying::new(MemoryStore::new(SystemClock)));
    let machine = Arc::new(LocalMachine::in_memory(
        dir.path().join("workspace"),
        Toolbox::builtin(),
    ));
    std::fs::create_dir_all(dir.path().join("workspace")).unwrap_or_else(|e| panic!("{e}"));
    let provider = Arc::new(ScriptedProvider::new(vec![]));
    let (factory_store, factory_machine) = (Arc::clone(&store), Arc::clone(&machine));
    let state = ApiState::with_options(
        Arc::clone(&store),
        Arc::new(move || {
            Worker::new(
                Arc::clone(&factory_store),
                Arc::clone(&provider) as _,
                Arc::clone(&factory_machine),
                "takeover-test",
            )
        }),
        TOKEN,
        AgentSpec {
            model: "scripted".into(),
            system: String::new(),
            tools: vec![],
            max_turns: 10,
        },
        ApiOptions {
            harness_homes: Some(Homes {
                claude: dir.path().join("claude-home"),
                codex: dir.path().join("codex-home"),
            }),
            takeover_detach: detach,
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
        store,
        machine,
        dir,
        http: reqwest::Client::new(),
    }
}

impl Office {
    /// A harness run whose "Claude Code" echoes what is typed.
    async fn harness_run(&self) -> RunId {
        let script = self.dir.path().join("interactive.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nif [ -t 0 ]; then echo tty:yes; fi\n\
             while IFS= read -r line; do echo \"got:$line\"; done\n",
        )
        .unwrap_or_else(|e| panic!("{e}"));
        let mut harness = HarnessSpec::claude_code();
        harness.binary = Some(PathBuf::from("/bin/sh"));
        harness.binary_args = vec![script.to_string_lossy().into_owned()];
        Worker::new(
            Arc::clone(&self.store),
            Arc::new(ScriptedProvider::new(vec![])) as _,
            Arc::clone(&self.machine),
            "creator",
        )
        .create(&RunSpec {
            origin: None,
            park_on_answer: false,
            task: "Fix the flaky test.".into(),
            agent: AgentSpec {
                model: String::new(),
                system: String::new(),
                tools: vec![],
                max_turns: 10,
            },
            runtime: Runtime::Harness(harness),
            machine: None,
        })
        .await
        .unwrap_or_else(|e| panic!("{e}"))
    }

    async fn post(&self, path: &str, body: Value) -> reqwest::Response {
        self.http
            .post(format!("{}{path}", self.base))
            .bearer_auth(TOKEN)
            .json(&body)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    }

    async fn ticket(&self, run: RunId) -> String {
        let resp = self
            .post(&format!("/v1/runs/{run}/takeover"), json!({}))
            .await;
        assert_eq!(resp.status(), 201);
        let body: Value = resp.json().await.unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            body["terminal"],
            format!(
                "/v1/runs/{run}/terminal?ticket={}",
                body["ticket"].as_str().unwrap_or("")
            )
        );
        body["ticket"].as_str().unwrap_or_default().to_owned()
    }

    fn socket_url(&self, run: RunId, ticket: &str) -> String {
        format!(
            "{}/v1/runs/{run}/terminal?ticket={ticket}",
            self.base.replacen("http", "ws", 1)
        )
    }

    async fn log(&self, run: RunId) -> Vec<Payload> {
        self.store
            .read(run, Seq(0))
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .iter()
            .map(|e| (*e.payload).clone())
            .collect()
    }

    async fn state(&self, run: RunId) -> String {
        let v: Value = self
            .http
            .get(format!("{}/v1/runs/{run}", self.base))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .json()
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        v["status"]["state"].as_str().unwrap_or_default().to_owned()
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Reads frames until the terminal output contains `want`.
async fn read_until(ws: &mut Socket, seen: &mut String, want: &str) {
    let ok = tokio::time::timeout(Duration::from_secs(10), async {
        while !seen.contains(want) {
            match ws.next().await {
                Some(Ok(Message::Binary(b))) => seen.push_str(&String::from_utf8_lossy(&b)),
                Some(Ok(Message::Text(t))) => seen.push_str(&format!("[{t}]")),
                Some(Ok(_)) => {}
                _ => return,
            }
        }
    })
    .await;
    assert!(ok.is_ok() && seen.contains(want), "no {want:?} in {seen:?}");
}

#[tokio::test]
async fn take_over_type_join_and_hand_back() {
    let office = office(Duration::from_secs(600)).await;
    let run = office.harness_run().await;

    let ticket = office.ticket(run).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(office.socket_url(run, &ticket))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut seen = String::new();
    read_until(&mut ws, &mut seen, "tty:yes").await;
    assert_eq!(office.state(run).await, "taken_over");

    ws.send(Message::Text(
        json!({"type": "resize", "cols": 90, "rows": 20})
            .to_string()
            .into(),
    ))
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    ws.send(Message::Binary(b"hello\r".to_vec().into()))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    read_until(&mut ws, &mut seen, "got:hello").await;

    // A ticket opens one socket.
    assert!(
        tokio_tungstenite::connect_async(office.socket_url(run, &ticket))
            .await
            .is_err()
    );
    assert!(
        tokio_tungstenite::connect_async(office.socket_url(run, "made-up"))
            .await
            .is_err()
    );
    // Messages wait until the hand back.
    let resp = office
        .post(&format!("/v1/runs/{run}/messages"), json!({"text": "Hi?"}))
        .await;
    assert_eq!(resp.status(), 409);

    // A second viewer joins the same terminal and sees what came before.
    let again = office.ticket(run).await;
    let (mut second, _) = tokio_tungstenite::connect_async(office.socket_url(run, &again))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mut seen2 = String::new();
    read_until(&mut second, &mut seen2, "got:hello").await;

    let resp = office
        .post(&format!("/v1/runs/{run}/hand-back"), json!({}))
        .await;
    assert_eq!(resp.status(), 200);
    let view: Value = resp.json().await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(view["status"]["state"], "parked");
    read_until(&mut ws, &mut seen, r#""type":"ended""#).await;

    let log = office.log(run).await;
    assert!(
        log.iter()
            .any(|p| matches!(p, Payload::TakeoverRequested { .. }))
    );
    assert!(
        log.iter()
            .any(|p| matches!(p, Payload::TakeoverStarted { .. }))
    );
    assert!(
        log.iter()
            .any(|p| matches!(p, Payload::TakeoverEnded { reason, .. } if reason == "handed_back"))
    );
    // Nobody is at the keyboard any more.
    let resp = office
        .post(&format!("/v1/runs/{run}/hand-back"), json!({}))
        .await;
    assert_eq!(resp.status(), 409);
}

#[tokio::test]
async fn a_takeover_nobody_watches_ends_and_resume_wakes_the_run() {
    let office = office(Duration::from_millis(400)).await;
    let run = office.harness_run().await;

    // Never attached: it ends on its own.
    office.ticket(run).await;
    let mut parked = false;
    for _ in 0..100 {
        if office.state(run).await == "parked" {
            parked = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(parked, "the detached takeover did not end");
    assert!(
        office
            .log(run)
            .await
            .iter()
            .any(|p| matches!(p, Payload::TakeoverEnded { reason, .. } if reason == "detached"))
    );

    // Taken over again and handed back to carry on: the note is the
    // message that wakes it.
    let ticket = office.ticket(run).await;
    let (mut ws, _) = tokio_tungstenite::connect_async(office.socket_url(run, &ticket))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    read_until(&mut ws, &mut String::new(), "tty:yes").await;
    office
        .post(
            &format!("/v1/runs/{run}/hand-back"),
            json!({"resume": true, "note": "Push it when the tests pass."}),
        )
        .await;
    let log = office.log(run).await;
    let ended = log
        .iter()
        .rposition(|p| matches!(p, Payload::TakeoverEnded { .. }))
        .unwrap_or(usize::MAX);
    let note = log
        .iter()
        .rposition(|p| matches!(p, Payload::UserMessage { content, .. } if content == "Push it when the tests pass."))
        .unwrap_or(0);
    assert!(ended < note && note < log.len(), "{log:#?}");
}

#[tokio::test]
async fn runs_that_cannot_be_taken_over_say_why() {
    let office = office(Duration::from_secs(600)).await;
    let native = Worker::new(
        Arc::clone(&office.store),
        Arc::new(ScriptedProvider::new(vec![])) as _,
        Arc::clone(&office.machine),
        "creator",
    )
    .create(&RunSpec {
        origin: None,
        park_on_answer: true,
        task: "Talk to me.".into(),
        agent: AgentSpec {
            model: "scripted".into(),
            system: String::new(),
            tools: vec![],
            max_turns: 3,
        },
        runtime: Runtime::Native,
        machine: None,
    })
    .await
    .unwrap_or_else(|e| panic!("{e}"));
    let resp = office
        .post(&format!("/v1/runs/{native}/takeover"), json!({}))
        .await;
    assert_eq!(resp.status(), 409);
    let resp = office
        .post(&format!("/v1/runs/{}/takeover", RunId::new()), json!({}))
        .await;
    assert_eq!(resp.status(), 404);
    let anonymous = office
        .http
        .post(format!("{}/v1/runs/{native}/takeover", office.base))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(anonymous.status(), 401);
}
