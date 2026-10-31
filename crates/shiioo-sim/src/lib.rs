//! Seeded, reproducible crash-recovery simulation for Shiioo's runtime.
//!
//! Each seed generates a [`Scenario`]: a scripted model conversation with
//! side-effecting, pure and idempotent tool calls, a store backend and a
//! machine with a file journal. The run is then driven by a sequence of
//! worker "lives". A life may crash at any [`CrashPoint`], race another
//! worker for the lease, be followed by a machine restart, or leave behind
//! a stale invocation that is replayed after a takeover. When the run has
//! finished (or stopped for a human), the invariants in [`check`] are
//! verified. The same seed always produces the same scenario and the same
//! sequence of lives.

use serde_json::{Value, json};
use shiioo_core::{
    CallId, Epoch, Event, EventStore, ManualClock, Payload, RunId, Seq, ToolClass, verify_chain,
};
use shiioo_runtime::{
    AgentSpec, BoxFuture, CallStatus, CrashPoint, Faults, Invocation, LocalMachine, MachineError,
    ModelResponse, RunOutcome, RunSpec, Runtime, ScriptedProvider, StopReason, Tool, ToolOutput,
    ToolSpec, Toolbox, Usage, Worker, WorkerError,
};
use shiioo_store::{MemoryStore, RedbStore};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const TTL_MS: u64 = 30_000;
/// After this many lives, faults stop so the run can finish.
const FAULTY_LIVES: usize = 25;
const MAX_LIVES: usize = 50;
/// Seeds the RNG for the owner's messages (`Scenario::conversation`).
const TALK_STREAM: u64 = 0x7A1C_0FFE_E5EE_D5ED;

const CRASH_POINTS: [CrashPoint; 5] = [
    CrashPoint::AfterModelResponse,
    CrashPoint::AfterToolIntent,
    CrashPoint::DuringToolExecution,
    CrashPoint::AfterToolExecuted,
    CrashPoint::AfterToolResult,
];

