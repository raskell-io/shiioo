//! `POST /v1/runs/{id}/follow-up`: the owner continues a finished harness
//! run in a new run on the same branch and session.

use serde_json::{Value, json};
use shiioo_api::{ApiState, router};
use shiioo_core::{EventStore, Payload, RunId, Seq, SystemClock};
use shiioo_runtime::{
    AgentSpec, HarnessSpec, LocalMachine, Origin, RunSpec, Runtime, ScriptedProvider, Toolbox,
    Worker,
};
use shiioo_store::{MemoryStore, Notifying};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const TOKEN: &str = "test-token";
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../shiioo-runtime/tests/fixtures/claude-code-read.jsonl"
);
const SESSION: &str = "a11a080d-e85c-457e-9b31-578ebfba0c1c";

type Store = Notifying<MemoryStore<SystemClock>>;

struct Office {
    base: String,
    store: Arc<Store>,
    machine: Arc<LocalMachine>,
    http: reqwest::Client,
    /// Holds the fake `claude` and the arguments of each invocation.
    dir: tempfile::TempDir,
}

async fn office() -> Office {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let d = dir.path().display();
    std::fs::write(
        dir.path().join("claude"),
        format!(
            "#!/bin/sh\n{{ printf '%s\\n' \"$@\"; echo ----; }} >> '{d}/args.txt'\ncat '{FIXTURE}'\n"
        ),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    std::fs::create_dir_all(dir.path().join("workspace")).unwrap_or_else(|e| panic!("{e}"));
    let store: Arc<Store> = Arc::new(Notifying::new(MemoryStore::new(SystemClock)));
    let machine = Arc::new(LocalMachine::in_memory(
        dir.path().join("workspace"),
        Toolbox::builtin(),
    ));
    let provider = Arc::new(ScriptedProvider::new(vec![ScriptedProvider::answer(
        "Hello.",
    )]));
    let (factory_store, factory_machine) = (Arc::clone(&store), Arc::clone(&machine));
    let state = ApiState::new(
        Arc::clone(&store),
        Arc::new(move || {
            Worker::new(
                Arc::clone(&factory_store),
                Arc::clone(&provider) as _,
                Arc::clone(&factory_machine),
                "follow-up-test",
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
        dir,
    }
}

impl Office {
    fn harness(&self) -> Runtime {
        let mut spec = HarnessSpec::claude_code();
        spec.binary = Some(PathBuf::from("/bin/sh"));
        spec.binary_args = vec![self.dir.path().join("claude").to_string_lossy().into_owned()];
        Runtime::Harness(spec)
    }

    /// Creates a run straight in the store; with `drive`, has the office
    /// drive it to its end.
    async fn create(&self, runtime: Runtime, origin: Option<Origin>, drive: bool) -> RunId {
        let run = Worker::new(
            Arc::clone(&self.store),
            Arc::new(ScriptedProvider::new(vec![])) as _,
            Arc::clone(&self.machine),
            "creator",
        )
        .create(&RunSpec {
            task: "Read README.md and report its first line.".into(),
            agent: AgentSpec {
                model: String::new(),
                system: String::new(),
                tools: vec![],
                max_turns: 10,
            },
            runtime,
            machine: None,
            park_on_answer: false,
            origin,
        })
        .await
        .unwrap_or_else(|e| panic!("{e}"));
        if drive {
            let resp = self
                .post(&format!("/v1/runs/{run}/resume"), json!({}), None)
                .await;
            assert_eq!(resp.status(), 202);
            self.wait_for(run, "completed").await;
        }
        run
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

    async fn follow_up(&self, run: RunId, text: &str, key: Option<&str>) -> reqwest::Response {
        self.post(
            &format!("/v1/runs/{run}/follow-up"),
            json!({ "text": text }),
            key,
        )
        .await
    }

    async fn spec(&self, run: RunId) -> RunSpec {
        let log = self
            .store
            .read(run, Seq(0))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        match log.first().map(|e| &*e.payload) {
            Some(Payload::RunCreated { spec }) => {
                serde_json::from_value(spec.clone()).unwrap_or_else(|e| panic!("{e}"))
            }
            other => panic!("{other:?}"),
        }
    }

    async fn wait_for(&self, run: RunId, state: &str) {
        for _ in 0..200 {
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
            if view["status"]["state"] == state {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("run {run} never reached {state}");
    }

    /// Each invocation of the fake `claude`.
    fn invocations(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.dir.path().join("args.txt"))
            .unwrap_or_default()
            .split("----\n")
            .filter(|s| !s.is_empty())
            .map(|s| s.lines().map(str::to_owned).collect())
            .collect()
    }
}

fn run_id(view: &Value) -> RunId {
    RunId(
        view["id"]
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .unwrap_or_else(|| panic!("{view}")),
    )
}

#[tokio::test]
async fn a_follow_up_continues_the_session_and_names_the_original() {
    let office = office().await;
    let first = office.create(office.harness(), None, true).await;
    let end = office
        .store
        .read(first, Seq(0))
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .len() as u64
        - 1;

    let resp = office
        .follow_up(first, "Now count its lines.", Some("f-1"))
        .await;
    assert_eq!(resp.status(), 201);
    let view: Value = resp.json().await.unwrap_or_default();
    let second = run_id(&view);
    assert_eq!(view["origin"], json!(first.to_string()));
    assert_eq!(view["task"], "Now count its lines.");
    office.wait_for(second, "completed").await;

    let spec = office.spec(second).await;
    assert_eq!(
        spec.origin,
        Some(Origin {
            run: first,
            seq: Seq(end),
            workspace: first,
            session: Some(SESSION.into()),
        })
    );
    assert_eq!(spec.runtime, office.harness());
    let calls = office.invocations();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls[1].iter().any(|a| a == SESSION), "{:?}", calls[1]);
    assert!(calls[1].iter().any(|a| a == "Now count its lines."));

    // A retry gets the same run back and starts nothing.
    let again = office
        .follow_up(first, "Now count its lines.", Some("f-1"))
        .await;
    assert_eq!(again.status(), 200);
    assert_eq!(run_id(&again.json().await.unwrap_or_default()), second);

    // A follow-up of a follow-up stays on the first run's branch.
    let resp = office.follow_up(second, "And the words.", None).await;
    assert_eq!(resp.status(), 201);
    let third = run_id(&resp.json().await.unwrap_or_default());
    let origin = office.spec(third).await.origin;
    assert_eq!(origin.as_ref().map(|o| o.run), Some(second));
    assert_eq!(origin.as_ref().map(|o| o.workspace), Some(first));
    office.wait_for(third, "completed").await;
}

#[tokio::test]
async fn follow_ups_are_refused_where_they_cannot_continue_anything() {
    let office = office().await;
    let first = office.create(office.harness(), None, true).await;

    assert_eq!(office.follow_up(first, "  ", None).await.status(), 400);
    assert_eq!(
        office.follow_up(RunId::new(), "Hi.", None).await.status(),
        404
    );

    // A native run has no session to continue.
    let native = office.create(Runtime::Native, None, true).await;
    assert_eq!(office.follow_up(native, "More.", None).await.status(), 409);

    // An open run takes messages, not follow-ups.
    let open = office.create(office.harness(), None, false).await;
    assert_eq!(office.follow_up(open, "More.", None).await.status(), 409);

    // Another run still works on the first run's branch.
    let busy = office
        .create(
            office.harness(),
            Some(Origin {
                run: first,
                seq: Seq(0),
                workspace: first,
                session: None,
            }),
            false,
        )
        .await;
    let resp = office.follow_up(first, "More.", None).await;
    assert_eq!(resp.status(), 409);
    let body = resp.text().await.unwrap_or_default();
    assert!(body.contains(&busy.to_string()[..8]), "{body}");
}
