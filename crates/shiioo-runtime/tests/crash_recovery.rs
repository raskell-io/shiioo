//! The kernel spike's exit test: kill a worker at every point of a turn,
//! let another worker take over, and check that the run finishes correctly
//! and that no side effect happens twice.

use serde_json::{Value, json};
use shiioo_core::{
    CallId, Epoch, EventStore, ManualClock, Payload, RunId, Seq, ToolClass, verify_chain,
};
use shiioo_runtime::Runtime;
use shiioo_runtime::{
    AgentSpec, BoxFuture, CrashPoint, Faults, Invocation, LocalMachine, MachineError, RunOutcome,
    RunSpec, ScriptedProvider, Tool, ToolOutput, ToolSpec, Toolbox, Worker, WorkerError,
};
use shiioo_store::{MemoryStore, RedbStore};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

const TTL: u64 = 30_000;

/// Stands in for anything that must not happen twice: a payment, a push,
/// a sent email.
struct ChargeCard(Arc<AtomicU32>);

impl Tool for ChargeCard {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "charge_card".into(),
            description: "Charges the customer's card.".into(),
            input_schema: json!({ "type": "object" }),
        }
    }

    fn class(&self, _: &Value) -> ToolClass {
        ToolClass::SideEffecting
    }

    fn execute<'a>(&'a self, input: &'a Value, _: &'a Path) -> BoxFuture<'a, ToolOutput> {
        let n = self.0.fetch_add(1, Ordering::SeqCst) + 1;
        let amount = input["amount"].clone();
        Box::pin(async move { ToolOutput::ok(format!("charged {amount} (charge #{n})")) })
    }
}

/// A pure tool, safe to run again.
struct ReadNote(Arc<AtomicU32>);

impl Tool for ReadNote {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_note".into(),
            description: "Reads the customer's note.".into(),
            input_schema: json!({ "type": "object" }),
        }
    }

    fn class(&self, _: &Value) -> ToolClass {
        ToolClass::Pure
    }

    fn execute<'a>(&'a self, _: &'a Value, _: &'a Path) -> BoxFuture<'a, ToolOutput> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { ToolOutput::ok("please deliver on Tuesday") })
    }
}

struct Fixture<S> {
    store: Arc<S>,
    clock: ManualClock,
    machine: Arc<LocalMachine>,
    provider: Arc<ScriptedProvider>,
    charges: Arc<AtomicU32>,
    reads: Arc<AtomicU32>,
    _dir: tempfile::TempDir,
}

impl<S: EventStore + 'static> Fixture<S> {
    fn new(make_store: impl FnOnce(&Path, ManualClock) -> S) -> Self {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let clock = ManualClock::at(1_700_000_000_000);
        let charges = Arc::new(AtomicU32::new(0));
        let reads = Arc::new(AtomicU32::new(0));
        let tools = Toolbox::default()
            .with(Arc::new(ChargeCard(Arc::clone(&charges))))
            .with(Arc::new(ReadNote(Arc::clone(&reads))));
        let machine = LocalMachine::open(dir.path(), dir.path().join("journal.jsonl"), tools)
            .unwrap_or_else(|e| panic!("machine: {e}"));
        Self {
            store: Arc::new(make_store(dir.path(), clock.clone())),
            clock,
            machine: Arc::new(machine),
            provider: Arc::new(ScriptedProvider::new(vec![
                ScriptedProvider::tool_use("t1", "charge_card", json!({ "amount": 42 })),
                ScriptedProvider::tool_use("t2", "read_note", json!({})),
                ScriptedProvider::answer("Charged 42 and noted Tuesday delivery."),
            ])),
            charges,
            reads,
            _dir: dir,
        }
    }

    fn worker(&self, id: &str, crash_at: Option<CrashPoint>) -> Worker<S> {
        Worker::new(
            Arc::clone(&self.store),
            Arc::clone(&self.provider) as _,
            Arc::clone(&self.machine),
            id,
        )
        .with_faults(Faults { crash_at })
    }

    async fn start(&self) -> RunId {
        let spec = RunSpec {
            origin: None,
            park_on_answer: false,
            task: "Charge the customer 42 and check their note.".into(),
            agent: AgentSpec {
                model: "scripted".into(),
                system: String::new(),
                tools: vec!["charge_card".into(), "read_note".into()],
                max_turns: 10,
            },
            runtime: Runtime::Native,
            machine: None,
        };
        self.worker("creator", None)
            .create(&spec)
            .await
            .unwrap_or_else(|e| panic!("create: {e}"))
    }

    async fn payloads(&self, run: RunId) -> Vec<Payload> {
        let log = self
            .store
            .read(run, Seq(0))
            .await
            .unwrap_or_else(|e| panic!("read: {e}"));
        assert_eq!(verify_chain(&log), Ok(()), "log must stay a valid chain");
        log.iter().map(|e| (*e.payload).clone()).collect()
    }
}

fn memory(_: &Path, clock: ManualClock) -> MemoryStore<ManualClock> {
    MemoryStore::new(clock)
}

fn redb(dir: &Path, clock: ManualClock) -> RedbStore {
    RedbStore::with_clock(dir.join("shiioo.redb"), clock).unwrap_or_else(|e| panic!("redb: {e}"))
}