/// splitmix64: tiny, fast, and good enough to pick scenarios.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n` (`n > 0`).
    pub fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n
    }

    /// True with probability `percent / 100`.
    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    pub fn pick<T: Copy>(&mut self, items: &[T]) -> T {
        items[self.below(items.len() as u64) as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Memory,
    Redb,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ToolKind {
    /// Side-effecting: must never run twice.
    Charge,
    /// Pure: may run again.
    Read,
    /// Idempotent: may run again with the same key.
    Write,
}

impl ToolKind {
    pub fn name(self) -> &'static str {
        match self {
            ToolKind::Charge => "charge",
            ToolKind::Read => "read",
            ToolKind::Write => "write",
        }
    }

    fn class(self, call: &str) -> ToolClass {
        match self {
            ToolKind::Charge => ToolClass::SideEffecting,
            ToolKind::Read => ToolClass::Pure,
            ToolKind::Write => ToolClass::Idempotent {
                key: call.to_owned(),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlannedCall {
    pub id: String,
    pub tool: ToolKind,
}

/// Everything a seed decides up front. The per-life choices (faults,
/// races, restarts, replays) are drawn from the same RNG while running.
#[derive(Clone, Debug)]
pub struct Scenario {
    pub seed: u64,
    pub backend: Backend,
    /// Tool-calling turns; a final answer follows them.
    pub turns: Vec<Vec<PlannedCall>>,
    pub final_text: String,
    /// The run parks after each answer (`park_on_answer`) and the owner
    /// replies; otherwise it completes, and messages arrive mid-run.
    pub conversation: bool,
    /// How many messages the owner sends (0–3).
    pub messages: usize,
}

impl Scenario {
    pub fn generate(seed: u64, rng: &mut Rng) -> Self {
        let backend = if seed.is_multiple_of(2) {
            Backend::Memory
        } else {
            Backend::Redb
        };
        // 1–6 turns in total, the last one being the final answer.
        let tool_turns = rng.below(6) as usize;
        let turns = (0..tool_turns)
            .map(|t| {
                let n = 1 + rng.below(3) as usize;
                (0..n)
                    .map(|i| PlannedCall {
                        id: format!("c{t}_{i}"),
                        tool: rng.pick(&[ToolKind::Charge, ToolKind::Read, ToolKind::Write]),
                    })
                    .collect()
            })
            .collect();
        // Drawn from their own stream, so adding them left every older
        // seed's turns, faults and races exactly as they were.
        let mut talk = Rng::new(seed ^ TALK_STREAM);
        Self {
            seed,
            backend,
            turns,
            final_text: format!("done (seed {seed})"),
            conversation: talk.chance(33),
            messages: talk.below(4) as usize,
        }
    }

    fn script(&self) -> Vec<ModelResponse> {
        let mut script: Vec<ModelResponse> = self
            .turns
            .iter()
            .map(|calls| ModelResponse {
                model: "scripted".into(),
                content: calls
                    .iter()
                    .map(|c| {
                        json!({
                            "type": "tool_use",
                            "id": c.id,
                            "name": c.tool.name(),
                            "input": { "call": c.id },
                        })
                    })
                    .collect(),
                stop_reason: StopReason::ToolUse,
                usage: Usage::default(),
            })
            .collect();
        script.push(ScriptedProvider::answer(&self.final_text));
        // One answer per message the owner may send, and one spare for a
        // message that lands after an answer but before the run settles.
        for n in 1..=self.messages + 1 {
            script.push(ScriptedProvider::answer(&format!("reply {n}")));
        }
        script
    }

    fn calls(&self) -> impl Iterator<Item = &PlannedCall> {
        self.turns.iter().flatten()
    }
}

/// Counts executions per call, as seen by the tools themselves.
#[derive(Clone, Default)]
struct Executions(Arc<Mutex<BTreeMap<String, u32>>>);

impl Executions {
    fn record(&self, call: &str) {
        if let Ok(mut m) = self.0.lock() {
            *m.entry(call.to_owned()).or_default() += 1;
        }
    }

    fn get(&self, call: &str) -> u32 {
        self.0
            .lock()
            .map(|m| m.get(call).copied().unwrap_or(0))
            .unwrap_or(0)
    }
}

struct SimTool {
    kind: ToolKind,
    executions: Executions,
}

impl Tool for SimTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.kind.name().into(),
            description: format!("simulated {:?} tool", self.kind),
            input_schema: json!({ "type": "object" }),
        }
    }

    fn class(&self, input: &Value) -> ToolClass {
        self.kind.class(input["call"].as_str().unwrap_or_default())
    }

    fn execute<'a>(&'a self, input: &'a Value, _: &'a Path) -> BoxFuture<'a, ToolOutput> {
        Box::pin(async move {
            // Yield once so a racing worker gets polled mid-turn.
            tokio::task::yield_now().await;
            let call = input["call"].as_str().unwrap_or_default();
            self.executions.record(call);
            ToolOutput::ok(format!("{} {call}", self.kind.name()))
        })
    }
}

/// What happened in a scenario that passed.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub seed: u64,
    pub lives: usize,
    pub crashes: usize,
    pub races: usize,
    pub restarts: usize,
    pub stale_replays: usize,
    pub needs_attention: bool,
    /// Owner messages delivered, and times the run parked.
    pub messages: usize,
    pub parks: usize,
}

/// A broken invariant, with everything needed to reproduce it.
#[derive(Debug)]
pub struct Violation {
    pub seed: u64,
    pub message: String,
    pub log: Vec<String>,
}

