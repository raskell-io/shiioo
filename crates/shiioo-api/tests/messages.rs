//! `POST /v1/runs/{id}/messages`: the owner writes to a run.

use serde_json::{Value, json};
use shiioo_api::{ApiState, router};
use shiioo_core::{EventStore, Payload, RunId, Seq, SystemClock};
use shiioo_runtime::{
    AgentSpec, HarnessSpec, LocalMachine, RunSpec, Runtime, ScriptedProvider, Toolbox, Worker,
};
use shiioo_store::{MemoryStore, Notifying};
use std::sync::Arc;
use std::time::Duration;

const TOKEN: &str = "test-token";

type Store = Notifying<MemoryStore<SystemClock>>;

struct Office {
    base: String,
    store: Arc<Store>,
    machine: Arc<LocalMachine>,
    http: reqwest::Client,
}

async fn office() -> Office {
    let store: Arc<Store> = Arc::new(Notifying::new(MemoryStore::new(SystemClock)));
    let dir = std::env::temp_dir().join(format!("shiioo-messages-{}", uuid::Uuid::new_v4()));
    let machine = Arc::new(LocalMachine::in_memory(dir, Toolbox::builtin()));
    let provider = Arc::new(ScriptedProvider::new(vec![
        ScriptedProvider::answer("Hello."),
        ScriptedProvider::answer("Sure, without a new dependency."),
        ScriptedProvider::answer("Anything else?"),
    ]));
    let (factory_store, factory_machine) = (Arc::clone(&store), Arc::clone(&machine));
    let state = ApiState::new(
        Arc::clone(&store),
        Arc::new(move || {
            Worker::new(
                Arc::clone(&factory_store),
                Arc::clone(&provider) as _,
                Arc::clone(&factory_machine),
                "messages-test",
            )
        }),
        TOKEN,
        AgentSpec {
            model: "scripted".into(),
            system: String::new(),
            tools: vec![],
            max_turns: 10,
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
        http: reqwest::Client::new(),
    }
}

impl Office {
    /// Creates a run straight in the store (the API has no way to ask for
    /// a conversation yet) and has the office drive it.
    async fn conversation(&self) -> RunId {
        let run = self.create(Runtime::Native, true).await;
        let resp = self
            .post(&format!("/v1/runs/{run}/resume"), json!({}), None)
            .await;
        assert_eq!(resp.status(), 202);
        self.wait_for(run, "parked", 1).await;
        run
    }

    async fn create(&self, runtime: Runtime, park_on_answer: bool) -> RunId {
        Worker::new(
            Arc::clone(&self.store),
            Arc::new(ScriptedProvider::new(vec![])) as _,
            Arc::clone(&self.machine),
            "creator",
        )
        .create(&RunSpec {
            origin: None,
            task: "Talk to me.".into(),
            agent: AgentSpec {
                model: "scripted".into(),
                system: String::new(),
                tools: vec![],
                max_turns: 10,
            },
            runtime,
            machine: None,
            park_on_answer,
        })
        .await
        .unwrap_or_else(|e| panic!("{e}"))
    }

    async fn post(&self, path: &str, body: Value, key: Option<&str>) -> reqwest::Response {
        let mut req = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(TOKEN)
            .json(&body);
        if let Some(k) = key {
            req = req.header("idempotency-key", k);
        }
        req.send().await.unwrap_or_else(|e| panic!("{e}"))
    }

    async fn say(&self, run: RunId, text: &str, key: Option<&str>) -> reqwest::Response {
        self.post(
            &format!("/v1/runs/{run}/messages"),
            json!({ "text": text }),
            key,
        )
        .await
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

    fn count(log: &[Payload], f: impl Fn(&Payload) -> bool) -> usize {
        log.iter().filter(|p| f(p)).count()
    }

    /// Waits until the run is in `state` with `answers` model responses.
    async fn wait_for(&self, run: RunId, state: &str, answers: usize) {
        for _ in 0..100 {
            let view: Value = self
                .http
                .get(format!("{}/v1/runs/{run}", self.base))
                .bearer_auth(TOKEN)
                .send()
                .await
                .unwrap_or_else(|e| panic!("{e}"))
                .json()
                .await
                .unwrap_or_else(|e| panic!("{e}"));
            let log = self.log(run).await;
            let got = Self::count(&log, |p| matches!(p, Payload::ModelResponseRecorded { .. }));
            if view["status"]["state"] == state && got == answers {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!(
            "run {run} never reached {state} with {answers} answers: {:?}",
            self.log(run).await
        );
    }
}

#[tokio::test]
async fn a_message_wakes_a_parked_run_once_even_when_retried() {
    let office = office().await;
    let run = office.conversation().await;

    let first = office
        .say(run, "Don't add a dependency.", Some("m-1"))
        .await;
    assert_eq!(first.status(), 202);
    let ack: Value = first.json().await.unwrap_or_default();
    office.wait_for(run, "parked", 2).await;

    // The retry gets the first message's seq back and changes nothing.
    let again = office
        .say(run, "Don't add a dependency.", Some("m-1"))
        .await;
    assert_eq!(again.status(), 200);
    assert_eq!(again.json::<Value>().await.unwrap_or_default(), ack);
    tokio::time::sleep(Duration::from_millis(100)).await;

    let log = office.log(run).await;
    assert_eq!(
        Office::count(&log, |p| matches!(p, Payload::UserMessage { .. })),
        1
    );
    assert_eq!(
        Office::count(&log, |p| matches!(p, Payload::RunWoken { .. })),
        1
    );
    let seq = ack["seq"].as_u64().unwrap_or(u64::MAX) as usize;
    assert!(
        matches!(&log[seq], Payload::UserMessage { author, content }
            if author == "owner" && content == "Don't add a dependency."),
        "{:?}",
        log.get(seq)
    );

    // A second message (another key) is a new turn.
    assert_eq!(office.say(run, "Thanks.", Some("m-2")).await.status(), 202);
    office.wait_for(run, "parked", 3).await;
}

#[tokio::test]
async fn messages_are_refused_where_nobody_would_read_them_and_harness_runs_take_them() {
    let office = office().await;

    // Empty.
    let run = office.conversation().await;
    assert_eq!(office.say(run, "   ", None).await.status(), 400);

    // A finished run.
    let done = office.create(Runtime::Native, false).await;
    office
        .post(&format!("/v1/runs/{done}/resume"), json!({}), None)
        .await;
    office.wait_for(done, "completed", 1).await;
    let resp = office.say(done, "One more thing.", None).await;
    assert_eq!(resp.status(), 409);

    // A harness run takes messages too (delivered when its process
    // finishes). `/bin/false` stands in for Claude Code, so the drive the
    // message starts fails at once instead of running a real harness.
    let mut spec = HarnessSpec::claude_code();
    spec.binary = Some("/bin/false".into());
    let harness = office.create(Runtime::Harness(spec), false).await;
    assert_eq!(office.say(harness, "Hello?", None).await.status(), 202);

    // Unknown run, and no token.
    let resp = office.say(RunId::new(), "Hi.", None).await;
    assert_eq!(resp.status(), 404);
    let anonymous = office
        .http
        .post(format!("{}/v1/runs/{run}/messages", office.base))
        .json(&json!({ "text": "Hi." }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(anonymous.status(), 401);
}

#[tokio::test]
async fn now_asks_for_an_interrupt_right_after_the_message() {
    let office = office().await;
    let mut spec = HarnessSpec::claude_code();
    spec.binary = Some("/bin/false".into());
    let run = office.create(Runtime::Harness(spec), false).await;
    let resp = office
        .post(
            &format!("/v1/runs/{run}/messages"),
            json!({ "text": "Stop, wrong file.", "now": true }),
            None,
        )
        .await;
    assert_eq!(resp.status(), 202);
    let ack: Value = resp.json().await.unwrap_or_else(|e| panic!("{e}"));
    let log = office.log(run).await;
    let seq = ack["seq"].as_u64().unwrap_or(0) as usize;
    assert!(
        matches!(&log[seq], Payload::UserMessage { content, .. } if content == "Stop, wrong file.")
    );
    assert!(matches!(&log[seq + 1], Payload::InterruptRequested { by } if by == "owner"));

    // Without `now`, no interrupt.
    office.say(run, "And the README.", None).await;
    let log = office.log(run).await;
    assert_eq!(
        Office::count(&log, |p| matches!(p, Payload::InterruptRequested { .. })),
        1
    );
}

#[tokio::test]
async fn cancelling_a_parked_run_closes_it() {
    let office = office().await;
    // A parked run has no worker; the cancel itself must get it closed.
    let run = office.conversation().await;
    let resp = office
        .post(&format!("/v1/runs/{run}/cancel"), json!({"reason": "not needed"}), None)
        .await;
    assert_eq!(resp.status(), 202);
    office.wait_for(run, "cancelled", 1).await;
}
