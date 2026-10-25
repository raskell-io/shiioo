use crate::context;
use crate::harness::claude_code::HookEndpoint;
use crate::harness::{self, HarnessError, HarnessProcess};
use crate::machine::{CallStatus, Invocation, LocalMachine, MachineError};
use crate::model::{
    ModelProvider, ModelRequest, ModelResponse, ProviderError, StopReason, ToolCall,
};
use crate::spec::{HarnessSpec, RunSpec, Runtime};
use crate::status::{as_text, unresolved_unknown};
use crate::tools::ToolOutput;
use crate::workspace::{self, WorktreeError};
use serde_json::{Value, json};
use shiioo_core::{
    CallId, Event, EventStore, Lease, Payload, Recovery, RunId, Seq, StoreError, ToolClass,
    WorkerId, dangling_tool_calls,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

pub type Observer = Arc<dyn Fn(&Event) + Send + Sync>;

/// Executes runs. Stateless between calls to [`Worker::drive`]: everything
/// it needs comes from the run's log, so any worker can pick up any run.
pub struct Worker<S> {
    store: Arc<S>,
    provider: Arc<dyn ModelProvider>,
    machine: Arc<LocalMachine>,
    pub id: WorkerId,
    pub ttl_ms: u64,
    pub faults: Faults,
    observer: Option<Observer>,
    hooks: Option<HookEndpoint>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunOutcome {
    Completed(String),
    Failed(String),
    /// The run can't continue without a decision, for example a
    /// side-effecting tool call whose outcome is unknown.
    NeedsAttention(String),
    /// Another worker took over, or the lease expired. Stop; the run is
    /// safe in the log.
    LostLease,
    /// The owner cancelled the run.
    Cancelled(String),
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Machine(#[from] MachineError),
    #[error(transparent)]
    Harness(#[from] HarnessError),
    #[error(transparent)]
    Worktree(#[from] WorktreeError),
    #[error("run spec: {0}")]
    Spec(String),
    #[error("simulated crash at {0:?}")]
    Crashed(CrashPoint),
}

/// Points in a turn where a test can make the worker die, as with
/// `kill -9`: it stops writing immediately and never releases its lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashPoint {
    AfterModelResponse,
    AfterToolIntent,
    /// The tool ran, but the machine died before journaling the result.
    DuringToolExecution,
    /// The tool ran and the machine journaled it; the worker died before
    /// recording the result in the run's log.
    AfterToolExecuted,
    AfterToolResult,
}

#[derive(Clone, Debug, Default)]
pub struct Faults {
    /// Crash at the first time this point is reached.
    pub crash_at: Option<CrashPoint>,
}

impl<S: EventStore + 'static> Worker<S> {
    pub fn new(
        store: Arc<S>,
        provider: Arc<dyn ModelProvider>,
        machine: Arc<LocalMachine>,
        id: impl Into<WorkerId>,
    ) -> Self {
        Self {
            store,
            provider,
            machine,
            id: id.into(),
            ttl_ms: 30_000,
            faults: Faults::default(),
            observer: None,
            hooks: None,
        }
    }

    /// Routes harness permission requests through Shiioo's API (rules,
    /// then the owner). Without it, the harness's own permission mode
    /// decides.
    pub fn with_hooks(mut self, hooks: HookEndpoint) -> Self {
        self.hooks = Some(hooks);
        self
    }

    pub fn with_faults(mut self, faults: Faults) -> Self {
        self.faults = faults;
        self
    }

    /// Called with every event this worker appends.
    pub fn with_observer(mut self, observer: Observer) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Starts a run with `spec`.
    pub async fn create(&self, spec: &RunSpec) -> Result<RunId, WorkerError> {
        let spec = serde_json::to_value(spec).map_err(|e| WorkerError::Spec(e.to_string()))?;
        Ok(self.store.create_run(spec).await?)
    }

    /// Takes the run's lease and works on it until it finishes, needs
    /// attention, or the lease is lost.
    pub async fn drive(&self, run_id: RunId) -> Result<RunOutcome, WorkerError> {
        let lease = match self
            .store
            .acquire_lease(run_id, self.id.clone(), self.ttl_ms)
            .await
        {
            Ok(lease) => lease,
            Err(StoreError::RunClosed(_)) => return self.closed_outcome(run_id).await,
            Err(e) => return Err(e.into()),
        };
        let log = self.store.read(run_id, Seq(0)).await?;
        if let (Some(observer), Some(ev)) = (&self.observer, log.last()) {
            observer(ev);
        }
        let spec = parse_spec(&log)?;

        let lease = Arc::new(Mutex::new(lease));
        let (_heartbeat, lease_lost) =
            Heartbeat::start(Arc::clone(&self.store), Arc::clone(&lease), self.ttl_ms);
        let mut session = Session {
            worker: self,
            run_id,
            lease,
            lease_lost,
            log,
            spec,
        };
        match session.run().await {
            Err(WorkerError::Store(StoreError::Fenced { .. } | StoreError::LeaseExpired(_)))
            | Err(WorkerError::Machine(MachineError::Fenced { .. })) => Ok(RunOutcome::LostLease),
            other => other,
        }
    }

    async fn closed_outcome(&self, run_id: RunId) -> Result<RunOutcome, WorkerError> {
        let log = self.store.read(run_id, Seq(0)).await?;
        Ok(match log.last().map(|e| &*e.payload) {
            Some(Payload::RunCompleted { output }) => RunOutcome::Completed(as_text(output)),
            Some(Payload::RunFailed { reason }) => RunOutcome::Failed(reason.clone()),
            Some(Payload::RunCancelled { reason }) => RunOutcome::Cancelled(reason.clone()),
            _ => RunOutcome::Failed("run is closed".into()),
        })
    }
}

/// Renews the lease every `ttl / 3` in the background while the worker is
/// busy (model calls can outlast a lease). Stops on drop, and on the first
/// failed renewal: it then flags the lease as lost, so a harness process
/// is killed at once, and the worker's next write fails and it stops.
struct Heartbeat(tokio::task::JoinHandle<()>);

impl Heartbeat {
    fn start<S: EventStore + 'static>(
        store: Arc<S>,
        lease: Arc<Mutex<Lease>>,
        ttl_ms: u64,
    ) -> (Self, watch::Receiver<bool>) {
        let (lost_tx, lost_rx) = watch::channel(false);
        let task = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(ttl_ms / 3)).await;
                let Some(current) = lease.lock().ok().map(|l| l.clone()) else {
                    return;
                };
                match store.renew_lease(&current, ttl_ms).await {
                    Ok(renewed) => {
                        if let Ok(mut l) = lease.lock() {
                            *l = renewed;
                        }
                    }
                    Err(_) => {
                        lost_tx.send_replace(true);
                        return;
                    }
                }
            }
        });
        (Self(task), lost_rx)
    }
}