impl fmt::Display for Violation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "seed {}: {}", self.seed, self.message)?;
        writeln!(
            f,
            "reproduce: SHIIOO_SIM_SEED={} cargo test -p shiioo-sim -- --nocapture",
            self.seed
        )?;
        writeln!(f, "log:")?;
        for line in &self.log {
            writeln!(f, "  {line}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Violation {}

/// Runs one seed on a fresh single-threaded Tokio runtime, so scheduling
/// (and with it any race between workers) is the same on every run.
pub fn run_scenario(seed: u64) -> Result<Report, Violation> {
    let fail = |message: String| Violation {
        seed,
        message,
        log: Vec::new(),
    };
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| fail(format!("runtime: {e}")))?;
    let dir = tempfile::tempdir().map_err(|e| fail(format!("tempdir: {e}")))?;
    let mut rng = Rng::new(seed);
    let scenario = Scenario::generate(seed, &mut rng);
    let clock = ManualClock::at(1_700_000_000_000);

    rt.block_on(async {
        match scenario.backend {
            Backend::Memory => {
                let store = Arc::new(MemoryStore::new(clock.clone()));
                Sim::new(scenario, store, clock, dir.path(), rng)?
                    .run()
                    .await
            }
            Backend::Redb => {
                let store = RedbStore::with_clock(dir.path().join("shiioo.redb"), clock.clone())
                    .map_err(|e| fail(format!("redb: {e}")))?;
                Sim::new(scenario, Arc::new(store), clock, dir.path(), rng)?
                    .run()
                    .await
            }
        }
    })
}

/// A worker that recorded a tool intent and then "stalled" instead of
/// dying. After a takeover, its late invocation must be refused.
struct Stale {
    epoch: Epoch,
    call_id: CallId,
    tool: String,
    input: Value,
    acquisitions_at_crash: usize,
}

struct Sim<S> {
    scenario: Scenario,
    store: Arc<S>,
    clock: ManualClock,
    rng: Rng,
    dir: PathBuf,
    provider: Arc<ScriptedProvider>,
    toolbox: Toolbox,
    executions: Executions,
    machine: Arc<LocalMachine>,
    /// Lease acquisitions seen by workers (via their observers).
    acquisitions: Arc<Mutex<usize>>,
    run: RunId,
    /// A side-effecting call that a `DuringToolExecution` crash hit: it ran,
    /// and nothing recorded it.
    hit_mid_execution: Option<CallId>,
    stale: Option<Stale>,
    report: Report,
    /// Decides when mid-run messages arrive; separate from `rng` so the
    /// lives of older seeds stay the same.
    talk: Rng,
}

impl<S: EventStore + 'static> Sim<S> {
    fn new(
        scenario: Scenario,
        store: Arc<S>,
        clock: ManualClock,
        dir: &Path,
        rng: Rng,
    ) -> Result<Self, Violation> {
        let seed = scenario.seed;
        let executions = Executions::default();
        let toolbox = [ToolKind::Charge, ToolKind::Read, ToolKind::Write]
            .into_iter()
            .fold(Toolbox::default(), |tb, kind| {
                tb.with(Arc::new(SimTool {
                    kind,
                    executions: executions.clone(),
                }))
            });
        let machine = open_machine(dir, &toolbox).map_err(|e| Violation {
            seed,
            message: format!("opening machine: {e}"),
            log: Vec::new(),
        })?;
        Ok(Self {
            provider: Arc::new(ScriptedProvider::new(scenario.script())),
            report: Report {
                seed,
                ..Report::default()
            },
            scenario,
            store,
            clock,
            rng,
            dir: dir.to_owned(),
            toolbox,
            executions,
            machine: Arc::new(machine),
            acquisitions: Arc::new(Mutex::new(0)),
            run: RunId::new(),
            hit_mid_execution: None,
            stale: None,
            talk: Rng::new(seed ^ TALK_STREAM ^ 1),
        })
    }

    fn worker(&self, id: String, crash_at: Option<CrashPoint>) -> Worker<S> {
        let acquisitions = Arc::clone(&self.acquisitions);
        Worker::new(
            Arc::clone(&self.store),
            Arc::clone(&self.provider) as _,
            Arc::clone(&self.machine),
            id.as_str(),
        )
        .with_faults(Faults { crash_at })
        .with_observer(Arc::new(move |ev: &Event| {
            if matches!(*ev.payload, Payload::LeaseAcquired { .. })
                && let Ok(mut n) = acquisitions.lock()
            {
                *n += 1;
            }
        }))
    }

    fn acquisitions(&self) -> usize {
        self.acquisitions.lock().map(|n| *n).unwrap_or(0)
    }

    async fn run(mut self) -> Result<Report, Violation> {
        let spec = RunSpec {
            origin: None,
            park_on_answer: self.scenario.conversation,
            task: format!("scenario {}", self.scenario.seed),
            agent: AgentSpec {
                model: "scripted".into(),
                system: String::new(),
                tools: vec!["charge".into(), "read".into(), "write".into()],
                max_turns: 50,
            },
            runtime: Runtime::Native,
            machine: None,
        };
        self.run = match self.worker("creator".into(), None).create(&spec).await {
            Ok(run) => run,
            Err(e) => return Err(self.violation(format!("create: {e}")).await),
        };

        let mut outcome = None;
        for life in 0..MAX_LIVES {
            self.report.lives = life + 1;
            // Outside a conversation the owner writes while the run works.
            if !self.scenario.conversation
                && self.report.messages < self.scenario.messages
                && self.talk.chance(30)
            {
                self.say().await?;
            }
            if self.rng.chance(15) {
                self.restart_machine().await?;
            }
            let faulty = life < FAULTY_LIVES;
            let fault = (faulty && self.rng.chance(60)).then(|| self.rng.pick(&CRASH_POINTS));
            // Races only on the memory store: redb runs on the blocking
            // pool, where interleaving would depend on OS scheduling.
            let race = faulty && self.scenario.backend == Backend::Memory && self.rng.chance(20);

            let crashed = if race {
                self.report.races += 1;
                let rival_fault = self.rng.chance(50).then(|| self.rng.pick(&CRASH_POINTS));
                let a = self.worker(format!("w{life}a"), fault);
                let b = self.worker(format!("w{life}b"), rival_fault);
                let (ra, rb) = tokio::join!(a.drive(self.run), b.drive(self.run));
                let ca = self.settle(ra, &mut outcome).await?;
                let cb = self.settle(rb, &mut outcome).await?;
                ca || cb
            } else {
                let w = self.worker(format!("w{life}"), fault);
                let r = w.drive(self.run).await;
                self.settle(r, &mut outcome).await?
            };

            self.check_chain().await?;
            if crashed {
                self.report.crashes += 1;
                self.note_crash(fault).await?;
                // The dead worker never releases its lease; wait it out.
                self.clock.advance_ms(TTL_MS + 1);
            }
            self.maybe_replay_stale().await?;
            if matches!(outcome, Some(RunOutcome::Parked(_))) {
                self.report.parks += 1;
                // The owner answers; the next life must wake the run.
                if self.report.messages < self.scenario.messages {
                    self.say().await?;
                    outcome = None;
                }
            }
            if outcome.is_some() {
                break;
            }
        }

        match outcome {
            Some(outcome) => self.check(outcome).await,
            None => Err(self
                .violation(format!("run did not finish within {MAX_LIVES} lives"))
                .await),
        }
    }

    /// The owner sends the next message, without a lease.
    async fn say(&mut self) -> Result<(), Violation> {
        self.report.messages += 1;
        let text = format!("message {}", self.report.messages);
        match self
            .store
            .append_external(
                self.run,
                vec![Payload::UserMessage {
                    author: "owner".into(),
                    content: text,
                }],
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => Err(self.violation(format!("sending a message: {e}")).await),
        }
    }

    /// Classifies one `drive` result. Returns whether the worker crashed.
    async fn settle(
        &mut self,
        result: Result<RunOutcome, WorkerError>,
        outcome: &mut Option<RunOutcome>,
    ) -> Result<bool, Violation> {
        match result {
            Ok(RunOutcome::LostLease) => Err(self
                .violation("lost lease, but the clock never moved during a life".into())
                .await),
            Ok(o) => {
                *outcome = Some(o);
                Ok(false)
            }
            Err(WorkerError::Crashed(_))
            | Err(WorkerError::Machine(MachineError::SimulatedCrash)) => Ok(true),
            // The loser of a race: someone else holds a valid lease.
            Err(WorkerError::Store(shiioo_core::StoreError::LeaseHeld { .. })) => Ok(false),
            Err(e) => Err(self.violation(format!("worker error: {e}")).await),
        }
    }

    /// Remembers what a crash left behind: a side-effecting call that ran
    /// unrecorded, or an intent a stalled worker might still send.
    async fn note_crash(&mut self, fault: Option<CrashPoint>) -> Result<(), Violation> {
        let log = self.log().await?;
        let Some(Payload::ToolIntent {
            call_id,
            tool,
            class,
            input,
        }) = log.last().map(|e| (*e.payload).clone())
        else {
            return Ok(());
        };
        let epoch = log.last().map_or(Epoch(0), |e| e.epoch);
        match fault {
            Some(CrashPoint::DuringToolExecution) if class == ToolClass::SideEffecting => {
                self.hit_mid_execution = Some(call_id);
            }
            Some(CrashPoint::AfterToolIntent) if self.stale.is_none() => {
                self.stale = Some(Stale {
                    epoch,
                    call_id,
                    tool,
                    input,
                    acquisitions_at_crash: self.acquisitions(),
                });
            }
            _ => {}
        }
        Ok(())
    }

    /// After someone else has taken the run over, the stalled worker wakes
    /// up and sends its invocation. The machine must refuse it.
    async fn maybe_replay_stale(&mut self) -> Result<(), Violation> {
        let Some(stale) = &self.stale else {
            return Ok(());
        };
        if self.acquisitions() <= stale.acquisitions_at_crash || !self.rng.chance(60) {
            return Ok(());
        }
        let Some(stale) = self.stale.take() else {
            return Ok(());
        };
        self.report.stale_replays += 1;
        let before = self.executions.get(&stale.call_id.0);
        let result = self
            .machine
            .execute(Invocation {
                run_id: self.run,
                epoch: stale.epoch,
                call_id: &stale.call_id,
                tool: &stale.tool,
                input: &stale.input,
            })
            .await;
        if !matches!(result, Err(MachineError::Fenced { .. })) {
            return Err(self
                .violation(format!(
                    "stale invocation of {} from epoch {:?} was not fenced: {result:?}",
                    stale.call_id.0, stale.epoch
                ))
                .await);
        }
        if self.executions.get(&stale.call_id.0) != before {
            return Err(self
                .violation(format!(
                    "stale invocation of {} ran the tool",
                    stale.call_id.0
                ))
                .await);
        }
        Ok(())
    }

    async fn restart_machine(&mut self) -> Result<(), Violation> {
        self.report.restarts += 1;
        match open_machine(&self.dir, &self.toolbox) {
            Ok(machine) => {
                self.machine = Arc::new(machine);
                Ok(())
            }
            Err(e) => Err(self.violation(format!("reopening machine: {e}")).await),
        }
    }

    async fn check_chain(&self) -> Result<(), Violation> {
        let log = self.log().await?;
        if let Err(e) = verify_chain(&log) {
            return Err(self.violation(format!("chain broken: {e}")).await);
        }
        Ok(())
    }

    /// The invariants every finished scenario must satisfy.
    async fn check(mut self, outcome: RunOutcome) -> Result<Report, Violation> {
        let log = self.log().await?;
        let mut problems = Vec::new();

        if let Err(e) = verify_chain(&log) {
            problems.push(format!("chain broken: {e}"));
        }
        if log.windows(2).any(|w| w[1].epoch < w[0].epoch) {
            problems.push("epochs decreased".into());
        }
        let leases = log
            .iter()
            .filter(|e| matches!(*e.payload, Payload::LeaseAcquired { .. }))
            .count();
        if leases != self.acquisitions() {
            problems.push(format!(
                "{leases} LeaseAcquired events, but workers acquired {} leases",
                self.acquisitions()
            ));
        }

        // At most one result per call, unless a later one resolves an
        // unknown outcome.
        let mut results: HashMap<&CallId, usize> = HashMap::new();
        let mut unknown: Vec<&CallId> = Vec::new();
        for ev in &log {
            match &*ev.payload {
                Payload::ToolResult { call_id, .. } => {
                    let n = results.entry(call_id).or_default();
                    *n += 1;
                    if *n > 1 && !unknown.contains(&call_id) {
                        problems.push(format!("call {} has {} results", call_id.0, n));
                    }
                }
                Payload::ToolOutcomeUnknown { call_id } => unknown.push(call_id),
                _ => {}
            }
        }

        // A parked run holds no lease: until a worker takes it again only
        // external input follows RunParked, and that worker's first own
        // event is RunWoken, after a message.
        let mut parked_at: Option<usize> = None;
        let mut lease_since_park = false;
        let mut message_since_park = false;
        for (i, ev) in log.iter().enumerate() {
            let p = &*ev.payload;
            if let Some(at) = parked_at {
                match p {
                    Payload::LeaseAcquired { .. } => lease_since_park = true,
                    Payload::UserMessage { .. } => message_since_park = true,
                    _ if p.is_external() => {}
                    Payload::RunWoken { .. } if lease_since_park && message_since_park => {
                        parked_at = None;
                    }
                    _ => {
                        problems.push(format!(
                            "#{i} {} follows the park at #{at} without a wake",
                            compact(ev)
                        ));
                        parked_at = None;
                    }
                }
            }
            if matches!(p, Payload::RunParked { .. }) {
                parked_at = Some(i);
                lease_since_park = false;
                message_since_park = false;
            }
        }
        // Every message the owner sent was seen by a later model call.
        let unanswered: Vec<usize> = log
            .iter()
            .enumerate()
            .filter(|(_, e)| matches!(*e.payload, Payload::UserMessage { .. }))
            .map(|(m, _)| m)
            .filter(|&m| {
                !log.iter().enumerate().skip(m + 1).any(|(r, e)| {
                    matches!(&*e.payload, Payload::ModelResponseRecorded { context_seq, .. }
                        if context_seq.map_or(r, |s| s.0 as usize) > m)
                })
            })
            .collect();
        let last_answer = log.iter().rev().find_map(|e| match &*e.payload {
            Payload::ModelResponseRecorded { response, .. } => {
                response["content"][0]["text"].as_str().map(str::to_owned)
            }
            _ => None,
        });

        // Side effects at most once.
        for call in self.scenario.calls().filter(|c| c.tool == ToolKind::Charge) {
            let n = self.executions.get(&call.id);
            if n > 1 {
                problems.push(format!("side-effecting call {} ran {n} times", call.id));
            }
        }

        match &outcome {
            RunOutcome::NeedsAttention(_) => {
                self.report.needs_attention = true;
                match (&self.hit_mid_execution, unknown.as_slice()) {
                    (Some(hit), [only]) if *only == hit => {
                        let epoch = log.last().map_or(Epoch(0), |e| e.epoch);
                        match self.machine.status(self.run, epoch, hit) {
                            Ok(CallStatus::Started) => {}
                            other => problems.push(format!(
                                "needs attention for {}, but the machine says {other:?}",
                                hit.0
                            )),
                        }
                        if self.executions.get(&hit.0) != 1 {
                            problems.push(format!(
                                "unknown call {} ran {} times, expected 1",
                                hit.0,
                                self.executions.get(&hit.0)
                            ));
                        }
                    }
                    (hit, unknown) => problems.push(format!(
                        "needs attention without a matching mid-execution crash \
                         (crash hit {hit:?}, unknown outcomes {unknown:?})"
                    )),
                }
            }
            RunOutcome::Parked(waiting_for) => {
                if !self.scenario.conversation {
                    problems.push("parked, but the run was not a conversation".into());
                }
                if self.report.messages != self.scenario.messages {
                    problems.push(format!(
                        "stopped parked after {} of {} messages",
                        self.report.messages, self.scenario.messages
                    ));
                }
                if waiting_for != "input"
                    || !matches!(
                        log.last().map(|e| &*e.payload),
                        Some(Payload::RunParked { .. })
                    )
                {
                    problems.push(format!(
                        "parked for {waiting_for:?}, but the log does not end with RunParked"
                    ));
                }
                if !unanswered.is_empty() {
                    problems.push(format!("messages at {unanswered:?} were never answered"));
                }
            }
            RunOutcome::Completed(text) => {
                if self.scenario.conversation {
                    problems.push("a conversation completed instead of parking".into());
                }
                if Some(text) != last_answer.as_ref() {
                    problems.push(format!(
                        "completed with {text:?}, but the last answer was {last_answer:?}"
                    ));
                }
                if !unanswered.is_empty() {
                    problems.push(format!(
                        "completed with messages at {unanswered:?} unanswered"
                    ));
                }
                if let Some(hit) = &self.hit_mid_execution {
                    problems.push(format!(
                        "completed although side-effecting call {} ran unrecorded",
                        hit.0
                    ));
                }
                for call in self.scenario.calls() {
                    if results.get(&CallId(call.id.clone())).copied() != Some(1) {
                        problems.push(format!("call {} does not have exactly one result", call.id));
                    }
                    if call.tool == ToolKind::Charge && self.executions.get(&call.id) != 1 {
                        problems.push(format!(
                            "side-effecting call {} ran {} times, expected 1",
                            call.id,
                            self.executions.get(&call.id)
                        ));
                    }
                }
                if !matches!(
                    log.last().map(|e| &*e.payload),
                    Some(Payload::RunCompleted { .. })
                ) {
                    problems.push("completed, but the log does not end with RunCompleted".into());
                }
            }
            other => problems.push(format!("unexpected outcome {other:?}")),
        }

        if problems.is_empty() {
            Ok(self.report)
        } else {
            Err(self.violation(problems.join("; ")).await)
        }
    }

    async fn log(&self) -> Result<Vec<Event>, Violation> {
        self.store
            .read(self.run, Seq(0))
            .await
            .map_err(|e| Violation {
                seed: self.scenario.seed,
                message: format!("reading the log: {e}"),
                log: Vec::new(),
            })
    }

    async fn violation(&self, message: String) -> Violation {
        let log = self
            .store
            .read(self.run, Seq(0))
            .await
            .map(|events| events.iter().map(compact).collect())
            .unwrap_or_default();
        Violation {
            seed: self.scenario.seed,
            message: format!("{message} (scenario: {:?})", self.scenario),
            log,
        }
    }
}

