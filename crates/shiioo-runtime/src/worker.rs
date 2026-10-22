use crate::context;
use crate::machine::{CallStatus, Invocation, LocalMachine, MachineError};
use crate::model::{
    ModelProvider, ModelRequest, ModelResponse, ProviderError, StopReason, ToolCall,
};
use crate::spec::RunSpec;
use crate::status::{as_text, unresolved_unknown};
use crate::tools::ToolOutput;
use serde_json::{Value, json};
use shiioo_core::{
    CallId, Event, EventStore, Lease, Payload, Recovery, RunId, Seq, StoreError, ToolClass,
    WorkerId, dangling_tool_calls,
};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error(transparent)]
    Provider(#[from] ProviderError),
    #[error(transparent)]
    Machine(#[from] MachineError),
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
        }
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
        let _heartbeat = Heartbeat::start(Arc::clone(&self.store), Arc::clone(&lease), self.ttl_ms);
        let mut session = Session {
            worker: self,
            run_id,
            lease,
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
            _ => RunOutcome::Failed("run is closed".into()),
        })
    }
}

/// Renews the lease every `ttl / 3` in the background while the worker is
/// busy (model calls can outlast a lease). Stops on drop, and on the first
/// failed renewal: the worker's next write then fails and it stops.
struct Heartbeat(tokio::task::JoinHandle<()>);

impl Heartbeat {
    fn start<S: EventStore + 'static>(
        store: Arc<S>,
        lease: Arc<Mutex<Lease>>,
        ttl_ms: u64,
    ) -> Self {
        Self(tokio::spawn(async move {
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
                    Err(_) => return,
                }
            }
        }))
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
    log: Vec<Event>,
    spec: RunSpec,
}

impl<S: EventStore + 'static> Session<'_, S> {
    async fn run(&mut self) -> Result<RunOutcome, WorkerError> {
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

    async fn append(&mut self, payloads: Vec<Payload>) -> Result<(), WorkerError> {
        let lease = self.current_lease()?;
        let next = Seq(self.log.len() as u64);
        let events = self.worker.store.append(&lease, next, payloads).await?;
        if let Some(observer) = &self.worker.observer {
            events.iter().for_each(|e| observer(e));
        }
        self.log.extend(events);
        Ok(())
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