impl Drop for Heartbeat {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Session<'w, S> {
    worker: &'w Worker<S>,
    run_id: RunId,
    lease: Arc<Mutex<Lease>>,
    lease_lost: watch::Receiver<bool>,
    log: Vec<Event>,
    spec: RunSpec,
}

/// How often a harness session is resumed after its process stopped
/// without finishing, before the run is failed.
const MAX_HARNESS_STARTS: usize = 5;

impl<S: EventStore + 'static> Session<'_, S> {
    async fn run(&mut self) -> Result<RunOutcome, WorkerError> {
        if let Some(outcome) = self.cancel_if_requested().await? {
            return Ok(outcome);
        }
        if let Runtime::Harness(spec) = self.spec.runtime.clone() {
            return self.run_harness(&spec).await;
        }
        if let Some(reason) = unresolved_unknown(&self.log) {
            return Ok(RunOutcome::NeedsAttention(reason));
        }
        if let Some(outcome) = self.recover().await? {
            return Ok(outcome);
        }
        // A previous worker may have died after recording a response but
        // before acting on all of it.
        if let Some(response) = self.latest_response()?
            && let Some(outcome) = self.act_on(response).await?
        {
            return Ok(outcome);
        }
        loop {
            if let Some(outcome) = self.cancel_if_requested().await? {
                return Ok(outcome);
            }
            let turns = self
                .log
                .iter()
                .filter(|e| matches!(*e.payload, Payload::ModelResponseRecorded { .. }))
                .count();
            if turns >= self.spec.agent.max_turns as usize {
                return self
                    .fail(format!("stopped after {turns} turns (max_turns)"))
                    .await;
            }

            let transcript = context::transcript(&self.spec, &self.log);
            let tools = self.worker.machine.toolbox().specs(&self.spec.agent.tools);
            let response = self
                .worker
                .provider
                .complete(ModelRequest {
                    model: &self.spec.agent.model,
                    transcript: &transcript,
                    tools: &tools,
                })
                .await?;
            let recorded = serde_json::to_value(&response)
                .map_err(|e| WorkerError::Spec(format!("encoding model response: {e}")))?;
            self.append(vec![Payload::ModelResponseRecorded {
                model: response.model.clone(),
                response: recorded,
            }])
            .await?;
            self.crash(CrashPoint::AfterModelResponse)?;

            if let Some(outcome) = self.act_on(response).await? {
                return Ok(outcome);
            }
        }
    }