fn open_machine(dir: &Path, toolbox: &Toolbox) -> Result<LocalMachine, MachineError> {
    LocalMachine::open(dir, dir.join("journal.jsonl"), toolbox.clone())
}

/// One line per event, for violation reports.
pub fn compact(ev: &Event) -> String {
    let what = match &*ev.payload {
        Payload::RunCreated { .. } => "run created".to_owned(),
        Payload::LeaseAcquired { worker, .. } => format!("lease → {}", worker.0),
        Payload::ModelResponseRecorded { response, .. } => {
            let calls: Vec<&str> = response["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|b| b["id"].as_str())
                .collect();
            if calls.is_empty() {
                "model: final answer".to_owned()
            } else {
                format!("model: calls {}", calls.join(","))
            }
        }
        Payload::ToolIntent {
            call_id,
            tool,
            class,
            ..
        } => {
            format!("intent {} {tool} {class:?}", call_id.0)
        }
        Payload::ToolResult {
            call_id, is_error, ..
        } => {
            format!(
                "result {}{}",
                call_id.0,
                if *is_error { " (error)" } else { "" }
            )
        }
        Payload::ToolOutcomeUnknown { call_id } => format!("unknown {}", call_id.0),
        Payload::HarnessSessionStarted { session_id, .. } => format!("harness {session_id}"),
        Payload::HarnessResult { subtype, .. } => format!("harness result {subtype}"),
        Payload::CapacityObserved { provider, .. } => format!("capacity {provider}"),
        Payload::UserMessage { author, .. } => format!("message from {author}"),
        Payload::PolicyDecision { tool, decision, .. } => format!("rules {decision} {tool}"),
        Payload::ApprovalRequested { approval_id, .. } => format!("approval? {approval_id}"),
        Payload::ApprovalDecided {
            approval_id,
            decision,
            ..
        } => format!("approval {approval_id} {decision}"),
        Payload::CancelRequested { by, .. } => format!("cancel by {by}"),
        Payload::InterruptRequested { by } => format!("interrupt by {by}"),
        Payload::TakeoverRequested { by } => format!("takeover requested by {by}"),
        Payload::TakeoverStarted { by } => format!("takeover by {by}"),
        Payload::TakeoverEnded { reason, .. } => format!("takeover ended: {reason}"),
        Payload::SandboxStarted { level, .. } => format!("sandbox {level}"),
        Payload::SecretRedacted { name, count } => format!("redacted {name} x{count}"),
        Payload::MailSent { subject, .. } => format!("mail sent: {subject}"),
        Payload::RunParked { waiting_for, .. } => format!("parked for {waiting_for}"),
        Payload::RunWoken { reason } => format!("woken by {reason}"),
        Payload::RunCancelled { .. } => "cancelled".to_owned(),
        Payload::RunCompleted { .. } => "completed".to_owned(),
        Payload::RunFailed { reason } => format!("failed: {reason}"),
    };
    format!("#{:<3} e{} {what}", ev.seq.0, ev.epoch.0)
}
