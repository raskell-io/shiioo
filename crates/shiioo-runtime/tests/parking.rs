//! Runs that park: a conversation answers, parks without a lease, and is
//! woken by the owner's next message; a message sent during a turn is
//! answered before the run completes or parks.

use serde_json::json;
use shiioo_core::{EventStore, ManualClock, Payload, RunId, Seq, verify_chain};
use shiioo_runtime::{
    AgentSpec, BoxFuture, LocalMachine, ModelProvider, ModelRequest, ModelResponse, ProviderError,
    RunOutcome, RunSpec, RunStatus, Runtime, ScriptedProvider, Toolbox, Transcript, Worker,
    run_status,
};
use shiioo_store::MemoryStore;
use std::sync::{Arc, Mutex};

/// A message to append to a run while the model is busy.
type Interjection = (Arc<MemoryStore<ManualClock>>, RunId, String);

/// Answers turn `n` with `answers[n]` and keeps every transcript it saw.
/// With `during_turn`, appends that message to the run while the first
/// call is in flight, as the owner would from the phone.
struct Recording {
    answers: Vec<ModelResponse>,
    seen: Mutex<Vec<Transcript>>,
    during_turn: Mutex<Option<Interjection>>,
}

impl ModelProvider for Recording {
    fn complete<'a>(
        &'a self,
        request: ModelRequest<'a>,
    ) -> BoxFuture<'a, Result<ModelResponse, ProviderError>> {
        let turn = request.transcript.turns.len();
        if let Ok(mut seen) = self.seen.lock() {
            seen.push(request.transcript.clone());
        }
        let inject = self.during_turn.lock().ok().and_then(|mut d| d.take());
        let answer = self.answers.get(turn).cloned();
        Box::pin(async move {
            if let Some((store, run, text)) = inject {
                store
                    .append_external(run, vec![message(&text)])
                    .await
                    .map_err(|e| ProviderError::Decode(e.to_string()))?;
            }
            answer.ok_or_else(|| ProviderError::Decode(format!("no answer for turn {turn}")))
        })
    }
}

fn message(text: &str) -> Payload {
    Payload::UserMessage {
        author: "owner".into(),
        content: text.into(),
    }
}

struct Fixture {
    store: Arc<MemoryStore<ManualClock>>,
    machine: Arc<LocalMachine>,
    provider: Arc<Recording>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new(answers: &[&str]) -> Self {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        Self {
            store: Arc::new(MemoryStore::new(ManualClock::at(1_700_000_000_000))),
            machine: Arc::new(LocalMachine::in_memory(dir.path(), Toolbox::builtin())),
            provider: Arc::new(Recording {
                answers: answers
                    .iter()
                    .map(|a| ScriptedProvider::answer(a))
                    .collect(),
                seen: Mutex::new(Vec::new()),
                during_turn: Mutex::new(None),
            }),
            _dir: dir,
        }
    }

    fn worker(&self, id: &str) -> Worker<MemoryStore<ManualClock>> {
        Worker::new(
            Arc::clone(&self.store),
            Arc::clone(&self.provider) as _,
            Arc::clone(&self.machine),
            id,
        )
    }

