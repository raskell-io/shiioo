//! Input the owner adds to a run while a worker is driving it: approval
//! decisions and cancellation requests are appended without the lease,
//! and the worker picks them up.

use serde_json::json;
use shiioo_core::{EventStore, ManualClock, Payload, RunId, Seq, verify_chain};
use shiioo_runtime::{
    AgentSpec, LocalMachine, RunOutcome, RunSpec, RunStatus, Runtime, ScriptedProvider, Toolbox,
    Worker, run_status,
};
use shiioo_store::MemoryStore;
use std::sync::Arc;

struct Fixture {
    store: Arc<MemoryStore<ManualClock>>,
    machine: Arc<LocalMachine>,
    provider: Arc<ScriptedProvider>,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let clock = ManualClock::at(1_700_000_000_000);
        Self {
            store: Arc::new(MemoryStore::new(clock)),
            machine: Arc::new(LocalMachine::in_memory(dir.path(), Toolbox::builtin())),
            provider: Arc::new(ScriptedProvider::new(vec![
                ScriptedProvider::tool_use(
                    "t1",
                    "write_file",
                    json!({ "path": "note.txt", "content": "hi" }),
                ),
                ScriptedProvider::answer("Done."),
            ])),
            _dir: dir,
        }
    }

    fn worker(&self) -> Worker<MemoryStore<ManualClock>> {
        Worker::new(
            Arc::clone(&self.store),
            Arc::clone(&self.provider) as _,
            Arc::clone(&self.machine),
            "w1",
        )
    }

    async fn start(&self) -> RunId {
        self.worker()
            .create(&RunSpec {
                task: "Write a note.".into(),
                agent: AgentSpec {
                    model: "scripted".into(),
                    system: String::new(),
                    tools: vec!["write_file".into()],
                    max_turns: 5,
                },
                runtime: Runtime::Native,
                machine: None,
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    }

    async fn payloads(&self, run: RunId) -> Vec<Payload> {
        let log = self
            .store
            .read(run, Seq(0))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(verify_chain(&log), Ok(()));
        log.iter().map(|e| (*e.payload).clone()).collect()
    }
}

#[tokio::test]
async fn a_cancel_request_closes_the_run_before_any_work() {
    let fx = Fixture::new();
    let run = fx.start().await;
    fx.store
        .append_external(
            run,
            vec![Payload::CancelRequested {
                by: "owner".into(),
                reason: "changed my mind".into(),
            }],
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    let outcome = fx
        .worker()
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        outcome,
        RunOutcome::Cancelled("cancelled by owner: changed my mind".into())
    );
    let p = fx.payloads(run).await;
    assert!(matches!(p.last(), Some(Payload::RunCancelled { .. })));
    assert!(
        !p.iter()
            .any(|x| matches!(x, Payload::ModelResponseRecorded { .. })),
        "no model call"
    );
    assert_eq!(fx.provider.calls(), 0);
    assert!(matches!(
        run_status(&fx.store.read(run, Seq(0)).await.unwrap_or_default()),
        RunStatus::Cancelled { .. }
    ));
}

/// The owner's decision lands between two of the worker's writes. The
/// worker's stale append gets a conflict, re-reads, and finishes anyway;
/// the decision stays in the log in its place.
#[tokio::test]
async fn owner_input_mid_run_is_absorbed_not_fatal() {
    let fx = Fixture::new();
    let run = fx.start().await;
    // Acquire a lease and write the first turn ourselves, as the worker
    // would, then slip in an external event before the worker continues.
    let worker = fx.worker();
    fx.store
        .append_external(
            run,
            vec![Payload::ApprovalDecided {
                approval_id: "ap-9".into(),
                decision: "allow".into(),
                by: "owner".into(),
                note: Some("go ahead".into()),
            }],
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    let outcome = worker.drive(run).await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(outcome, RunOutcome::Completed("Done.".into()));
    let p = fx.payloads(run).await;
    assert!(p.iter().any(
        |x| matches!(x, Payload::ApprovalDecided { approval_id, .. } if approval_id == "ap-9")
    ));
    assert!(matches!(p.last(), Some(Payload::RunCompleted { .. })));
}

#[tokio::test]
async fn status_shows_an_open_approval() {
    let fx = Fixture::new();
    let run = fx.start().await;
    fx.store
        .append_external(
            run,
            vec![Payload::ApprovalRequested {
                approval_id: "ap-1".into(),
                call_id: Some("t9".into()),
                tool: "Bash".into(),
                input: json!({ "command": "git push origin main" }),
                summary: "Bash: git push origin main".into(),
                expires_at: None,
            }],
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let log = fx.store.read(run, Seq(0)).await.unwrap_or_default();
    assert_eq!(
        run_status(&log),
        RunStatus::WaitingApproval {
            approval_id: "ap-1".into(),
            summary: "Bash: git push origin main".into()
        }
    );

    fx.store
        .append_external(
            run,
            vec![Payload::ApprovalDecided {
                approval_id: "ap-1".into(),
                decision: "deny".into(),
                by: "owner".into(),
                note: None,
            }],
        )
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let log = fx.store.read(run, Seq(0)).await.unwrap_or_default();
    assert_eq!(run_status(&log), RunStatus::Open);
}