    /// Acts on a recorded model response: stops on a refusal or truncated
    /// response, finishes the run if no tools were requested, and otherwise
    /// runs every requested call that has no intent recorded yet (calls with
    /// an intent were settled by recovery).
    async fn act_on(&mut self, response: ModelResponse) -> Result<Option<RunOutcome>, WorkerError> {
        // Check the stop reason before running any tool in the response.
        match &response.stop_reason {
            StopReason::Refusal => {
                return self
                    .fail("the model declined the request".into())
                    .await
                    .map(Some);
            }
            StopReason::MaxTokens => {
                return self
                    .fail("the response hit max_tokens before finishing".into())
                    .await
                    .map(Some);
            }
            _ => {}
        }
        let calls = response.tool_calls();
        if calls.is_empty() {
            let text = response.text();
            self.append(vec![Payload::RunCompleted {
                output: json!(text),
            }])
            .await?;
            return Ok(Some(RunOutcome::Completed(text)));
        }
        for call in calls {
            if self.intent(&CallId(call.id.clone())).is_err() {
                self.call_tool(call).await?;
            }
        }
        Ok(None)
    }

    /// The most recent model response, decoded.
    fn latest_response(&self) -> Result<Option<ModelResponse>, WorkerError> {
        self.log
            .iter()
            .rev()
            .find_map(|e| match &*e.payload {
                Payload::ModelResponseRecorded { response, .. } => Some(response),
                _ => None,
            })
            .map(|r| {
                serde_json::from_value(r.clone())
                    .map_err(|e| WorkerError::Spec(format!("decoding recorded response: {e}")))
            })
            .transpose()
    }

    /// Drives an external harness for this run. The harness executes its
    /// own tools; Shiioo records what it sees. A session that was started
    /// by a previous worker is resumed rather than started again.
    async fn run_harness(&mut self, spec: &HarnessSpec) -> Result<RunOutcome, WorkerError> {
        let cwd = match &spec.repo {
            Some(repo) => {
                let root = self.worker.machine.worktrees_root().ok_or_else(|| {
                    WorkerError::Spec("this machine has no worktree directory for repo runs".into())
                })?;
                workspace::ensure(root, repo, self.run_id).await?.path
            }
            None => self.worker.machine.workspace().to_path_buf(),
        };
        let starts: Vec<String> = self
            .log
            .iter()
            .filter_map(|e| match &*e.payload {
                Payload::HarnessSessionStarted { session_id, .. } => Some(session_id.clone()),
                _ => None,
            })
            .collect();
        if starts.len() >= MAX_HARNESS_STARTS {
            return self
                .fail(format!(
                    "harness stopped without finishing {} times",
                    starts.len()
                ))
                .await;
        }
        let (prompt, resume) = match starts.last() {
            Some(session) => (harness::CONTINUE_PROMPT, Some(session.as_str())),
            None => (self.spec.task.as_str(), None),
        };
        let mut driver = harness::driver_for(spec, &self.spec.agent, &cwd).await;
        let run_id = self.run_id.to_string();
        let cmd = driver.command(
            spec,
            &self.spec.agent,
            &cwd,
            prompt,
            resume,
            self.worker.hooks.as_ref().map(|h| (h, run_id.as_str())),
        );
        // The time budget counts from the run's first event, so resumes
        // share it. A far-future deadline stands in for "no limit".
        let deadline = match spec.max_minutes {
            Some(minutes) => {
                let started_ms = self.log.first().map_or(0, |e| e.recorded_at.0);
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(started_ms);
                let elapsed = now_ms.saturating_sub(started_ms);
                let left = (minutes * 60_000).saturating_sub(elapsed);
                tokio::time::Instant::now() + Duration::from_millis(left)
            }
            None => tokio::time::Instant::now() + Duration::from_secs(60 * 60 * 24 * 365),
        };
        let mut process = HarnessProcess::spawn(cmd)?;
        let mut lost = self.lease_lost.clone();
        loop {
            let line = tokio::select! {
                line = process.next_line() => line?,
                () = tokio::time::sleep_until(deadline) => {
                    process.kill().await;
                    return self
                        .fail(format!(
                            "time budget of {} minutes exceeded",
                            spec.max_minutes.unwrap_or(0)
                        ))
                        .await;
                }
                _ = lost.changed() => {
                    process.kill().await;
                    return Ok(RunOutcome::LostLease);
                }
            };
            let Some(line) = line else { break };
            let step = driver.interpret(&line)?;
            if !step.payloads.is_empty() {
                self.append(step.payloads).await?;
            }
            if let Some(outcome) = step.outcome {
                return Ok(outcome);
            }
            if cancel_requested(&self.log).is_some() {
                process.kill().await;
                if let Some(outcome) = self.cancel_if_requested().await? {
                    return Ok(outcome);
                }
            }
        }
        // Output ended without a result: the process died or was killed.
        // The run stays open; the next drive resumes the session.
        let (status, stderr) = process.finish().await;
        Err(HarnessError::ExitedEarly {
            status: status.map_or("unknown".into(), |s| s.to_string()),
            stderr: truncate(&stderr, 2000),
        }
        .into())
    }