    async fn start(&self, park_on_answer: bool) -> RunId {
        self.worker("w1")
            .create(&RunSpec {
                park_on_answer,
                task: "Talk to me.".into(),
                agent: AgentSpec {
                    model: "scripted".into(),
                    system: String::new(),
                    tools: vec![],
                    max_turns: 10,
                },
                runtime: Runtime::Native,
                machine: None,
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    }

    async fn drive(&self, worker: &str, run: RunId) -> RunOutcome {
        self.worker(worker)
            .drive(run)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    }

    async fn say(&self, run: RunId, text: &str) {
        self.store
            .append_external(run, vec![message(text)])
            .await
            .unwrap_or_else(|e| panic!("{e}"));
    }

    async fn log(&self, run: RunId) -> Vec<Payload> {
        let log = self
            .store
            .read(run, Seq(0))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(verify_chain(&log), Ok(()));
        log.iter().map(|e| (*e.payload).clone()).collect()
    }

    async fn status(&self, run: RunId) -> RunStatus {
        run_status(
            &self
                .store
                .read(run, Seq(0))
                .await
                .unwrap_or_else(|e| panic!("{e}")),
        )
    }

    fn seen(&self) -> Vec<Transcript> {
        self.provider
            .seen
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }
}

fn kinds(log: &[Payload]) -> Vec<String> {
    log.iter()
        .map(|p| {
            serde_json::to_value(p).unwrap_or_default()["type"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect()
}

#[tokio::test]
async fn a_conversation_parks_and_is_woken_by_a_message() {
    let fx = Fixture::new(&["Hello.", "Sure, the retry helper."]);
    let run = fx.start(true).await;

    assert_eq!(
        fx.drive("w1", run).await,
        RunOutcome::Parked("input".into())
    );
    assert_eq!(
        fx.status(run).await,
        RunStatus::Parked {
            waiting_for: "input".into()
        }
    );

    // Nothing new: driving again does nothing, and takes no lease.
    let before = fx.log(run).await.len();
    assert_eq!(
        fx.drive("w2", run).await,
        RunOutcome::Parked("input".into())
    );
    assert_eq!(fx.log(run).await.len(), before);

    // A message makes it runnable; another worker takes it at once (the
    // parker's lease ended with the park, not at its TTL) and wakes it.
    fx.say(run, "Use the existing helper.").await;
    assert_eq!(fx.status(run).await, RunStatus::Open);
    assert_eq!(
        fx.drive("w2", run).await,
        RunOutcome::Parked("input".into())
    );

    assert_eq!(
        kinds(&fx.log(run).await),
        [
            "run_created",
            "lease_acquired",
            "model_response_recorded",
            "run_parked",
            "user_message",
            "lease_acquired",
            "run_woken",
            "model_response_recorded",
            "run_parked",
        ]
    );
    // The second call saw the owner's message after the first answer.
    let seen = fx.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].turns[0].messages, vec!["Use the existing helper."]);
}

#[tokio::test]
async fn a_message_during_the_turn_is_answered_before_completing() {
    let fx = Fixture::new(&["Done.", "Done, without a new dependency."]);
    let run = fx.start(false).await;
    if let Ok(mut d) = fx.provider.during_turn.lock() {
        *d = Some((Arc::clone(&fx.store), run, "Don't add a dependency.".into()));
    }

    assert_eq!(
        fx.drive("w1", run).await,
        RunOutcome::Completed("Done, without a new dependency.".into())
    );
    let log = fx.log(run).await;
    assert_eq!(
        kinds(&log),
        [
            "run_created",
            "lease_acquired",
            "user_message",
            "model_response_recorded",
            "model_response_recorded",
            "run_completed",
        ]
    );
    let seen = fx.seen();
    assert_eq!(seen[1].turns[0].messages, vec!["Don't add a dependency."]);
}

#[tokio::test]
async fn a_message_before_the_first_answer_joins_the_task() {
    let fx = Fixture::new(&["On it."]);
    let run = fx.start(false).await;
    fx.say(run, "In the sango repo.").await;
    fx.drive("w1", run).await;
    assert_eq!(fx.seen()[0].task, "Talk to me.\n\nIn the sango repo.");
}

#[tokio::test]
async fn nothing_may_follow_a_park_in_its_batch() {
    let fx = Fixture::new(&[]);
    let run = fx.start(true).await;
    let lease = fx
        .store
        .acquire_lease(run, "w1".into(), 30_000)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let err = fx
        .store
        .append(
            &lease,
            Seq(2),
            vec![
                Payload::RunParked {
                    waiting_for: "input".into(),
                    wake_at: None,
                },
                json_step(),
            ],
        )
        .await;
    assert!(err.is_err());
}

fn json_step() -> Payload {
    Payload::ModelResponseRecorded {
        model: "scripted".into(),
        response: json!({ "content": [] }),
        context_seq: None,
    }
}