async fn crash_then_recover<S: EventStore + 'static>(fx: Fixture<S>, point: CrashPoint) {
    let run = fx.start().await;

    let first = fx.worker("w1", Some(point)).drive(run).await;
    assert!(
        matches!(first, Err(WorkerError::Crashed(p)) if p == point),
        "{point:?}: expected a crash, got {first:?}"
    );

    // w1 is dead and never released its lease. Once it expires, w2 takes over.
    fx.clock.advance_ms(TTL + 1);
    let second = fx
        .worker("w2", None)
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{point:?}: w2 failed: {e}"));

    let payloads = fx.payloads(run).await;
    let takeovers = payloads
        .iter()
        .filter(|p| matches!(p, Payload::LeaseAcquired { .. }))
        .count();
    assert_eq!(takeovers, 2, "{point:?}");
    assert_eq!(
        fx.charges.load(Ordering::SeqCst),
        1,
        "{point:?}: the card must be charged exactly once"
    );

    if point == CrashPoint::DuringToolExecution {
        // The charge happened but nobody recorded it, not even the machine.
        // Retrying could charge twice, so the run stops for a human.
        assert!(
            matches!(second, RunOutcome::NeedsAttention(_)),
            "{second:?}"
        );
        assert!(payloads.iter().any(|p| matches!(
            p,
            Payload::ToolOutcomeUnknown { call_id } if *call_id == CallId::from("t1")
        )));
        assert!(!payloads.iter().any(Payload::is_terminal));
    } else {
        assert_eq!(
            second,
            RunOutcome::Completed("Charged 42 and noted Tuesday delivery.".into()),
            "{point:?}"
        );
        assert!(matches!(
            payloads.last(),
            Some(Payload::RunCompleted { .. })
        ));
        let results_for_t1 = payloads
            .iter()
            .filter(|p| matches!(p, Payload::ToolResult { call_id, .. } if *call_id == CallId::from("t1")))
            .count();
        assert_eq!(results_for_t1, 1, "{point:?}: one recorded result per call");
    }
}

const ALL_POINTS: [CrashPoint; 5] = [
    CrashPoint::AfterModelResponse,
    CrashPoint::AfterToolIntent,
    CrashPoint::DuringToolExecution,
    CrashPoint::AfterToolExecuted,
    CrashPoint::AfterToolResult,
];

#[tokio::test]
async fn every_crash_point_recovers_on_memory() {
    for point in ALL_POINTS {
        crash_then_recover(Fixture::new(memory), point).await;
    }
}

#[tokio::test]
async fn every_crash_point_recovers_on_redb() {
    for point in ALL_POINTS {
        crash_then_recover(Fixture::new(redb), point).await;
    }
}

#[tokio::test]
async fn happy_path() {
    let fx = Fixture::new(redb);
    let run = fx.start().await;
    let outcome = fx
        .worker("w1", None)
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        outcome,
        RunOutcome::Completed("Charged 42 and noted Tuesday delivery.".into())
    );
    assert_eq!(fx.charges.load(Ordering::SeqCst), 1);
    assert_eq!(fx.reads.load(Ordering::SeqCst), 1);
    assert_eq!(fx.provider.calls(), 3);

    // Driving a finished run again does nothing and reports the outcome.
    let again = fx
        .worker("w2", None)
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(again, outcome);
    assert_eq!(fx.charges.load(Ordering::SeqCst), 1);
}

/// The split-brain case at the machine: w1 records its intent, then stalls
/// instead of dying. w2 takes over and charges the card. When w1 wakes up
/// and sends its stale invocation, the machine refuses it, and the store
/// refuses its writes.
#[tokio::test]
async fn stalled_worker_cannot_act_after_takeover() {
    let fx = Fixture::new(memory);
    let run = fx.start().await;
    let stalled = fx
        .worker("w1", Some(CrashPoint::AfterToolIntent))
        .drive(run)
        .await;
    assert!(matches!(stalled, Err(WorkerError::Crashed(_))));

    fx.clock.advance_ms(TTL + 1);
    let outcome = fx
        .worker("w2", None)
        .drive(run)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(matches!(outcome, RunOutcome::Completed(_)));
    assert_eq!(fx.charges.load(Ordering::SeqCst), 1);

    // w1 wakes up with its epoch-1 lease and sends the charge it intended.
    let late = fx
        .machine
        .execute(Invocation {
            run_id: run,
            epoch: Epoch(1),
            call_id: &CallId::from("t1"),
            tool: "charge_card",
            input: &json!({ "amount": 42 }),
        })
        .await;
    assert!(
        matches!(
            late,
            Err(MachineError::Fenced {
                presented: Epoch(1),
                current: Epoch(2)
            })
        ),
        "{late:?}"
    );
    assert_eq!(fx.charges.load(Ordering::SeqCst), 1);
}

/// The machine's journal survives a restart of the machine itself: a call
/// that finished before the restart is answered from the journal, not run
/// again.
#[tokio::test]
async fn journal_survives_machine_restart() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let charges = Arc::new(AtomicU32::new(0));
    let open = || {
        LocalMachine::open(
            dir.path(),
            dir.path().join("journal.jsonl"),
            Toolbox::default().with(Arc::new(ChargeCard(Arc::clone(&charges)))),
        )
        .unwrap_or_else(|e| panic!("{e}"))
    };
    let run = RunId::new();
    let call = CallId::from("c1");
    let input = json!({ "amount": 1 });
    let inv = Invocation {
        run_id: run,
        epoch: Epoch(1),
        call_id: &call,
        tool: "charge_card",
        input: &input,
    };

    let first = open().execute(inv).await.unwrap_or_else(|e| panic!("{e}"));
    let restarted = open();
    let second = restarted
        .execute(inv)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(first, second);
    assert_eq!(charges.load(Ordering::SeqCst), 1);

    // The fence also survived: epoch 0 is refused.
    let stale = Invocation {
        epoch: Epoch(0),
        ..inv
    };
    assert!(matches!(
        restarted.execute(stale).await,
        Err(MachineError::Fenced { .. })
    ));
}
