use crate::context;
use crate::harness::claude_code::HookEndpoint;
use crate::harness::{self, HarnessDriver, HarnessError, HarnessProcess};
use crate::machine::{CallStatus, Invocation, LocalMachine, MachineError};
use crate::model::{
    ModelProvider, ModelRequest, ModelResponse, ProviderError, StopReason, ToolCall,
};
use crate::redact::Redactor;
use crate::sandbox::{Sandbox, SandboxContext};
use crate::secrets::{self, NoSecrets, SecretSource};
use crate::spec::{HarnessSpec, RunSpec, Runtime};
use crate::status::{as_text, parked, unresolved_unknown, wake_reason};
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
    secrets: Arc<dyn SecretSource>,
    /// This machine's repositories by label, for runs whose spec names a
    /// repository of another machine (a node working for the office).
    repos: std::collections::BTreeMap<String, std::path::PathBuf>,
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
    /// The run is parked, waiting for this (`input`), and holds no lease.
    Parked(String),
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
            secrets: Arc::new(NoSecrets),
            repos: std::collections::BTreeMap::new(),
        }
    }

    /// The repositories on this machine by label. A harness run whose spec
    /// names a label resolves it here; a path from another machine that
    /// does not exist here fails the run with the label.
    pub fn with_repos(
        mut self,
        repos: std::collections::BTreeMap<String, std::path::PathBuf>,
    ) -> Self {
        self.repos = repos;
        self
    }

    /// Where `secret:<name>` references in a harness's environment are
    /// resolved, and whose values are kept out of the log.
    pub fn with_secrets(mut self, secrets: Arc<dyn SecretSource>) -> Self {
        self.secrets = secrets;
        self
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
        // A parked run with nothing new is left alone, without taking (and
        // then holding) its lease.
        let before = self.store.read(run_id, Seq(0)).await?;
        if let Some((_, waiting_for)) = parked(&before)
            && wake_reason(&before).is_none()
        {
            return Ok(RunOutcome::Parked(waiting_for.to_owned()));
        }
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

/// How often a harness run looks for a cancellation while its process
/// prints nothing (a long build, a hung command).
const CANCEL_POLL: Duration = Duration::from_secs(2);

/// How long an interrupted harness gets to close its turn and save its
/// session before it is killed.
const INTERRUPT_GRACE: Duration = Duration::from_secs(10);

impl<S: EventStore + 'static> Session<'_, S> {
    async fn run(&mut self) -> Result<RunOutcome, WorkerError> {
        if let Some(outcome) = self.cancel_if_requested().await? {
            return Ok(outcome);
        }
        if parked(&self.log).is_some() {
            match wake_reason(&self.log) {
                Some(reason) => {
                    self.append(vec![Payload::RunWoken {
                        reason: reason.into(),
                    }])
                    .await?;
                }
                // Parked again between the check in `drive` and the lease.
                None => {
                    return Ok(RunOutcome::Parked(
                        parked(&self.log).map_or("input", |(_, w)| w).to_owned(),
                    ));
                }
            }
        }
        if let Runtime::Harness(spec) = self.spec.runtime.clone() {
            // A message the harness has not seen when it finishes is
            // delivered by resuming its session: one more process.
            loop {
                if let Some(outcome) = self.run_harness(&spec).await? {
                    return Ok(outcome);
                }
            }
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
            let context_seq = Seq(self.log.len() as u64);
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
                context_seq: Some(context_seq),
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
            let (last, outcome) = if self.spec.park_on_answer {
                (
                    Payload::RunParked {
                        waiting_for: "input".into(),
                        wake_at: None,
                    },
                    RunOutcome::Parked("input".into()),
                )
            } else {
                (
                    Payload::RunCompleted {
                        output: json!(text),
                    },
                    RunOutcome::Completed(text),
                )
            };
            // A message that arrived during the turn is answered first.
            return Ok(self.settle(last, native_seen).await?.then_some(outcome));
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
    async fn run_harness(&mut self, spec: &HarnessSpec) -> Result<Option<RunOutcome>, WorkerError> {
        // The repository as this machine has it: by label when the spec
        // names one this machine knows, else by the spec's path.
        let repo = match (&spec.repo_label, &spec.repo) {
            (Some(label), _) if self.worker.repos.contains_key(label) => {
                self.worker.repos.get(label).cloned()
            }
            (_, Some(path)) if path.join(".git").exists() => Some(path.clone()),
            (label, Some(path)) => {
                return self
                    .fail(format!(
                        "repository {} is not available on this machine ({} does not exist; \
                         add it to [repos] here)",
                        label.as_deref().unwrap_or("?"),
                        path.display()
                    ))
                    .await
                    .map(Some);
            }
            (_, None) => None,
        };
        let spec = &HarnessSpec {
            repo: repo.clone(),
            ..spec.clone()
        };
        // A follow-up works in the worktree, branch and sandbox state of
        // the run it continues.
        let workspace_run = self.spec.workspace_run(self.run_id);
        let cwd = match &repo {
            Some(repo) => {
                let root = self.worker.machine.worktrees_root().ok_or_else(|| {
                    WorkerError::Spec("this machine has no worktree directory for repo runs".into())
                })?;
                workspace::ensure(root, repo, workspace_run).await?.path
            }
            None => self.worker.machine.workspace().to_path_buf(),
        };
        let own_session = self.log.iter().rev().find_map(|e| match &*e.payload {
            Payload::HarnessSessionStarted { session_id, .. } => Some(session_id.clone()),
            _ => None,
        });
        // A follow-up's first process resumes the session it continues,
        // with the follow-up's task as the prompt.
        let resumed = own_session.is_some();
        let session =
            own_session.or_else(|| self.spec.origin.as_ref().and_then(|o| o.session.clone()));
        // Starts since the session last finished cleanly: resumes after a
        // crash, not deliveries of the owner's messages.
        let unfinished = unfinished_starts(&self.log);
        if unfinished >= MAX_HARNESS_STARTS {
            return self
                .fail(format!(
                    "harness stopped without finishing {unfinished} times"
                ))
                .await
                .map(Some);
        }
        // The harness's environment: secrets are resolved here and only
        // here; a reference nobody can resolve is the owner's to fix.
        let env = match secrets::resolve_env(&spec.env, &*self.worker.secrets) {
            Ok(env) => env,
            Err(reason) => return self.fail(reason).await.map(Some),
        };
        let ctx = SandboxContext {
            run_id: workspace_run,
            harness: spec.harness,
            cwd: &cwd,
            repo: spec.repo.as_deref(),
            sandboxes_root: self.worker.machine.sandboxes_root(),
            hooks: self.worker.hooks.as_ref(),
            env: &env,
            secrets: &*self.worker.secrets,
            binary_override: spec.binary.as_deref(),
        };
        let sandbox = match Sandbox::prepare(&spec.isolation, &ctx).await {
            Ok(sandbox) => sandbox,
            Err(e) => {
                return self
                    .fail(format!("{} isolation: {e}", spec.isolation.level()))
                    .await
                    .map(Some);
            }
        };
        // Where the harness runs is on the record before it starts. This
        // event also marks what the process is told: messages before it
        // go into its prompt, messages after it are unread.
        self.append(vec![sandbox.started()]).await?;
        let prompt = harness_prompt(&self.log, &self.spec.task, resumed);
        let mut driver = harness::driver_for(spec, &self.spec.agent, &cwd).await;
        let run_id = self.run_id.to_string();
        let hooks = sandbox.hooks();
        let cmd = driver.command(
            spec,
            &self.spec.agent,
            &cwd,
            &prompt,
            session.as_deref(),
            hooks.as_ref().map(|h| (h, run_id.as_str())),
        );
        let cmd = sandbox.wrap(cmd);
        // Everything the harness could print that must not be recorded:
        // the office's secrets and the API token it was handed.
        let mut redactable = self.worker.secrets.all();
        if let Some(h) = &hooks {
            redactable.push(("shiioo-api-token".into(), h.token.clone()));
        }
        let redactor = Redactor::new(redactable);
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
        let result = self
            .drive_harness(cmd, &mut *driver, &redactor, deadline, spec)
            .await;
        sandbox.teardown().await;
        result
    }

    /// Runs the harness process and records its output until it ends.
    async fn drive_harness(
        &mut self,
        cmd: tokio::process::Command,
        driver: &mut dyn HarnessDriver,
        redactor: &Redactor,
        deadline: tokio::time::Instant,
        spec: &HarnessSpec,
    ) -> Result<Option<RunOutcome>, WorkerError> {
        let mut process = HarnessProcess::spawn(cmd)?;
        let mut lost = self.lease_lost.clone();
        let mut poll = tokio::time::interval(CANCEL_POLL);
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        poll.tick().await;
        loop {
            let line = tokio::select! {
                line = process.next_line() => line?,
                _ = poll.tick() => {
                    // Output also checks for a cancellation or an
                    // interrupt; a silent process would otherwise never
                    // see one.
                    self.catch_up().await?;
                    if cancel_requested(&self.log).is_some() {
                        process.kill().await;
                        if let Some(outcome) = self.cancel_if_requested().await? {
                            return Ok(Some(outcome));
                        }
                    }
                    if interrupt_requested(&self.log) {
                        process.interrupt(INTERRUPT_GRACE).await;
                        return Ok(None);
                    }
                    continue;
                }
                () = tokio::time::sleep_until(deadline) => {
                    process.kill().await;
                    return self
                        .fail(format!(
                            "time budget of {} minutes exceeded",
                            spec.max_minutes.unwrap_or(0)
                        ))
                        .await
                        .map(Some);
                }
                _ = lost.changed() => {
                    process.kill().await;
                    return Ok(Some(RunOutcome::LostLease));
                }
            };
            let Some(line) = line else { break };
            let step = driver.interpret(&line)?;
            let mut payloads = redactor.redact_payloads(step.payloads);
            // The harness finished its task: complete (or park), unless
            // the owner wrote since this process started.
            if let Some(RunOutcome::Completed(text)) = &step.outcome
                && let Some(completed) =
                    payloads.pop_if(|p| matches!(p, Payload::RunCompleted { .. }))
            {
                if !payloads.is_empty() {
                    self.append(payloads).await?;
                }
                let (last, outcome) = if self.spec.park_on_answer {
                    (
                        Payload::RunParked {
                            waiting_for: "input".into(),
                            wake_at: None,
                        },
                        RunOutcome::Parked("input".into()),
                    )
                } else {
                    (completed, RunOutcome::Completed(text.clone()))
                };
                if self.settle(last, harness_seen).await? {
                    return Ok(Some(redact_outcome(redactor, outcome)));
                }
                // Let the process exit on its own, so the session is saved
                // before it is resumed with the message.
                process.finish().await;
                return Ok(None);
            }
            if !payloads.is_empty() {
                self.append(payloads).await?;
            }
            if let Some(outcome) = step.outcome {
                // The outcome is printed and sent to the phone; it must
                // say what the record says.
                return Ok(Some(redact_outcome(redactor, outcome)));
            }
            if cancel_requested(&self.log).is_some() {
                process.kill().await;
                if let Some(outcome) = self.cancel_if_requested().await? {
                    return Ok(Some(outcome));
                }
            }
            // The owner asked to be heard now: stop the process and
            // resume the session with their message (`harness_prompt`).
            if interrupt_requested(&self.log) {
                process.interrupt(INTERRUPT_GRACE).await;
                return Ok(None);
            }
        }
        // Output ended without a result: the process died or was killed.
        // The run stays open; the next drive resumes the session.
        let (status, stderr) = process.finish().await;
        let stderr = redactor.redact_str(&stderr, &mut std::collections::BTreeMap::new());
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

    /// Appends the run's last word for now (`RunCompleted` or `RunParked`)
    /// unless the owner wrote since the latest model response: then
    /// nothing is appended and `false` says the model must answer first.
    /// The append names the seq it expects, so a message can't slip in
    /// between the check and the write.
    async fn settle(
        &mut self,
        last: Payload,
        seen: fn(&[Event]) -> usize,
    ) -> Result<bool, WorkerError> {
        let lease = self.current_lease()?;
        for _ in 0..4 {
            self.catch_up().await?;
            if unread_input(&self.log, seen) {
                return Ok(false);
            }
            let next = Seq(self.log.len() as u64);
            match self
                .worker
                .store
                .append(&lease, next, vec![last.clone()])
                .await
            {
                Ok(events) => {
                    self.absorb(events);
                    return Ok(true);
                }
                Err(StoreError::SeqConflict { actual, .. }) if actual > next => {}
                Err(e) => return Err(e.into()),
            }
        }
        Err(WorkerError::Spec(
            "the log kept changing while finishing".into(),
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

fn redact_outcome(redactor: &Redactor, outcome: RunOutcome) -> RunOutcome {
    let mut counts = std::collections::BTreeMap::new();
    match outcome {
        RunOutcome::Completed(t) => RunOutcome::Completed(redactor.redact_str(&t, &mut counts)),
        RunOutcome::Failed(t) => RunOutcome::Failed(redactor.redact_str(&t, &mut counts)),
        RunOutcome::NeedsAttention(t) => {
            RunOutcome::NeedsAttention(redactor.redact_str(&t, &mut counts))
        }
        RunOutcome::Cancelled(t) => RunOutcome::Cancelled(redactor.redact_str(&t, &mut counts)),
        RunOutcome::LostLease => RunOutcome::LostLease,
        RunOutcome::Parked(w) => RunOutcome::Parked(w),
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

/// An `InterruptRequested` since the current harness process started:
/// one from before it was for an earlier process (or none at all).
fn interrupt_requested(log: &[Event]) -> bool {
    log.iter()
        .skip(harness_seen(log))
        .any(|e| matches!(*e.payload, Payload::InterruptRequested { .. }))
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

/// A message from the owner at or after `seen(log)`: the agent has not
/// seen it yet.
fn unread_input(log: &[Event], seen: fn(&[Event]) -> usize) -> bool {
    log.iter()
        .skip(seen(log))
        .any(|e| matches!(*e.payload, Payload::UserMessage { .. }))
}

/// Native loop: what the latest model request was built from.
fn native_seen(log: &[Event]) -> usize {
    log.iter()
        .enumerate()
        .rev()
        .find_map(|(i, e)| match &*e.payload {
            Payload::ModelResponseRecorded { context_seq, .. } => {
                Some(context_seq.map_or(i, |s| s.0 as usize))
            }
            _ => None,
        })
        .unwrap_or(0)
}

/// Harness: the latest process start. Messages before it were in its
/// prompt (`harness_prompt`).
fn harness_seen(log: &[Event]) -> usize {
    log.iter()
        .rposition(|e| matches!(*e.payload, Payload::SandboxStarted { .. }))
        .map_or(0, |i| i + 1)
}

/// What a harness process is told, given the log up to and including the
/// `SandboxStarted` written for it: the task, the owner's messages since
/// the previous process started, or, after a crash, a request to continue.
fn harness_prompt(log: &[Event], task: &str, resumed: bool) -> String {
    let starts: Vec<usize> = log
        .iter()
        .enumerate()
        .filter(|(_, e)| matches!(*e.payload, Payload::SandboxStarted { .. }))
        .map(|(i, _)| i)
        .collect();
    // Messages after the last process that got as far as starting its
    // session; one that failed before that (a bad flag, a missing binary)
    // was told nothing.
    let ran = |i: usize| {
        let next = starts
            .iter()
            .find(|&&s| s > i)
            .copied()
            .unwrap_or(log.len());
        log[i..next]
            .iter()
            .any(|e| matches!(*e.payload, Payload::HarnessSessionStarted { .. }))
    };
    let (from, to) = match starts.split_last() {
        Some((current, earlier)) => (
            earlier.iter().rev().copied().find(|&i| ran(i)).unwrap_or(0),
            *current,
        ),
        None => (0, log.len()),
    };
    let messages: Vec<String> = log[from..to]
        .iter()
        .filter_map(|e| match &*e.payload {
            Payload::UserMessage { author, content } if author == "owner" => Some(content.clone()),
            Payload::UserMessage { author, content } => Some(format!("{author}: {content}")),
            _ => None,
        })
        .collect();
    let messages = messages.join("\n\n");
    let interrupted = log[from..to]
        .iter()
        .any(|e| matches!(*e.payload, Payload::InterruptRequested { .. }));
    let finished = log[..to]
        .iter()
        .rev()
        .take_while(|e| !matches!(*e.payload, Payload::HarnessSessionStarted { .. }))
        .any(|e| matches!(*e.payload, Payload::HarnessResult { .. }));
    match (resumed, finished, messages.is_empty()) {
        (false, _, true) => task.to_owned(),
        (false, _, false) => format!("{task}\n\n{messages}"),
        (true, true, false) => messages,
        (true, _, true) => harness::CONTINUE_PROMPT.to_owned(),
        (true, false, false) if interrupted => {
            format!("{}\n\n{messages}", harness::INTERRUPTED_PROMPT)
        }
        (true, false, false) => format!(
            "{}\n\nWhile you were stopped, the owner wrote:\n\n{messages}",
            harness::CONTINUE_PROMPT
        ),
    }
}

/// Harness sessions started since the session last finished cleanly or
/// was stopped on the owner's request.
fn unfinished_starts(log: &[Event]) -> usize {
    log.iter()
        .rev()
        .take_while(|e| {
            !matches!(
                *e.payload,
                Payload::HarnessResult { .. } | Payload::InterruptRequested { .. }
            )
        })
        .filter(|e| matches!(*e.payload, Payload::HarnessSessionStarted { .. }))
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiioo_core::{Epoch, RunId, Timestamp};

    fn log(payloads: Vec<Payload>) -> Vec<Event> {
        let run = RunId::new();
        let mut out: Vec<Event> = Vec::new();
        for (i, p) in payloads.into_iter().enumerate() {
            let prev = out.last().map(|e| e.hash);
            match Event::seal(run, Seq(i as u64), Epoch(1), Timestamp(i as u64), prev, p) {
                Ok(ev) => out.push(ev),
                Err(e) => panic!("{e}"),
            }
        }
        out
    }

    fn start() -> Payload {
        Payload::SandboxStarted {
            level: "worktree".into(),
            detail: json!({}),
        }
    }

    fn session() -> Payload {
        Payload::HarnessSessionStarted {
            harness: "claude-code".into(),
            version: String::new(),
            session_id: "s".into(),
            cwd: String::new(),
            model: String::new(),
        }
    }

    fn result() -> Payload {
        Payload::HarnessResult {
            harness: "claude-code".into(),
            session_id: "s".into(),
            subtype: "success".into(),
            is_error: false,
            cost_usd: None,
            usage: json!({}),
            permission_denials: json!([]),
        }
    }

    fn say(text: &str) -> Payload {
        Payload::UserMessage {
            author: "owner".into(),
            content: text.into(),
        }
    }

    fn created() -> Payload {
        Payload::RunCreated { spec: json!({}) }
    }

    #[test]
    fn the_first_process_gets_the_task_and_earlier_messages() {
        let l = log(vec![created(), say("In sango."), start()]);
        assert_eq!(harness_prompt(&l, "Fix it.", false), "Fix it.\n\nIn sango.");
        let l = log(vec![created(), start()]);
        assert_eq!(harness_prompt(&l, "Fix it.", false), "Fix it.");
    }

    #[test]
    fn after_a_clean_finish_the_prompt_is_the_new_messages() {
        let l = log(vec![
            created(),
            start(),
            session(),
            say("One."),
            result(),
            say("Two."),
            start(),
        ]);
        assert_eq!(harness_prompt(&l, "Fix it.", true), "One.\n\nTwo.");
    }

    #[test]
    fn after_a_crash_the_harness_continues_and_hears_the_messages() {
        let l = log(vec![created(), start(), session(), start()]);
        assert_eq!(
            harness_prompt(&l, "Fix it.", true),
            harness::CONTINUE_PROMPT
        );
        let l = log(vec![created(), start(), session(), say("Hurry."), start()]);
        let prompt = harness_prompt(&l, "Fix it.", true);
        assert!(prompt.starts_with(harness::CONTINUE_PROMPT), "{prompt}");
        assert!(prompt.ends_with("the owner wrote:\n\nHurry."), "{prompt}");
    }

    #[test]
    fn a_process_that_never_started_was_told_nothing() {
        // The message went into the prompt of a process that failed before
        // starting its session; the next one must carry it.
        let l = log(vec![
            created(),
            start(),
            session(),
            say("Also the README."),
            result(),
            start(),
            start(),
        ]);
        assert_eq!(harness_prompt(&l, "Fix it.", true), "Also the README.");
        let l = log(vec![created(), say("In sango."), start(), start()]);
        assert_eq!(harness_prompt(&l, "Fix it.", false), "Fix it.\n\nIn sango.");
    }

    fn interrupt() -> Payload {
        Payload::InterruptRequested { by: "owner".into() }
    }

    #[test]
    fn an_interrupted_harness_hears_why_it_was_stopped() {
        let l = log(vec![
            created(),
            start(),
            session(),
            say("Use the backoff helper."),
            interrupt(),
            start(),
        ]);
        assert_eq!(
            harness_prompt(&l, "Fix it.", true),
            format!("{}\n\nUse the backoff helper.", harness::INTERRUPTED_PROMPT)
        );
        // The process finished before the interrupt reached it: the
        // message is an ordinary next prompt.
        let l = log(vec![
            created(),
            start(),
            session(),
            say("Use the backoff helper."),
            interrupt(),
            result(),
            start(),
        ]);
        assert_eq!(
            harness_prompt(&l, "Fix it.", true),
            "Use the backoff helper."
        );
    }

    #[test]
    fn an_interrupt_is_for_the_process_running_when_it_came() {
        let l = log(vec![
            created(),
            start(),
            session(),
            say("Now."),
            interrupt(),
        ]);
        assert!(interrupt_requested(&l));
        let l = log(vec![
            created(),
            start(),
            session(),
            say("Now."),
            interrupt(),
            start(),
        ]);
        assert!(!interrupt_requested(&l));
    }

    #[test]
    fn interrupts_do_not_count_against_the_start_limit() {
        let l = log(vec![
            session(),
            interrupt(),
            session(),
            interrupt(),
            session(),
        ]);
        assert_eq!(unfinished_starts(&l), 1);
    }

    #[test]
    fn only_crashes_count_against_the_start_limit() {
        let l = log(vec![session(), result(), session(), result(), session()]);
        assert_eq!(unfinished_starts(&l), 1);
        let l = log(vec![session(), session(), session()]);
        assert_eq!(unfinished_starts(&l), 3);
    }

    #[test]
    fn a_harness_has_seen_what_came_before_its_start() {
        let l = log(vec![created(), say("Before."), start(), session()]);
        assert!(!unread_input(&l, harness_seen));
        let l = log(vec![created(), start(), session(), say("During.")]);
        assert!(unread_input(&l, harness_seen));
    }
}