    async fn call_tool(&mut self, call: ToolCall) -> Result<(), WorkerError> {
        let class = self
            .available(&call.name)
            .map_or(ToolClass::Pure, |tool| tool.class(&call.input));
        let call_id = CallId(call.id);
        self.append(vec![Payload::ToolIntent {
            call_id: call_id.clone(),
            tool: call.name.clone(),
            class,
            input: call.input.clone(),
        }])
        .await?;
        self.crash(CrashPoint::AfterToolIntent)?;

        if self.worker.faults.crash_at == Some(CrashPoint::DuringToolExecution) {
            self.worker.machine.crash_before_next_finish();
        }
        let output = match self
            .dispatch(&call_id, &call.name, &call.input, false)
            .await
        {
            Err(WorkerError::Machine(MachineError::SimulatedCrash)) => {
                return Err(WorkerError::Crashed(CrashPoint::DuringToolExecution));
            }
            other => other?,
        };
        self.crash(CrashPoint::AfterToolExecuted)?;

        self.append(vec![result(call_id, output)]).await?;
        self.crash(CrashPoint::AfterToolResult)
    }

    /// Handles tool calls a previous worker recorded an intent for but no
    /// outcome. The machine's journal decides: if the call finished, its
    /// journaled result is recorded; if it never arrived, it runs now (the
    /// fence keeps the old worker from sending it late); if it started and
    /// never finished, pure and idempotent calls run again and
    /// side-effecting ones become an unknown outcome for a human.
    async fn recover(&mut self) -> Result<Option<RunOutcome>, WorkerError> {
        for rec in dangling_tool_calls(&self.log) {
            let (call_id, class) = match rec {
                Recovery::Rerun { call_id, class } => (call_id, class),
                Recovery::MarkUnknown { call_id } => (call_id, ToolClass::SideEffecting),
            };
            let (tool, input) = self.intent(&call_id)?;
            let epoch = self.epoch()?;
            let status = self.worker.machine.status(self.run_id, epoch, &call_id)?;
            let output = match status {
                CallStatus::Finished(output) => output,
                CallStatus::NotReceived => self.dispatch(&call_id, &tool, &input, false).await?,
                CallStatus::Started if class == ToolClass::SideEffecting => {
                    self.append(vec![Payload::ToolOutcomeUnknown {
                        call_id: call_id.clone(),
                    }])
                    .await?;
                    return Ok(Some(RunOutcome::NeedsAttention(format!(
                        "tool call {} ({tool}) may or may not have run; check its effects, then \
                         record its result",
                        call_id.0
                    ))));
                }
                CallStatus::Started => self.dispatch(&call_id, &tool, &input, true).await?,
            };
            self.append(vec![result(call_id, output)]).await?;
        }
        Ok(None)
    }

    /// Sends a call to the machine. Tools the agent may not use are
    /// answered with an error without reaching the machine.
    async fn dispatch(
        &self,
        call_id: &CallId,
        tool: &str,
        input: &Value,
        rerun: bool,
    ) -> Result<ToolOutput, WorkerError> {
        if self.available(tool).is_none() {
            return Ok(ToolOutput::error(format!(
                "tool '{tool}' is not available to this agent"
            )));
        }
        let inv = Invocation {
            run_id: self.run_id,
            epoch: self.epoch()?,
            call_id,
            tool,
            input,
        };
        let machine = &self.worker.machine;
        Ok(if rerun {
            machine.rerun(inv).await?
        } else {
            machine.execute(inv).await?
        })
    }

    fn available(&self, name: &str) -> Option<&Arc<dyn crate::tools::Tool>> {
        if !self.spec.agent.tools.iter().any(|t| t == name) {
            return None;
        }
        self.worker.machine.toolbox().get(name)
    }

    fn intent(&self, call_id: &CallId) -> Result<(String, Value), WorkerError> {
        self.log
            .iter()
            .find_map(|e| match &*e.payload {
                Payload::ToolIntent {
                    call_id: id,
                    tool,
                    input,
                    ..
                } if id == call_id => Some((tool.clone(), input.clone())),
                _ => None,
            })
            .ok_or_else(|| WorkerError::Spec(format!("no intent for call {}", call_id.0)))
    }

    async fn fail(&mut self, reason: String) -> Result<RunOutcome, WorkerError> {
        self.append(vec![Payload::RunFailed {
            reason: reason.clone(),
        }])
        .await?;
        Ok(RunOutcome::Failed(reason))
    }

    /// Appends under the lease. The owner may have appended external input
    /// (a decision, a cancellation) since this worker last read the log; a
    /// `SeqConflict` then means "re-read and try again", not "stop".
    async fn append(&mut self, payloads: Vec<Payload>) -> Result<(), WorkerError> {
        let lease = self.current_lease()?;
        for _ in 0..4 {
            let next = Seq(self.log.len() as u64);
            match self
                .worker
                .store
                .append(&lease, next, payloads.clone())
                .await
            {
                Ok(events) => {
                    self.absorb(events);
                    return Ok(());
                }
                Err(StoreError::SeqConflict { actual, .. }) if actual > next => {
                    self.catch_up().await?;
                }
                Err(e) => return Err(e.into()),
            }
        }
        Err(WorkerError::Spec(
            "the log kept changing while appending".into(),
        ))
    }

    /// Reads events appended by others since the last read.
    async fn catch_up(&mut self) -> Result<(), WorkerError> {
        let from = Seq(self.log.len() as u64);
        let events = self.worker.store.read(self.run_id, from).await?;
        self.absorb(events);
        Ok(())
    }

    fn absorb(&mut self, events: Vec<Event>) {
        if let Some(observer) = &self.worker.observer {
            events.iter().for_each(|e| observer(e));
        }
        self.log.extend(events);
    }

    /// If the owner asked for cancellation, closes the run with
    /// `RunCancelled` and reports it.
    async fn cancel_if_requested(&mut self) -> Result<Option<RunOutcome>, WorkerError> {
        let Some(reason) = cancel_requested(&self.log) else {
            return Ok(None);
        };
        self.append(vec![Payload::RunCancelled {
            reason: reason.clone(),
        }])
        .await?;
        Ok(Some(RunOutcome::Cancelled(reason)))
    }

    fn crash(&self, point: CrashPoint) -> Result<(), WorkerError> {
        if self.worker.faults.crash_at == Some(point) {
            return Err(WorkerError::Crashed(point));
        }
        Ok(())
    }

    fn current_lease(&self) -> Result<Lease, WorkerError> {
        self.lease
            .lock()
            .map(|l| l.clone())
            .map_err(|_| WorkerError::Spec("lease mutex poisoned".into()))
    }

    fn epoch(&self) -> Result<shiioo_core::Epoch, WorkerError> {
        Ok(self.current_lease()?.epoch)
    }
}

fn truncate(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

/// The reason of a `CancelRequested` that no `RunCancelled` has answered.
fn cancel_requested(log: &[Event]) -> Option<String> {
    if log.last().is_some_and(|e| e.payload.is_terminal()) {
        return None;
    }
    log.iter().rev().find_map(|e| match &*e.payload {
        Payload::CancelRequested { by, reason } => Some(if reason.is_empty() {
            format!("cancelled by {by}")
        } else {
            format!("cancelled by {by}: {reason}")
        }),
        _ => None,
    })
}

fn result(call_id: CallId, output: ToolOutput) -> Payload {
    Payload::ToolResult {
        call_id,
        output: json!(output.content),
        is_error: output.is_error,
    }
}

fn parse_spec(log: &[Event]) -> Result<RunSpec, WorkerError> {
    match log.first().map(|e| &*e.payload) {
        Some(Payload::RunCreated { spec }) => {
            serde_json::from_value(spec.clone()).map_err(|e| WorkerError::Spec(e.to_string()))
        }
        _ => Err(WorkerError::Spec(
            "log does not start with RunCreated".into(),
        )),
    }
}
