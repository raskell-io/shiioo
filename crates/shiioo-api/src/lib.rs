//! Shiioo's HTTP API (`docs/design/api.md`, ADR-0004) and the service layer
//! behind it, which the Telegram bot uses too.
//!
//! - `POST /v1/runs` starts a run and a worker for it; `GET /v1/runs` lists runs.
//! - `GET  /v1/runs/{id}` folds the log into a status.
//! - `GET  /v1/runs/{id}/events?from=N` returns events from `seq` N.
//! - `GET  /v1/runs/{id}/events/stream` streams events as SSE. Each SSE id
//!   is the event's `seq`; reconnecting with `Last-Event-ID` resumes right
//!   after it, served from the log, so a client never misses or repeats an
//!   event. The stream ends after the run's terminal event.
//! - `POST /v1/runs/{id}/resume` starts a worker for an open run again.
//! - `POST /v1/runs/{id}/cancel` asks the run to stop.
//! - `POST /v1/runs/{id}/permissions` is called by a harness hook before a
//!   tool call: the rules answer at once, or the owner is asked and the
//!   request waits for `POST /v1/approvals/{id}`.
//! - `POST /v1/runs/{id}/hook/pre-tool-use` is the same, taking the hook's
//!   stdin JSON and answering in the hook's own output format, for sandboxes
//!   whose hook is a `curl` call.
//!
//! - `GET  /v1/runs/{id}/diff` is the run's branch diff against the repo's HEAD.
//! - `GET  /v1/report` is the morning report, on demand.
//! - `GET  /v1/roles`, `GET /v1/routines`, `POST /v1/routines/{name}/run`: the
//!   configured roles and routines, and firing a routine now.
//! - `POST /v1/plans` starts the chief of staff on a goal; `GET /v1/plans`,
//!   `GET /v1/plans/{id}` and `POST /v1/plans/{id}/approve` read and approve
//!   plans (approval creates the issues the intake works on).
//! - Machines (nodes working for the office): `POST /v1/machines/enrollments`
//!   makes a one-time token; `POST /v1/machines/enroll` (the token is the
//!   credential) exchanges it for a machine token; `GET /v1/machines`,
//!   `DELETE /v1/machines/{name}`; nodes `POST …/{name}/heartbeat` and long-poll
//!   `GET …/{name}/work`. Node workers use the office's log over HTTP:
//!   `POST /v1/runs/{id}/lease`, `POST /v1/runs/{id}/lease/renew`,
//!   `POST /v1/runs/{id}/events`, with store errors in a JSON form that keeps
//!   fencing intact.
//! - `GET  /v1/runs/{id}/artifacts` lists the files a run produced;
//!   `GET /v1/runs/{id}/artifacts/{path}` downloads one.
//! - `POST /v1/mail/send` sends a mail for a run (through the rules and the
//!   owner's approval) or for the owner.
//! - `GET  /` serves the mobile dashboard (`ui/`), which calls the API with the token.
//!
//! Everything except `GET /v1/health` and the dashboard files needs
//! `Authorization: Bearer <token>`.

pub mod artifacts;
pub mod employees;
pub mod isolation;
pub mod machines;
pub mod mail;
pub mod plans;
pub mod report;
pub mod roles;
pub mod routines;
pub mod schedule;
pub mod takeover;

pub use artifacts::Artifact;
use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::Stream;
pub use isolation::IsolationPolicy;
pub use machines::{Machine, Machines};
pub use mail::{MailOutcome, MailRequest, MailSender};
pub use plans::{IssueCreator, Plan, PlanItem, PlanStatus, Plans};
pub use roles::Role;
pub use routines::{Routine, RoutineState};
pub use schedule::Schedule;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shiioo_core::{Event, EventStore, Lease, Payload, RunId, Seq, StoreError, Timestamp, WorkerId};
use shiioo_rules::{Decision, PermissionRequest, RuleSet};
use shiioo_runtime::harness::claude_code::HOOK_TIMEOUT_SECS;
use shiioo_runtime::{
    AgentSpec, HarnessSpec, Origin, RunOutcome, RunSpec, RunStatus, Runtime, Worker, branch_for,
    open_approval, run_status, wake_reason,
};
use shiioo_store::Notifying;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;

pub use employees::{Employee, EmployeeKind, Pick, Skipped, default_employees, pick};

pub type WorkerFactory<S> = Arc<dyn Fn() -> Worker<Notifying<S>> + Send + Sync>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// How long a permission request waits for the owner. A little under the
/// harness's own hook timeout, so Shiioo records the outcome first.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(HOOK_TIMEOUT_SECS - 120);

/// Where the office tells the owner about things. Implemented by the
/// Telegram layer; absent in tests and in the bare API.
pub trait Notifications: Send + Sync {
    fn approval_requested(
        &self,
        run: RunId,
        approval_id: String,
        summary: String,
    ) -> BoxFuture<'_, ()>;
    /// `outcome` is a short phrase: "allowed", "denied", "expired".
    fn approval_resolved(&self, approval_id: String, outcome: String) -> BoxFuture<'_, ()>;
    /// `media` is the files the run produced that are worth sending along
    /// (images, audio, video, PDFs under the size limit), with their bytes
    /// available through [`ApiState::artifact_bytes`].
    fn run_ended(
        &self,
        run: RunId,
        task: String,
        status: RunStatus,
        media: Vec<Artifact>,
    ) -> BoxFuture<'_, ()>;
    /// Anything else the office says on its own: a routine that started, a
    /// finished plan, the morning report.
    fn message(&self, text: String) -> BoxFuture<'_, ()>;
}

pub struct ApiOptions {
    pub rules: RuleSet,
    /// Who takes new tasks, in order of preference ([`employees`]).
    pub employees: Vec<Employee>,
    /// Latest rate-limit windows per provider, for routing and for the
    /// owner's "how much staff do I have left this week" question.
    pub capacity: Arc<CapacityTracker>,
    /// Repository label → path, for `/run <label> …` and for repo-scoped
    /// rules.
    pub repos: BTreeMap<String, PathBuf>,
    pub notifications: Option<Arc<dyn Notifications>>,
    /// Which isolation level new harness runs get.
    pub isolation: IsolationPolicy,
    /// Environment every harness run gets; values are literal or
    /// `secret:<name>`.
    pub env: BTreeMap<String, String>,
    /// Reusable agent configurations (`[[roles]]`).
    pub roles: Vec<Role>,
    /// Scheduled runs (`[[routines]]`).
    pub routines: Vec<Routine>,
    /// When the morning report goes out (`[report] schedule`); None = never
    /// on its own (`/report` still works).
    pub report: Option<String>,
    /// Where plans and the routine state are kept (`.shiioo/`). None keeps
    /// them in memory only.
    pub data_dir: Option<PathBuf>,
    /// Creates the issues of an approved plan. None disables approving.
    pub issues: Option<Arc<dyn IssueCreator>>,
    /// The label the intake watches, put on issues created from plans.
    pub intake_label: String,
    /// Where this machine keeps run worktrees (`.shiioo/worktrees`), for
    /// listing a run's files.
    pub worktrees: Option<PathBuf>,
    /// Sends mail for `POST /v1/mail/send`. None: the office has no mail.
    pub mail: Option<Arc<dyn MailSender>>,
    /// Where the harnesses keep their session files, for importing what
    /// happened during a takeover. None = `HOME` and the harnesses' own
    /// environment variables.
    pub harness_homes: Option<shiioo_runtime::harness::transcript::Homes>,
    /// How long a takeover nobody is attached to keeps its terminal (D6).
    pub takeover_detach: std::time::Duration,
}

impl Default for ApiOptions {
    fn default() -> Self {
        Self {
            rules: RuleSet::defaults(),
            employees: default_employees(),
            capacity: Arc::new(CapacityTracker::default()),
            repos: BTreeMap::new(),
            notifications: None,
            isolation: IsolationPolicy::default(),
            env: BTreeMap::new(),
            roles: Vec::new(),
            routines: Vec::new(),
            report: None,
            data_dir: None,
            issues: None,
            intake_label: "shiioo".into(),
            worktrees: None,
            mail: None,
            harness_homes: None,
            takeover_detach: std::time::Duration::from_secs(30 * 60),
        }
    }
}

/// One rate-limit window as a provider reports it (Claude Code: `five_hour`
/// and `seven_day`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CapacityWindow {
    /// 0.0 – 1.0 of the window used.
    pub utilization: f64,
    /// Unix seconds when the window resets.
    pub resets_at: Option<u64>,
    pub status: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProviderCapacity {
    pub provider: String,
    /// When this was observed, ms since the epoch.
    pub observed_at: u64,
    pub windows: BTreeMap<String, CapacityWindow>,
}

/// The latest `CapacityObserved` per provider. Fed by the workers'
/// observer as runs report windows, seeded from the logs at startup.
#[derive(Default)]
pub struct CapacityTracker {
    latest: Mutex<HashMap<String, ProviderCapacity>>,
}

impl CapacityTracker {
    /// Records a provider's windows if newer than what is known.
    pub fn observe(&self, provider: &str, windows: &Value, observed_at: u64) {
        let parsed: BTreeMap<String, CapacityWindow> = windows
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(name, w)| {
                Some((
                    name.clone(),
                    CapacityWindow {
                        utilization: w["utilization"].as_f64()?,
                        resets_at: w["resetsAt"].as_u64().or_else(|| w["resets_at"].as_u64()),
                        status: w["status"].as_str().map(str::to_owned),
                    },
                ))
            })
            .collect();
        if parsed.is_empty() {
            return;
        }
        let mut latest = lock(&self.latest);
        let newer = latest
            .get(provider)
            .is_none_or(|known| observed_at >= known.observed_at);
        if newer {
            latest.insert(
                provider.to_owned(),
                ProviderCapacity {
                    provider: provider.to_owned(),
                    observed_at,
                    windows: parsed,
                },
            );
        }
    }

    /// Feeds one event, if it is a capacity observation.
    pub fn observe_event(&self, ev: &Event) {
        if let Payload::CapacityObserved { provider, windows } = &*ev.payload {
            self.observe(provider, windows, ev.recorded_at.0);
        }
    }

    pub fn snapshot(&self) -> Vec<ProviderCapacity> {
        let mut all: Vec<ProviderCapacity> = lock(&self.latest).values().cloned().collect();
        all.sort_by(|a, b| a.provider.cmp(&b.provider));
        all
    }

    /// The latest known windows of one provider.
    pub fn provider(&self, provider: &str) -> Option<ProviderCapacity> {
        lock(&self.latest).get(provider).cloned()
    }

    /// One line per provider, for the phone: `claude-code: 5h 19%, 7d 5% (resets Fri 23:00)`.
    pub fn text(&self) -> String {
        let all = self.snapshot();
        if all.is_empty() {
            return "No capacity data yet; it appears after the first harness run.".to_owned();
        }
        all.iter()
            .map(|p| {
                let windows: Vec<String> = p
                    .windows
                    .iter()
                    .map(|(name, w)| {
                        let label = match name.as_str() {
                            "five_hour" => "5h".to_owned(),
                            "seven_day" => "7d".to_owned(),
                            other => other.replace('_', " "),
                        };
                        match w.resets_at {
                            Some(at) => format!(
                                "{label} {:.0}% (resets in {})",
                                w.utilization * 100.0,
                                humanize_until(at)
                            ),
                            None => format!("{label} {:.0}%", w.utilization * 100.0),
                        }
                    })
                    .collect();
                format!("{}: {}", p.provider, windows.join(", "))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn humanize_until(unix_seconds: u64) -> String {
    let now = now_ms().0 / 1000;
    if unix_seconds <= now {
        return "a moment".to_owned();
    }
    let secs = unix_seconds - now;
    match secs {
        0..=3599 => format!("{}m", secs / 60),
        3600..=86_399 => format!("{}h {}m", secs / 3600, (secs % 3600) / 60),
        _ => format!("{}d {}h", secs / 86_400, (secs % 86_400) / 3600),
    }
}

struct PendingApproval {
    run_id: RunId,
    decide: oneshot::Sender<bool>,
}

pub struct ApiState<S> {
    store: Arc<Notifying<S>>,
    workers: WorkerFactory<S>,
    token: String,
    /// The agent used when a request doesn't specify one.
    default_agent: AgentSpec,
    options: ApiOptions,
    /// `Idempotency-Key` → run, so a retried create doesn't start a second
    /// run. In memory for now: it does not survive a restart.
    idempotency: Mutex<HashMap<String, RunId>>,
    /// `(run, Idempotency-Key)` → where that message landed. Held across
    /// the append, so a retry racing the first attempt waits for it
    /// instead of appending twice. In memory, like `idempotency`.
    message_keys: tokio::sync::Mutex<HashMap<(RunId, String), MessageAck>>,
    /// Follow-ups by `(run, Idempotency-Key)`: a retried follow-up gets
    /// the run the first one started.
    follow_up_keys: tokio::sync::Mutex<HashMap<(RunId, String), RunId>>,
    /// Runs this process is driving, so a resume doesn't start a second
    /// worker in the same process (the lease would refuse it anyway).
    active: Mutex<HashSet<RunId>>,
    /// Approval requests whose hook is waiting for an answer.
    pending: Mutex<HashMap<String, PendingApproval>>,
    /// The chief of staff's plans (`.shiioo/plans.json`).
    plans: Mutex<Plans>,
    /// What the scheduler has fired (`.shiioo/routines-state.json`).
    routine_state: Mutex<RoutineState>,
    /// When this process started, in ms: routines are not backfilled
    /// before it.
    started_at: u64,
    /// Nodes that work for the office (`.shiioo/machines.json`).
    machines: Mutex<Machines>,
    /// Woken when a run addressed to a machine appears, so long-polling
    /// nodes hear about it at once.
    work_notify: tokio::sync::Notify,
    /// Runs the owner has taken over in this process.
    takeovers: takeover::Takeovers,
}

/// A run as the API and the bot present it.
#[derive(Clone, Debug, Serialize)]
pub struct RunView {
    pub id: RunId,
    pub task: String,
    /// The `seq` the next event will get.
    pub next_seq: u64,
    pub status: RunStatus,
    /// When the last event was recorded, in ms since the epoch.
    pub updated_at: u64,
    /// The worker that last held the lease (`local-<pid>`, `node-<name>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker: Option<String>,
    /// The machine the run is addressed to, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
    /// For a follow-up, the run it continues.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<RunId>,
    /// When the run was created, in ms since the epoch.
    #[serde(default)]
    pub created_at: u64,
    /// Who works on it: `claude-code`, `codex`, or the native model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// The repository a harness run works on: its label, else the
    /// directory name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The branch a repository run works on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
}

/// What a harness asks before a tool call (Claude Code `PreToolUse` input).
#[derive(Clone, Debug, Deserialize)]
pub struct PermissionInput {
    pub tool_name: String,
    #[serde(default)]
    pub tool_input: Value,
    #[serde(default)]
    pub tool_use_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct PermissionOutcome {
    /// `allow` or `deny`.
    pub decision: String,
    pub reason: String,
    pub approval_id: Option<String>,
}

#[derive(Deserialize)]
pub struct DecideBody {
    /// `allow` or `deny`.
    pub decision: String,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Deserialize)]
pub struct MessageBody {
    pub text: String,
    /// Stop a working harness so it reads the message now, instead of
    /// when its turn ends (`InterruptRequested`). Follow-ups ignore it.
    #[serde(default)]
    pub now: bool,
}

/// Where a message landed in the run's log.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageAck {
    pub seq: u64,
}

#[derive(Deserialize, Default)]
pub struct CancelBody {
    #[serde(default)]
    pub reason: String,
}

impl<S: EventStore + 'static> ApiState<S> {
    pub fn new(
        store: Arc<Notifying<S>>,
        workers: WorkerFactory<S>,
        token: impl Into<String>,
        default_agent: AgentSpec,
    ) -> Arc<Self> {
        Self::with_options(store, workers, token, default_agent, ApiOptions::default())
    }

    pub fn with_options(
        store: Arc<Notifying<S>>,
        workers: WorkerFactory<S>,
        token: impl Into<String>,
        default_agent: AgentSpec,
        options: ApiOptions,
    ) -> Arc<Self> {
        let plans = options
            .data_dir
            .as_deref()
            .map(plans::plans_file)
            .and_then(|path| match Plans::load(&path) {
                Ok(plans) => Some(plans),
                Err(e) => {
                    eprintln!(
                        "plans: cannot read {}: {e}; starting without",
                        path.display()
                    );
                    None
                }
            })
            .unwrap_or_default();
        let routine_state = options
            .data_dir
            .as_deref()
            .map(|d| d.join("routines-state.json"))
            .and_then(|path| match RoutineState::load(&path) {
                Ok(state) => Some(state),
                Err(e) => {
                    eprintln!(
                        "routines: cannot read {}: {e}; starting without",
                        path.display()
                    );
                    None
                }
            })
            .unwrap_or_default();
        let machines = options
            .data_dir
            .as_deref()
            .map(|d| d.join("machines.json"))
            .and_then(|path| match Machines::load(&path) {
                Ok(m) => Some(m),
                Err(e) => {
                    eprintln!(
                        "machines: cannot read {}: {e}; starting without",
                        path.display()
                    );
                    None
                }
            })
            .unwrap_or_default();
        Arc::new(Self {
            store,
            workers,
            token: token.into(),
            default_agent,
            options,
            idempotency: Mutex::new(HashMap::new()),
            message_keys: tokio::sync::Mutex::new(HashMap::new()),
            follow_up_keys: tokio::sync::Mutex::new(HashMap::new()),
            active: Mutex::new(HashSet::new()),
            pending: Mutex::new(HashMap::new()),
            plans: Mutex::new(plans),
            routine_state: Mutex::new(routine_state),
            started_at: routines::now_ms(),
            machines: Mutex::new(machines),
            work_notify: tokio::sync::Notify::new(),
            takeovers: takeover::Takeovers::default(),
        })
    }

    // ----- machines: nodes working for the office -----

    fn save_machines(&self, machines: &Machines) {
        if let Some(dir) = &self.options.data_dir {
            let path = dir.join("machines.json");
            if let Err(e) = machines.save(&path) {
                eprintln!("machines: cannot write {}: {e}", path.display());
            }
        }
    }

    /// A one-time enrollment token for a new node, with these labels.
    pub fn enrollment_token(&self, labels: BTreeMap<String, String>) -> String {
        let mut machines = lock(&self.machines);
        let token = machines.create_enrollment(labels, now_ms().0);
        self.save_machines(&machines);
        token
    }

    /// `POST /v1/machines/enroll`: the token is the credential.
    pub fn enroll_machine(
        &self,
        enrollment_token: &str,
        name: &str,
        labels: BTreeMap<String, String>,
        capabilities: Vec<String>,
        version: String,
    ) -> Result<String, ApiError> {
        let mut machines = lock(&self.machines);
        let token = machines
            .enroll(
                enrollment_token,
                name,
                labels,
                capabilities,
                version,
                now_ms().0,
            )
            .map_err(ApiError::BadRequest)?;
        self.save_machines(&machines);
        Ok(token)
    }

    /// The machine a bearer token belongs to, if it is a machine's.
    pub fn machine_for_token(&self, token: &str) -> Option<String> {
        lock(&self.machines)
            .authenticate(token)
            .map(|m| m.name.clone())
    }

    pub fn machine_heartbeat(
        &self,
        name: &str,
        version: Option<String>,
        capabilities: Option<Vec<String>>,
        in_flight: Vec<String>,
    ) -> bool {
        let mut machines = lock(&self.machines);
        let known = machines.heartbeat(name, version, capabilities, in_flight, now_ms().0);
        if known {
            self.save_machines(&machines);
        }
        known
    }

    pub fn revoke_machine(&self, name: &str) -> bool {
        let mut machines = lock(&self.machines);
        let removed = machines.revoke(name);
        if removed {
            self.save_machines(&machines);
        }
        removed
    }

    pub fn machines_list(&self) -> Vec<Machine> {
        lock(&self.machines).machines.values().cloned().collect()
    }

    pub fn machines_text(&self) -> String {
        lock(&self.machines).text(now_ms().0)
    }

    /// Whether `selector` names a known machine (by name or label).
    pub fn machine_known(&self, selector: &str) -> bool {
        !lock(&self.machines).select(selector, now_ms().0).is_empty()
    }

    /// Open runs addressed to `machine` (by name or a label it has) that are
    /// not finished. The node tries each lease; one held elsewhere is
    /// refused by the store. Waits up to `wait` for work to appear.
    pub async fn work_for(&self, machine: &str, wait: Duration) -> Result<Vec<RunId>, ApiError> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let runs = self.open_runs_for(machine).await?;
            if !runs.is_empty() || tokio::time::Instant::now() >= deadline {
                return Ok(runs);
            }
            tokio::select! {
                () = self.work_notify.notified() => {}
                () = tokio::time::sleep_until(deadline) => {}
            }
        }
    }

    async fn open_runs_for(&self, machine: &str) -> Result<Vec<RunId>, ApiError> {
        let labels: Vec<String> = lock(&self.machines)
            .machines
            .get(machine)
            .map(|m| m.labels.iter().map(|(k, v)| format!("{k}={v}")).collect())
            .unwrap_or_default();
        let mut out = Vec::new();
        for run_id in self.store.list_runs().await? {
            let Ok(log) = self.store.read(run_id, Seq(0)).await else {
                continue;
            };
            let Some(selector) = run_spec(&log).and_then(|s| s.machine) else {
                continue;
            };
            if selector != machine && !labels.contains(&selector) {
                continue;
            }
            if matches!(
                run_status(&log),
                RunStatus::Open | RunStatus::WaitingApproval { .. }
            ) {
                out.push(run_id);
            }
        }
        Ok(out)
    }

    /// Where a run is addressed to, if anywhere.
    pub async fn run_machine(&self, run_id: RunId) -> Option<String> {
        let log = self.store.read(run_id, Seq(0)).await.ok()?;
        run_spec(&log)?.machine
    }

    pub fn roles(&self) -> &[Role] {
        &self.options.roles
    }

    /// One line per role, for `/roles`.
    pub fn roles_text(&self) -> String {
        if self.options.roles.is_empty() {
            return "No roles configured. Add [[roles]] to config.toml to name one in /run @role …"
                .to_owned();
        }
        self.options
            .roles
            .iter()
            .map(Role::describe)
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Assigns a run to a role: the role's employee (or the broker), with
    /// the role's instructions, tools, model, level and limits laid over.
    pub fn assign_role(
        &self,
        role: &str,
        repo: Option<PathBuf>,
        isolation: Option<&str>,
    ) -> Result<(Runtime, AgentSpec, String), ApiError> {
        let role_name = role.trim_start_matches('@');
        let role = self
            .options
            .roles
            .iter()
            .find(|r| r.name == role_name)
            .ok_or_else(|| {
                ApiError::BadRequest(format!(
                    "no role named {role_name}; configured: {}",
                    self.options
                        .roles
                        .iter()
                        .map(|r| r.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            })?;
        let (mut runtime, mut agent, employee) = self.assign_with(
            &role.employee,
            repo,
            isolation.or(role.isolation.as_deref()),
        )?;
        role.apply(&mut runtime, &mut agent);
        Ok((runtime, agent, format!("{employee} as @{}", role.name)))
    }

    /// The path of a configured repository label, or of a path given as is.
    pub fn repo_for(&self, label_or_path: &str) -> Result<PathBuf, ApiError> {
        if let Some(path) = self.repo_path(label_or_path) {
            return Ok(path);
        }
        let path = PathBuf::from(label_or_path);
        if path.is_absolute() && path.join(".git").exists() {
            return Ok(path);
        }
        Err(ApiError::BadRequest(format!(
            "unknown repo {label_or_path}; configured: {}",
            self.repos().join(", ")
        )))
    }

    // ----- plans: the chief of staff -----

    /// Starts a planning run for `goal` on `repo` and remembers it as a plan.
    pub async fn start_plan(
        self: &Arc<Self>,
        repo_label: &str,
        goal: &str,
        role: Option<&str>,
    ) -> Result<RunId, ApiError> {
        let path = self.repo_for(repo_label)?;
        let (runtime, mut agent, _) = match role {
            Some(role) => self.assign_role(role, Some(path), None)?,
            None => self.assign("auto", Some(path))?,
        };
        // A harness gets a read-only allow list; the native loop keeps its
        // employee's tools (the rules still apply).
        if matches!(runtime, Runtime::Harness(_)) {
            agent.tools = plans::planner_tools();
        }
        let spec = RunSpec {
            origin: None,
            park_on_answer: false,
            task: plans::planner_task(repo_label, goal),
            agent,
            runtime,
            machine: None,
        };
        let value = serde_json::to_value(&spec).map_err(|e| ApiError::Internal(e.to_string()))?;
        let run_id = self.store.create_run(value).await?;
        {
            let mut plans = lock(&self.plans);
            plans.plans.insert(
                run_id.to_string(),
                Plan {
                    run_id: run_id.to_string(),
                    repo: repo_label.to_owned(),
                    goal: goal.to_owned(),
                    created_at: routines::now_ms(),
                    notes: String::new(),
                    items: Vec::new(),
                    status: PlanStatus::Planning,
                },
            );
            self.save_plans(&plans);
        }
        self.spawn_drive(run_id);
        Ok(run_id)
    }

    fn save_plans(&self, plans: &Plans) {
        if let Some(dir) = &self.options.data_dir
            && let Err(e) = plans.save(&plans::plans_file(dir))
        {
            eprintln!(
                "plans: cannot write {}: {e}",
                plans::plans_file(dir).display()
            );
        }
    }

    /// If `run_id` was a planning run, reads the plan out of its result and
    /// returns the message for the owner; `None` for ordinary runs.
    pub fn plan_finished(&self, run_id: RunId, status: &RunStatus) -> Option<String> {
        let mut plans = lock(&self.plans);
        let plan = plans.plans.get_mut(&run_id.to_string())?;
        if !matches!(plan.status, PlanStatus::Planning) {
            return None;
        }
        plan.status = match status {
            RunStatus::Completed { output } => match plans::parse_plan(output) {
                Ok((notes, items)) => {
                    plan.notes = notes;
                    plan.items = items;
                    PlanStatus::Proposed
                }
                Err(reason) => PlanStatus::Failed {
                    reason: format!(
                        "{reason}; the planner said: {}",
                        one_line(&Value::from(output.as_str()), 300)
                    ),
                },
            },
            RunStatus::Failed { reason }
            | RunStatus::Cancelled { reason }
            | RunStatus::NeedsAttention { reason } => PlanStatus::Failed {
                reason: reason.clone(),
            },
            RunStatus::Open
            | RunStatus::WaitingApproval { .. }
            | RunStatus::Parked { .. }
            | RunStatus::TakenOver { .. } => PlanStatus::Failed {
                    reason: "the planning run stopped without finishing".into(),
                },
        };
        let text = plans::plan_text(plan);
        self.save_plans(&plans);
        Some(text)
    }

    pub fn plan_text(&self, id_or_prefix: &str) -> Result<String, ApiError> {
        let plans = lock(&self.plans);
        plans
            .find(id_or_prefix)
            .map(plans::plan_text)
            .map_err(ApiError::NotFound)
    }

    pub fn plan_details(&self, id_or_prefix: &str) -> Result<String, ApiError> {
        let plans = lock(&self.plans);
        plans
            .find(id_or_prefix)
            .map(plans::plan_details)
            .map_err(ApiError::NotFound)
    }

    pub fn plans_list(&self) -> Vec<Plan> {
        let mut all: Vec<Plan> = lock(&self.plans).plans.values().cloned().collect();
        all.sort_by_key(|p| std::cmp::Reverse(p.created_at));
        all
    }

    /// The plans, newest first, one line each.
    pub fn plans_text(&self) -> String {
        let all = self.plans_list();
        if all.is_empty() {
            return "No plans yet. /plan <repo> <goal> asks the chief of staff for one.".to_owned();
        }
        all.iter()
            .take(10)
            .map(|p| {
                let state = match &p.status {
                    PlanStatus::Planning => "planning".to_owned(),
                    PlanStatus::Proposed => format!("{} items proposed", p.items.len()),
                    PlanStatus::Approved { issues } => format!("approved, {} issues", issues.len()),
                    PlanStatus::Failed { .. } => "failed".to_owned(),
                };
                format!(
                    "📋 {} {}: {} ({state})",
                    &p.run_id[..8],
                    p.repo,
                    p.goal.chars().take(60).collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Creates one issue per item, labelled for the intake, and marks the
    /// plan approved. Partial failures keep what was created.
    pub async fn approve_plan(&self, id_or_prefix: &str) -> Result<String, ApiError> {
        let issues = self.options.issues.as_ref().ok_or_else(|| {
            ApiError::Conflict("this office cannot create issues (no gh configured)".into())
        })?;
        let plan = {
            let plans = lock(&self.plans);
            plans
                .find(id_or_prefix)
                .cloned()
                .map_err(ApiError::NotFound)?
        };
        if !matches!(plan.status, PlanStatus::Proposed) {
            return Err(ApiError::Conflict(format!(
                "plan {} is not waiting for approval",
                &plan.run_id[..8]
            )));
        }
        let repo = self.repo_for(&plan.repo)?;
        let mut created = Vec::new();
        let mut failure = None;
        for i in 0..plan.items.len() {
            match issues
                .create_issue(
                    &repo,
                    &plan.items[i].title,
                    &plans::issue_body(&plan, i),
                    &self.options.intake_label,
                )
                .await
            {
                Ok(url) => created.push(url),
                Err(e) => {
                    failure = Some(format!("item {}: {e}", i + 1));
                    break;
                }
            }
        }
        let text = {
            let mut plans = lock(&self.plans);
            if let Some(p) = plans.plans.get_mut(&plan.run_id) {
                p.status = PlanStatus::Approved {
                    issues: created.clone(),
                };
            }
            self.save_plans(&plans);
            plans
                .find(&plan.run_id)
                .map(plans::plan_text)
                .unwrap_or_default()
        };
        match failure {
            Some(why) => Err(ApiError::Failed(format!(
                "created {} of {} issues, then: {why}",
                created.len(),
                plan.items.len()
            ))),
            None => Ok(format!(
                "{text}\n\nThe intake picks them up within a minute, in order."
            )),
        }
    }

    // ----- routines and the report -----

    /// Starts a routine's run now. The reply names the run.
    pub async fn fire_routine(self: &Arc<Self>, name: &str) -> Result<String, ApiError> {
        self.fire_routine_run(name).await.map(|(_, reply)| reply)
    }

    /// [`fire_routine`](Self::fire_routine), also returning the run id.
    pub async fn fire_routine_run(
        self: &Arc<Self>,
        name: &str,
    ) -> Result<(RunId, String), ApiError> {
        let routine = self
            .options
            .routines
            .iter()
            .find(|r| r.name == name)
            .cloned()
            .ok_or_else(|| ApiError::NotFound(format!("no routine named {name}")))?;
        let repo = routine
            .repo
            .as_deref()
            .map(|r| self.repo_for(r))
            .transpose()?;
        let (runtime, agent, employee) = match &routine.role {
            Some(role) => self.assign_role(role, repo, None)?,
            None => self.assign(&routine.employee, repo)?,
        };
        let run_id = self
            .start_run(RunSpec {
                origin: None,
                park_on_answer: false,
                task: routine.task.clone(),
                agent,
                runtime,
                machine: routine.machine.clone(),
            })
            .await?;
        {
            let mut state = lock(&self.routine_state);
            state
                .last_fired
                .insert(routine.name.clone(), routines::now_ms());
            self.save_routine_state(&state);
        }
        Ok((
            run_id,
            format!(
                "⏰ routine {} started run {} ({employee}): {}",
                routine.name,
                &run_id.to_string()[..8],
                routine.task.chars().take(120).collect::<String>()
            ),
        ))
    }

    fn save_routine_state(&self, state: &RoutineState) {
        if let Some(dir) = &self.options.data_dir {
            let path = dir.join("routines-state.json");
            if let Err(e) = state.save(&path) {
                eprintln!("routines: cannot write {}: {e}", path.display());
            }
        }
    }

    /// One line per routine with its schedule and next firing.
    pub fn routines_text(&self) -> String {
        if self.options.routines.is_empty() && self.options.report.is_none() {
            return "No routines configured. Add [[routines]] to config.toml.".to_owned();
        }
        let summary = self.routines_summary();
        if summary.is_empty() {
            "No routines are enabled.".to_owned()
        } else {
            summary
        }
    }

    /// `name: schedule, next <when>` per enabled routine, plus the report.
    pub(crate) fn routines_summary(&self) -> String {
        let now = chrono::Local::now();
        let state = lock(&self.routine_state);
        let mut lines = Vec::new();
        for r in self.options.routines.iter().filter(|r| r.enabled) {
            let line = match r.parsed() {
                Ok(schedule) => {
                    let anchor = state
                        .last_fired
                        .get(&r.name)
                        .copied()
                        .unwrap_or(self.started_at);
                    let next = routines::ms_to_local(anchor)
                        .map(|a| schedule.next_after(a.max(now.min(a))))
                        .map(|n| if n < now { schedule.next_after(now) } else { n });
                    format!(
                        "⏰ {}: {schedule}, next {}{}",
                        r.name,
                        next.map_or_else(|| "?".to_owned(), schedule::humanize),
                        r.repo
                            .as_deref()
                            .map_or(String::new(), |repo| format!(" on {repo}"))
                    )
                }
                Err(e) => format!("⏰ {}: {e}", r.name),
            };
            lines.push(line);
        }
        if let Some(report) = &self.options.report {
            match schedule::parse(report) {
                Ok(schedule) => lines.push(format!(
                    "📋 report: {schedule}, next {}",
                    schedule::humanize(schedule.next_after(now))
                )),
                Err(e) => lines.push(format!("📋 report: {e}")),
            }
        }
        lines.join("\n")
    }

    /// The scheduler: checks every 30 seconds what is due and fires it.
    /// Runs until the future is dropped.
    pub async fn run_scheduler(self: Arc<Self>) {
        let report_schedule =
            self.options
                .report
                .as_deref()
                .and_then(|r| match schedule::parse(r) {
                    Ok(s) => Some(s),
                    Err(e) => {
                        eprintln!("report: {e}; the report is only available with /report");
                        None
                    }
                });
        let routines: Vec<(Routine, Schedule)> = self
            .options
            .routines
            .iter()
            .filter(|r| r.enabled)
            .filter_map(|r| match r.parsed() {
                Ok(s) => Some((r.clone(), s)),
                Err(e) => {
                    eprintln!("{e}; the routine is skipped");
                    None
                }
            })
            .collect();
        if routines.is_empty() && report_schedule.is_none() {
            return;
        }
        loop {
            self.scheduler_tick(&routines, report_schedule.as_ref())
                .await;
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    }

    async fn scheduler_tick(
        self: &Arc<Self>,
        routines: &[(Routine, Schedule)],
        report: Option<&Schedule>,
    ) {
        let now = chrono::Local::now();
        for (routine, schedule) in routines {
            let anchor = lock(&self.routine_state)
                .last_fired
                .get(&routine.name)
                .copied()
                .unwrap_or(self.started_at);
            if routines::due(schedule, anchor, now).is_none() {
                continue;
            }
            let reply = match self.fire_routine(&routine.name).await {
                Ok(reply) => reply,
                Err(e) => {
                    let mut state = lock(&self.routine_state);
                    state
                        .last_fired
                        .insert(routine.name.clone(), routines::now_ms());
                    self.save_routine_state(&state);
                    format!("⏰ routine {} could not start: {e}", routine.name)
                }
            };
            if let Some(n) = &self.options.notifications {
                n.message(reply).await;
            }
        }
        if let Some(schedule) = report {
            let (anchor, since) = {
                let state = lock(&self.routine_state);
                (
                    state.last_report.unwrap_or(self.started_at),
                    state.last_report,
                )
            };
            if routines::due(schedule, anchor, now).is_some() {
                let text = self.report(since).await;
                {
                    let mut state = lock(&self.routine_state);
                    state.last_report = Some(routines::now_ms());
                    self.save_routine_state(&state);
                }
                if let Some(n) = &self.options.notifications {
                    n.message(text).await;
                }
            }
        }
    }

    pub fn repos(&self) -> Vec<String> {
        self.options.repos.keys().cloned().collect()
    }

    pub fn repo_path(&self, label: &str) -> Option<PathBuf> {
        self.options.repos.get(label).cloned()
    }

    /// The configured label of a repository path.
    pub fn label_of(&self, path: &PathBuf) -> Option<String> {
        self.options
            .repos
            .iter()
            .find(|(_, p)| *p == path)
            .map(|(label, _)| label.clone())
    }

    pub fn isolation(&self) -> &IsolationPolicy {
        &self.options.isolation
    }

    pub fn default_agent(&self) -> &AgentSpec {
        &self.default_agent
    }

    pub fn capacity(&self) -> &Arc<CapacityTracker> {
        &self.options.capacity
    }

    pub fn employees(&self) -> &[Employee] {
        &self.options.employees
    }

    /// Picks who does a new task: `auto` asks the capacity broker, a name
    /// picks that employee regardless of capacity. Returns the runtime and
    /// agent to run it with, and the employee's name.
    pub fn assign(
        &self,
        employee: &str,
        repo: Option<PathBuf>,
    ) -> Result<(Runtime, AgentSpec, String), ApiError> {
        self.assign_with(employee, repo, None)
    }

    /// [`assign`](Self::assign) with an isolation level named by the
    /// caller instead of the one the policy gives the repository.
    pub fn assign_with(
        &self,
        employee: &str,
        repo: Option<PathBuf>,
        isolation: Option<&str>,
    ) -> Result<(Runtime, AgentSpec, String), ApiError> {
        let label = repo.as_ref().and_then(|path| self.label_of(path));
        let isolation = self
            .options
            .isolation
            .resolve(label.as_deref(), isolation)
            .map_err(ApiError::BadRequest)?;
        let chosen = if employee == "auto" {
            pick(&self.options.employees, &self.options.capacity)
                .map_err(|skipped| {
                    let why: Vec<String> = skipped
                        .iter()
                        .map(|s| format!("{}: {}", s.name, s.reason))
                        .collect();
                    ApiError::Conflict(format!(
                        "every employee is out of capacity ({})",
                        why.join("; ")
                    ))
                })?
                .employee
        } else {
            self.options
                .employees
                .iter()
                .find(|e| e.name == employee)
                .cloned()
                .ok_or_else(|| {
                    ApiError::BadRequest(format!(
                        "no employee named {employee}; configured: {}",
                        self.options
                            .employees
                            .iter()
                            .map(|e| e.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                })?
        };
        let harness_agent = AgentSpec {
            model: String::new(),
            system: String::new(),
            tools: Vec::new(),
            max_turns: 50,
        };
        let (runtime, agent) = match &chosen.kind {
            EmployeeKind::ClaudeCode => {
                let mut h = HarnessSpec::claude_code();
                h.repo = repo;
                h.repo_label = label;
                h.isolation = isolation;
                h.env = self.options.env.clone();
                (Runtime::Harness(h), harness_agent)
            }
            EmployeeKind::Codex => {
                let mut h = HarnessSpec::codex();
                h.repo = repo;
                h.repo_label = label;
                h.isolation = isolation;
                h.env = self.options.env.clone();
                (Runtime::Harness(h), harness_agent)
            }
            EmployeeKind::Native { model } => (
                Runtime::Native,
                AgentSpec {
                    model: model.clone(),
                    system: String::new(),
                    tools: self.default_agent.tools.clone(),
                    max_turns: 50,
                },
            ),
        };
        Ok((runtime, agent, chosen.name))
    }

    /// Loads the latest capacity observations from every run's log. Run
    /// once at startup; the workers' observer keeps it current after that.
    pub async fn seed_capacity(&self) -> Result<(), ApiError> {
        for run_id in self.store.list_runs().await? {
            if let Ok(log) = self.store.read(run_id, Seq(0)).await {
                for ev in &log {
                    self.options.capacity.observe_event(ev);
                }
            }
        }
        Ok(())
    }

    /// Starts a run and a worker for it.
    pub async fn start_run(self: &Arc<Self>, spec: RunSpec) -> Result<RunId, ApiError> {
        if let Some(selector) = &spec.machine
            && !self.machine_known(selector)
        {
            return Err(ApiError::BadRequest(format!(
                "no enrolled machine matches {selector}; /machines lists them"
            )));
        }
        let spec = serde_json::to_value(&spec).map_err(|e| ApiError::Internal(e.to_string()))?;
        let run_id = self.store.create_run(spec).await?;
        self.spawn_drive(run_id);
        Ok(run_id)
    }

    pub async fn run_view(&self, run_id: RunId) -> Result<RunView, ApiError> {
        let log = self.store.read(run_id, Seq(0)).await?;
        Ok(view_of(run_id, &log))
    }

    /// All runs, most recently updated first.
    pub async fn list_runs(&self) -> Result<Vec<RunView>, ApiError> {
        let mut views = Vec::new();
        for run_id in self.store.list_runs().await? {
            if let Ok(log) = self.store.read(run_id, Seq(0)).await {
                views.push(view_of(run_id, &log));
            }
        }
        views.sort_by_key(|v| std::cmp::Reverse(v.updated_at));
        Ok(views)
    }

    /// Finds a run by its full id or a unique prefix.
    pub async fn resolve_run(&self, id_or_prefix: &str) -> Result<RunId, ApiError> {
        if let Ok(id) = uuid::Uuid::parse_str(id_or_prefix) {
            return Ok(RunId(id));
        }
        let prefix = id_or_prefix.trim().to_lowercase();
        if prefix.len() < 4 {
            return Err(ApiError::BadRequest(
                "give at least 4 characters of the run id".into(),
            ));
        }
        let matches: Vec<RunId> = self
            .store
            .list_runs()
            .await?
            .into_iter()
            .filter(|r| r.to_string().starts_with(&prefix))
            .collect();
        match matches.as_slice() {
            [one] => Ok(*one),
            [] => Err(ApiError::NotFound(format!("no run starts with {prefix}"))),
            _ => Err(ApiError::Conflict(format!(
                "{} runs start with {prefix}",
                matches.len()
            ))),
        }
    }

    /// The last `last` events of a run, one line each.
    pub async fn log_text(&self, run_id: RunId, last: usize) -> Result<String, ApiError> {
        let log = self.store.read(run_id, Seq(0)).await?;
        let skip = log.len().saturating_sub(last);
        Ok(log
            .iter()
            .skip(skip)
            .map(|e| format!("#{} {}", e.seq.0, describe(e)))
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// The diff of the run's branch against the repository's current HEAD
    /// (three-dot, so the owner's later commits don't show up). Capped at
    /// 200 KiB.
    pub async fn diff(&self, run_id: RunId) -> Result<String, ApiError> {
        let log = self.store.read(run_id, Seq(0)).await?;
        let spec = run_spec(&log);
        let workspace = spec.as_ref().map_or(run_id, |s| s.workspace_run(run_id));
        let repo = spec
            .and_then(|spec| match spec.runtime {
                Runtime::Harness(h) => h.repo,
                Runtime::Native => None,
            })
            .ok_or_else(|| ApiError::NotFound("this run has no repository".into()))?;
        // A follow-up's branch is the one it continues.
        let range = format!("HEAD...{}", branch_for(workspace));
        let out = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["diff", "--stat", "-p", &range])
            .output()
            .await
            .map_err(|e| ApiError::Failed(format!("running git: {e}")))?;
        if !out.status.success() {
            return Err(ApiError::Failed(
                String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            ));
        }
        const CAP: usize = 200 * 1024;
        let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
        if text.len() > CAP {
            let mut end = CAP;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            text.push_str("\n… truncated\n");
        }
        Ok(text)
    }

    /// The owner writes to a run: the message is appended without a lease
    /// and the run is driven if that made it runnable (a parked run wakes;
    /// a run being worked on reads it at its next turn, a harness when its
    /// process finishes, by resuming the session with it). With `key`, a
    /// repeat returns where the first one landed (`Ok((ack, false))`).
    /// With `now`, a harness at work is stopped and resumed with it at
    /// once (`InterruptRequested` after the message).
    pub async fn send_message(
        self: &Arc<Self>,
        run_id: RunId,
        text: &str,
        now: bool,
        key: Option<String>,
    ) -> Result<(MessageAck, bool), ApiError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ApiError::BadRequest("the message is empty".into()));
        }
        if self.takeovers.is_live(run_id) {
            return Err(ApiError::Conflict(
                "you're at the keyboard: type into the terminal, or hand back first".into(),
            ));
        }
        let mut keys = self.message_keys.lock().await;
        if let Some(ack) = key.as_ref().and_then(|k| keys.get(&(run_id, k.clone()))) {
            return Ok((*ack, false));
        }
        let mut payloads = vec![Payload::UserMessage {
            author: "owner".into(),
            content: text.to_owned(),
        }];
        if now {
            payloads.push(Payload::InterruptRequested { by: "owner".into() });
        }
        let events = match self.store.append_external(run_id, payloads).await {
            Ok(events) => events,
            Err(StoreError::RunClosed(_)) => {
                return Err(ApiError::Conflict(
                    "the run has finished; it no longer accepts messages".into(),
                ));
            }
            Err(e) => return Err(e.into()),
        };
        let ack = MessageAck {
            seq: events.first().map_or(0, |e| e.seq.0),
        };
        if let Some(k) = key {
            keys.insert((run_id, k), ack);
        }
        drop(keys);
        // A parked run is runnable again; an open one that nobody drives
        // (after a restart, say) is picked up so the message is answered.
        if matches!(self.run_view(run_id).await?.status, RunStatus::Open) {
            self.spawn_drive(run_id);
            self.work_notify.notify_waiters();
        }
        Ok((ack, true))
    }

    /// Follows up on a finished harness run: starts a run that continues
    /// it on the same worktree and branch, resuming its harness session
    /// with `text` as the prompt (`docs/design/desk.md` R9). The finished
    /// run stays closed. With `key`, a repeat returns the run the first
    /// one started (`Ok((run, false))`).
    pub async fn follow_up(
        self: &Arc<Self>,
        run_id: RunId,
        text: &str,
        key: Option<String>,
    ) -> Result<(RunId, bool), ApiError> {
        let text = text.trim();
        if text.is_empty() {
            return Err(ApiError::BadRequest("the message is empty".into()));
        }
        let mut keys = self.follow_up_keys.lock().await;
        if let Some(run) = key.as_ref().and_then(|k| keys.get(&(run_id, k.clone()))) {
            return Ok((*run, false));
        }
        let log = self.store.read(run_id, Seq(0)).await?;
        if !log.last().is_some_and(|e| e.payload.is_terminal()) {
            return Err(ApiError::Conflict(
                "the run is still open; send it a message instead".into(),
            ));
        }
        let original =
            run_spec(&log).ok_or_else(|| ApiError::Internal("the run has no spec".into()))?;
        if matches!(original.runtime, Runtime::Native) {
            return Err(ApiError::Conflict(
                "only Claude Code and Codex runs can be followed up; start a new task instead"
                    .into(),
            ));
        }
        let workspace = original.workspace_run(run_id);
        // Two runs in one worktree would edit the same files.
        if let Some(busy) = self.open_run_in(workspace).await? {
            return Err(ApiError::Conflict(format!(
                "run {} is still working on this branch; wait for it or send it a message",
                &busy.to_string()[..8]
            )));
        }
        let session = log
            .iter()
            .rev()
            .find_map(|e| match &*e.payload {
                Payload::HarnessSessionStarted { session_id, .. } => Some(session_id.clone()),
                _ => None,
            })
            .or_else(|| original.origin.as_ref().and_then(|o| o.session.clone()));
        let spec = RunSpec {
            task: text.to_owned(),
            origin: Some(Origin {
                run: run_id,
                seq: Seq(log.len().saturating_sub(1) as u64),
                workspace,
                session,
            }),
            ..original
        };
        let run = self.start_run(spec).await?;
        if let Some(k) = key {
            keys.insert((run_id, k), run);
        }
        Ok((run, true))
    }

    /// An unfinished run working in `workspace`'s worktree, if any.
    async fn open_run_in(&self, workspace: RunId) -> Result<Option<RunId>, ApiError> {
        for run in self.store.list_runs().await? {
            let log = self.store.read(run, Seq(0)).await?;
            if log.last().is_some_and(|e| e.payload.is_terminal()) {
                continue;
            }
            if run_spec(&log).is_some_and(|s| s.workspace_run(run) == workspace) {
                return Ok(Some(run));
            }
        }
        Ok(None)
    }

    /// Asks a run to stop. A run nobody works on (parked, or left open by
    /// a restart) is driven once, which closes it.
    pub async fn cancel(
        self: &Arc<Self>,
        run_id: RunId,
        by: &str,
        reason: &str,
    ) -> Result<(), ApiError> {
        match self
            .store
            .append_external(
                run_id,
                vec![Payload::CancelRequested {
                    by: by.to_owned(),
                    reason: reason.to_owned(),
                }],
            )
            .await
        {
            Ok(_) => {
                self.spawn_drive(run_id);
                Ok(())
            }
            Err(StoreError::RunClosed(_)) => {
                Err(ApiError::Conflict("the run has already finished".into()))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Judges a tool call for a run: the rules decide, or the owner is
    /// asked and this waits for the answer (or the timeout, which denies).
    pub async fn permission(
        self: &Arc<Self>,
        run_id: RunId,
        input: PermissionInput,
    ) -> Result<PermissionOutcome, ApiError> {
        let log = self.store.read(run_id, Seq(0)).await?;
        let repo = repo_label(&self.options.repos, &log);
        let request = PermissionRequest {
            tool: &input.tool_name,
            input: &input.tool_input,
            repo: repo.as_deref(),
        };
        let verdict = self.options.rules.evaluate(&request);
        let summary = RuleSet::summarize(&request);
        let call_id = input.tool_use_id.clone().map(shiioo_core::CallId);
        let decision_event = Payload::PolicyDecision {
            call_id: call_id.clone(),
            tool: input.tool_name.clone(),
            decision: decision_word(verdict.decision).into(),
            rule: verdict.rule.map(|i| format!("rule {}", i + 1)),
            reason: verdict.reason.clone(),
        };

        if verdict.decision != Decision::Ask {
            self.store
                .append_external(run_id, vec![decision_event])
                .await?;
            return Ok(PermissionOutcome {
                decision: decision_word(verdict.decision).into(),
                reason: verdict.reason,
                approval_id: None,
            });
        }

        let approval_id = uuid::Uuid::new_v4().to_string();
        let (tx, rx) = oneshot::channel();
        lock(&self.pending).insert(approval_id.clone(), PendingApproval { run_id, decide: tx });
        self.store
            .append_external(
                run_id,
                vec![
                    decision_event,
                    Payload::ApprovalRequested {
                        approval_id: approval_id.clone(),
                        call_id,
                        tool: input.tool_name.clone(),
                        input: input.tool_input.clone(),
                        summary: summary.clone(),
                        expires_at: Some(now_ms().plus_ms(APPROVAL_TIMEOUT.as_millis() as u64)),
                    },
                ],
            )
            .await?;
        if let Some(n) = &self.options.notifications {
            n.approval_requested(run_id, approval_id.clone(), summary.clone())
                .await;
        }

        let allowed = match tokio::time::timeout(APPROVAL_TIMEOUT, rx).await {
            Ok(Ok(allowed)) => allowed,
            // Decided through another path, or timed out: record the
            // timeout; a decision recorded elsewhere stands.
            Ok(Err(_)) => false,
            Err(_) => {
                lock(&self.pending).remove(&approval_id);
                self.store
                    .append_external(
                        run_id,
                        vec![Payload::ApprovalDecided {
                            approval_id: approval_id.clone(),
                            decision: "deny".into(),
                            by: "timeout".into(),
                            note: Some(format!(
                                "no answer within {} hours",
                                APPROVAL_TIMEOUT.as_secs() / 3600
                            )),
                        }],
                    )
                    .await
                    .ok();
                if let Some(n) = &self.options.notifications {
                    n.approval_resolved(approval_id.clone(), "expired".into())
                        .await;
                }
                false
            }
        };
        Ok(PermissionOutcome {
            decision: if allowed { "allow" } else { "deny" }.into(),
            reason: if allowed {
                format!("allowed by the owner ({summary})")
            } else {
                format!("denied by the owner ({summary})")
            },
            approval_id: Some(approval_id),
        })
    }

    /// Records the owner's answer to an approval and wakes the waiting
    /// hook. `notify` tells the Telegram layer to mark its message; the bot
    /// passes `false` because it edits the message itself.
    pub async fn decide(
        &self,
        approval_id: &str,
        allow: bool,
        by: &str,
        note: Option<String>,
        notify: bool,
    ) -> Result<RunId, ApiError> {
        let pending = lock(&self.pending).remove(approval_id);
        let run_id = match &pending {
            Some(p) => p.run_id,
            None => self.find_open_approval(approval_id).await?,
        };
        self.store
            .append_external(
                run_id,
                vec![Payload::ApprovalDecided {
                    approval_id: approval_id.to_owned(),
                    decision: if allow { "allow" } else { "deny" }.into(),
                    by: by.to_owned(),
                    note,
                }],
            )
            .await?;
        if let Some(p) = pending {
            p.decide.send(allow).ok();
        }
        if notify && let Some(n) = &self.options.notifications {
            n.approval_resolved(
                approval_id.to_owned(),
                if allow { "allowed" } else { "denied" }.into(),
            )
            .await;
        }
        Ok(run_id)
    }

    /// After a restart nothing is pending in memory; the log still knows.
    async fn find_open_approval(&self, approval_id: &str) -> Result<RunId, ApiError> {
        for run_id in self.store.list_runs().await? {
            if let Ok(log) = self.store.read(run_id, Seq(0)).await
                && open_approval(&log).is_some_and(|(id, _)| id == approval_id)
            {
                return Ok(run_id);
            }
        }
        Err(ApiError::NotFound(format!(
            "no open approval {approval_id}"
        )))
    }

    /// Drives a run in the background until it finishes, needs attention,
    /// or fails, then tells the owner. A lost lease is retried a few times:
    /// on one machine that means the lease expired (the laptop slept).
    fn spawn_drive(self: &Arc<Self>, run_id: RunId) {
        // The owner is at the keyboard; the takeover holds the run.
        if self.takeovers.is_live(run_id) {
            return;
        }
        if !lock(&self.active).insert(run_id) {
            return;
        }
        let state = Arc::clone(self);
        tokio::spawn(async move {
            // A run addressed to a machine is the machine's to drive; the
            // office only wakes the nodes that are waiting for work.
            if state.run_machine(run_id).await.is_some() {
                lock(&state.active).remove(&run_id);
                state.work_notify.notify_waiters();
                return;
            }
            let worker = (state.workers)();
            for _ in 0..3 {
                match worker.drive(run_id).await {
                    Ok(RunOutcome::LostLease) => continue,
                    Ok(_) => break,
                    Err(e) => {
                        eprintln!("run {run_id}: worker stopped: {e}");
                        break;
                    }
                }
            }
            lock(&state.active).remove(&run_id);
            // A message that arrived while this drive was ending found the
            // run still active and did not start another; look again now
            // that it is not.
            if let Ok(log) = state.store.read(run_id, Seq(0)).await
                && wake_reason(&log).is_some()
            {
                state.spawn_drive(run_id);
                return;
            }
            state.after_run(run_id).await;
        });
    }

    /// What happens when a run reaches its end, wherever it ran: a plan is
    /// read out of a planning run, the owner is told.
    async fn after_run(self: &Arc<Self>, run_id: RunId) {
        let Ok(view) = self.run_view(run_id).await else {
            return;
        };
        // Stopped for a takeover: the owner is the one who asked.
        if matches!(&view.status, RunStatus::TakenOver { .. })
            || matches!(&view.status, RunStatus::Parked { waiting_for } if waiting_for == "takeover")
        {
            return;
        }
        // A finished planning run becomes a plan; the owner hears about
        // the plan, not about the run.
        let plan = self.plan_finished(run_id, &view.status);
        if let Some(n) = &self.options.notifications {
            match plan {
                Some(text) => n.message(text).await,
                None => {
                    let media = if matches!(view.status, RunStatus::Completed { .. }) {
                        self.artifacts(run_id)
                            .await
                            .unwrap_or_default()
                            .into_iter()
                            .filter(Artifact::is_media)
                            .take(5)
                            .collect()
                    } else {
                        Vec::new()
                    };
                    n.run_ended(run_id, view.task, view.status, media).await;
                }
            }
        }
    }
}

fn decision_word(d: Decision) -> &'static str {
    match d {
        Decision::Allow => "allow",
        Decision::Deny => "deny",
        Decision::Ask => "ask",
    }
}

fn now_ms() -> Timestamp {
    Timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    )
}

/// The run's spec, from its first event.
pub(crate) fn run_spec(log: &[Event]) -> Option<RunSpec> {
    match log.first().map(|e| &*e.payload) {
        Some(Payload::RunCreated { spec }) => serde_json::from_value(spec.clone()).ok(),
        _ => None,
    }
}

/// The label of the repository a harness run works on: a configured label
/// whose path matches, else the directory name.
fn repo_label(repos: &BTreeMap<String, PathBuf>, log: &[Event]) -> Option<String> {
    let Runtime::Harness(h) = run_spec(log)?.runtime else {
        return None;
    };
    let repo = h.repo?;
    repos
        .iter()
        .find(|(_, path)| **path == repo)
        .map(|(label, _)| label.clone())
        .or_else(|| repo.file_name().map(|n| n.to_string_lossy().into_owned()))
}

fn view_of(run_id: RunId, log: &[Event]) -> RunView {
    let spec = run_spec(log);
    let harness = spec.as_ref().and_then(|s| match &s.runtime {
        Runtime::Harness(h) => Some(h.clone()),
        Runtime::Native => None,
    });
    let task = match log.first().map(|e| &*e.payload) {
        Some(Payload::RunCreated { spec }) => spec["task"].as_str().unwrap_or_default().to_owned(),
        _ => String::new(),
    };
    let worker = log.iter().rev().find_map(|e| match &*e.payload {
        Payload::LeaseAcquired { worker, .. } => Some(worker.0.clone()),
        _ => None,
    });
    let machine = match log.first().map(|e| &*e.payload) {
        Some(Payload::RunCreated { spec }) => spec["machine"].as_str().map(str::to_owned),
        _ => None,
    };
    RunView {
        id: run_id,
        task,
        next_seq: log.len() as u64,
        status: run_status(log),
        updated_at: log.last().map_or(0, |e| e.recorded_at.0),
        worker,
        machine,
        origin: spec.as_ref().and_then(|s| s.origin.as_ref()).map(|o| o.run),
        created_at: log.first().map_or(0, |e| e.recorded_at.0),
        runtime: spec.as_ref().map(|s| match &s.runtime {
            Runtime::Harness(h) => h.harness.name().to_owned(),
            Runtime::Native => s.agent.model.clone(),
        }),
        repo: harness.as_ref().and_then(|h| {
            h.repo_label
                .clone()
                .or_else(|| dir_name(h.repo.as_deref()?))
        }),
        branch: harness
            .as_ref()
            .filter(|h| h.repo.is_some())
            .and(spec.as_ref())
            .map(|s| branch_for(s.workspace_run(run_id))),
    }
}

fn dir_name(path: &std::path::Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// One line describing an event, for logs and phone messages.
pub fn describe(ev: &Event) -> String {
    match &*ev.payload {
        Payload::RunCreated { spec } => format!("run created: {}", one_line(&spec["task"], 100)),
        Payload::LeaseAcquired { worker, .. } => {
            format!("lease acquired by {} (epoch {})", worker.0, ev.epoch.0)
        }
        Payload::ModelResponseRecorded {
            model, response, ..
        } => {
            let text: Vec<&str> = response["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|b| b["type"] == "text")
                .filter_map(|b| b["text"].as_str())
                .collect();
            format!("{model}: {}", one_line(&Value::from(text.join(" ")), 140))
        }
        Payload::ToolIntent { tool, input, .. } => format!("→ {tool} {}", one_line(input, 120)),
        Payload::ToolResult {
            output, is_error, ..
        } => format!(
            "{} {}",
            if *is_error { "✗" } else { "←" },
            one_line(output, 120)
        ),
        Payload::ToolOutcomeUnknown { call_id } => format!("? outcome of {} unknown", call_id.0),
        Payload::HarnessSessionStarted {
            harness,
            session_id,
            model,
            ..
        } => format!("{harness} session {session_id} ({model})"),
        Payload::HarnessResult {
            subtype,
            cost_usd,
            permission_denials,
            ..
        } => format!(
            "harness result: {subtype}, cost ${:.4}, {} permission denial(s)",
            cost_usd.unwrap_or(0.0),
            permission_denials.as_array().map_or(0, Vec::len)
        ),
        Payload::CapacityObserved { provider, windows } => format!(
            "capacity {provider}: 5h {:.0}%, 7d {:.0}% used",
            windows["five_hour"]["utilization"].as_f64().unwrap_or(0.0) * 100.0,
            windows["seven_day"]["utilization"].as_f64().unwrap_or(0.0) * 100.0
        ),
        Payload::UserMessage { author, content } => {
            format!(
                "{author}: {}",
                one_line(&Value::from(content.as_str()), 140)
            )
        }
        Payload::PolicyDecision {
            tool,
            decision,
            reason,
            ..
        } => format!("rules: {decision} {tool} ({reason})"),
        Payload::ApprovalRequested { summary, .. } => format!("waiting for approval: {summary}"),
        Payload::ApprovalDecided { decision, by, .. } => format!("{decision} by {by}"),
        Payload::CancelRequested { by, reason } => format!("cancel requested by {by}: {reason}"),
        Payload::InterruptRequested { by } => format!("interrupt requested by {by}"),
        Payload::TakeoverRequested { by } => format!("takeover requested by {by}"),
        Payload::TakeoverStarted { by } => format!("{by} took over at the keyboard"),
        Payload::TakeoverEnded { reason, session } => match session {
            Some(s) => format!("takeover ended ({reason}), session {s}"),
            None => format!("takeover ended ({reason})"),
        },
        Payload::SandboxStarted { level, detail } => {
            let mut parts = Vec::new();
            if let Some(c) = detail["container"].as_str() {
                parts.push(c.to_owned());
            }
            if let Some(s) = detail["sandbox"].as_str() {
                parts.push(s.to_owned());
            }
            if let Some(mode) = detail["egress"]["mode"].as_str() {
                parts.push(format!("egress {mode}"));
            }
            if parts.is_empty() {
                format!("isolation: {level}")
            } else {
                format!("isolation: {level} ({})", parts.join(", "))
            }
        }
        Payload::SecretRedacted { name, count } => {
            format!("redacted secret {name} ({count} occurrence(s))")
        }
        Payload::MailSent { to, subject, .. } => {
            format!("mail sent to {}: {subject}", to.join(", "))
        }
        Payload::RunParked { waiting_for, .. } => format!("parked, waiting for {waiting_for}"),
        Payload::RunWoken { reason } => format!("woken by {reason}"),
        Payload::RunCompleted { output } => format!("completed: {}", one_line(output, 140)),
        Payload::RunFailed { reason } => format!("failed: {reason}"),
        Payload::RunCancelled { reason } => format!("cancelled: {reason}"),
    }
}

fn one_line(v: &Value, max: usize) -> String {
    let s = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s,
    }
}

pub fn router<S: EventStore + 'static>(state: Arc<ApiState<S>>) -> Router {
    let authenticated = Router::new()
        .route("/v1/runs", post(create_run::<S>).get(list_runs::<S>))
        .route("/v1/runs/{id}", get(get_run::<S>))
        .route(
            "/v1/runs/{id}/events",
            get(list_events::<S>).post(append_events::<S>),
        )
        .route("/v1/runs/{id}/events/stream", get(stream_events::<S>))
        .route("/v1/events/stream", get(office_events::<S>))
        .route("/v1/runs/{id}/resume", post(resume_run::<S>))
        .route("/v1/runs/{id}/cancel", post(cancel_run::<S>))
        .route("/v1/runs/{id}/messages", post(post_message::<S>))
        .route("/v1/runs/{id}/follow-up", post(post_follow_up::<S>))
        .route("/v1/runs/{id}/takeover", post(takeover::post_takeover::<S>))
        .route("/v1/runs/{id}/hand-back", post(takeover::post_hand_back::<S>))
        .route("/v1/runs/{id}/lease", post(acquire_lease::<S>))
        .route("/v1/runs/{id}/lease/renew", post(renew_lease::<S>))
        .route("/v1/machines", get(machines_list::<S>))
        .route("/v1/machines/enrollments", post(create_enrollment::<S>))
        .route(
            "/v1/machines/{name}",
            axum::routing::delete(revoke_machine::<S>),
        )
        .route(
            "/v1/machines/{name}/heartbeat",
            post(machine_heartbeat::<S>),
        )
        .route("/v1/machines/{name}/work", get(machine_work::<S>))
        .route("/v1/runs/{id}/permissions", post(permission::<S>))
        .route(
            "/v1/runs/{id}/hook/pre-tool-use",
            post(permission_hook::<S>),
        )
        .route("/v1/runs/{id}/diff", get(diff_run::<S>))
        .route("/v1/runs/{id}/artifacts", get(list_artifacts::<S>))
        .route("/v1/runs/{id}/artifacts/{*path}", get(get_artifact::<S>))
        .route("/v1/mail/send", post(send_mail::<S>))
        .route("/v1/approvals/{id}", post(decide::<S>))
        .route("/v1/capacity", get(capacity::<S>))
        .route("/v1/report", get(report::<S>))
        .route("/v1/roles", get(roles::<S>))
        .route("/v1/routines", get(routines_list::<S>))
        .route("/v1/routines/{name}/run", post(routine_run::<S>))
        .route("/v1/plans", get(plans_list::<S>).post(create_plan::<S>))
        .route("/v1/plans/{id}", get(get_plan::<S>))
        .route("/v1/plans/{id}/approve", post(approve_plan::<S>))
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_token::<S>,
        ));
    Router::new()
        .route(
            "/v1/health",
            get(|| async { Json(json!({ "status": "ok" })) }),
        )
        .route("/v1/machines/enroll", post(enroll_machine::<S>))
        // The ticket from `POST …/takeover` is this route's credential.
        .route(
            "/v1/runs/{id}/terminal",
            get(takeover::terminal_socket::<S>),
        )
        .route("/", get(ui_index))
        .route("/ui", get(ui_index))
        .route("/ui/{*path}", get(ui_file))
        .merge(authenticated)
        .with_state(state)
}

// The dashboard: static files compiled into the binary. The page itself
// is public; everything it shows comes from authenticated calls.
async fn ui_index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../ui/index.html"),
    )
}

/// Every file under `ui/` the page loads, by its path below `/ui/`.
/// `tests/dashboard.rs` checks this list against the directory.
pub const UI_FILES: &[(&str, &str)] = &[
    ("app.css", include_str!("../ui/app.css")),
    ("app.js", include_str!("../ui/app.js")),
    ("commands.js", include_str!("../ui/commands.js")),
    ("core.js", include_str!("../ui/core.js")),
    ("describe.js", include_str!("../ui/describe.js")),
    ("desk.js", include_str!("../ui/desk.js")),
    ("keys.js", include_str!("../ui/keys.js")),
    ("office.js", include_str!("../ui/office.js")),
    ("palette.js", include_str!("../ui/palette.js")),
    ("stream.js", include_str!("../ui/stream.js")),
    ("views/diff.js", include_str!("../ui/views/diff.js")),
    ("views/kit.js", include_str!("../ui/views/kit.js")),
    ("views/log.js", include_str!("../ui/views/log.js")),
    ("views/run.js", include_str!("../ui/views/run.js")),
    ("views/runs.js", include_str!("../ui/views/runs.js")),
    ("views/side.js", include_str!("../ui/views/side.js")),
    ("views/terminal.js", include_str!("../ui/views/terminal.js")),
    ("vendor/xterm-addon-fit.mjs", include_str!("../ui/vendor/xterm-addon-fit.mjs")),
    ("vendor/xterm-addon-webgl.mjs", include_str!("../ui/vendor/xterm-addon-webgl.mjs")),
    ("vendor/xterm.css", include_str!("../ui/vendor/xterm.css")),
    ("vendor/xterm.mjs", include_str!("../ui/vendor/xterm.mjs")),
];

/// The dashboard's fonts (`ui/fonts/`, Latin subsets of the brand fonts
/// in `assets/brand/fonts`), served from the binary like the code.
pub const UI_FONTS: &[(&str, &[u8])] = &[
    (
        "fonts/inter.woff2",
        include_bytes!("../ui/fonts/inter.woff2"),
    ),
    (
        "fonts/bricolage.woff2",
        include_bytes!("../ui/fonts/bricolage.woff2"),
    ),
    (
        "fonts/jetbrains-mono.woff2",
        include_bytes!("../ui/fonts/jetbrains-mono.woff2"),
    ),
];

async fn ui_file(Path(path): Path<String>) -> Response {
    if let Some((_, body)) = UI_FONTS.iter().find(|(name, _)| *name == path) {
        return (
            [
                (header::CONTENT_TYPE, "font/woff2"),
                (header::CACHE_CONTROL, "public, max-age=604800"),
            ],
            *body,
        )
            .into_response();
    }
    let Some((_, body)) = UI_FILES.iter().find(|(name, _)| *name == path) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let content_type = match path.rsplit('.').next() {
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        _ => "application/octet-stream",
    };
    ([(header::CONTENT_TYPE, content_type)], *body).into_response()
}

async fn diff_run<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let text = state.diff(parse_run(&id)?).await?;
    Ok(([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text))
}

async fn require_token<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    match presented {
        Some(token) if constant_time_eq(token.as_bytes(), state.token.as_bytes()) => {
            next.run(request).await
        }
        // An enrolled machine's token is as good as the office's: the
        // owner's own machines are the office.
        Some(token) if state.machine_for_token(token).is_some() => next.run(request).await,
        _ => ApiError::Unauthorized.into_response(),
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[derive(Deserialize)]
struct CreateRun {
    task: String,
    agent: Option<AgentSpec>,
    /// `{"kind":"native"}` (default) or `{"kind":"harness", …}`.
    runtime: Option<Runtime>,
    /// `"auto"` lets the capacity broker choose among the configured
    /// employees; a name picks one. Overrides `runtime` and `agent`.
    employee: Option<String>,
    /// With `employee`: the repository the task is about (a configured
    /// label or an absolute path).
    repo: Option<String>,
    /// Isolation level for a harness run: `worktree`, `container` or
    /// `openshell`. Default: the office's policy for the repository.
    isolation: Option<String>,
    /// A configured role (`[[roles]]`) to run as. Overrides `runtime` and
    /// `agent` like `employee` does.
    role: Option<String>,
    /// A machine to run on: an enrolled node's name or `label=value`.
    machine: Option<String>,
}

async fn create_run<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    headers: HeaderMap,
    Json(body): Json<CreateRun>,
) -> Result<(StatusCode, Json<RunView>), ApiError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if let Some(run_id) = key
        .as_ref()
        .and_then(|k| lock(&state.idempotency).get(k).copied())
    {
        return Ok((StatusCode::OK, Json(state.run_view(run_id).await?)));
    }
    let machine = body.machine.clone();
    let spec = match (&body.role, &body.employee) {
        (Some(role), _) => {
            let repo = body
                .repo
                .as_deref()
                .map(|r| state.repo_for(r))
                .transpose()?;
            let (runtime, agent, _) = state.assign_role(role, repo, body.isolation.as_deref())?;
            RunSpec {
                origin: None,
                park_on_answer: false,
                task: body.task,
                agent,
                runtime,
                machine,
            }
        }
        (None, Some(employee)) => {
            let repo = body.repo.as_deref().map(|label_or_path| {
                state
                    .repo_path(label_or_path)
                    .unwrap_or_else(|| PathBuf::from(label_or_path))
            });
            let (runtime, agent, _) =
                state.assign_with(employee, repo, body.isolation.as_deref())?;
            RunSpec {
                origin: None,
                park_on_answer: false,
                task: body.task,
                agent,
                runtime,
                machine,
            }
        }
        (None, None) => {
            let mut runtime = body.runtime.unwrap_or_default();
            if let (Some(level), Runtime::Harness(h)) = (&body.isolation, &mut runtime) {
                let label = h.repo.as_ref().and_then(|p| state.label_of(p));
                h.isolation = state
                    .options
                    .isolation
                    .resolve(label.as_deref(), Some(level))
                    .map_err(ApiError::BadRequest)?;
            }
            RunSpec {
                origin: None,
                park_on_answer: false,
                task: body.task,
                agent: body.agent.unwrap_or_else(|| state.default_agent.clone()),
                runtime,
                machine,
            }
        }
    };
    let run_id = state.start_run(spec).await?;
    if let Some(key) = key {
        lock(&state.idempotency).insert(key, run_id);
    }
    Ok((StatusCode::CREATED, Json(state.run_view(run_id).await?)))
}

// ----- node-facing store routes -----

#[derive(Deserialize)]
struct AcquireLease {
    worker: WorkerId,
    ttl_ms: u64,
}

async fn acquire_lease<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    Json(body): Json<AcquireLease>,
) -> Result<Json<Lease>, ApiError> {
    let run_id = parse_run(&id)?;
    Ok(Json(
        state
            .store
            .acquire_lease(run_id, body.worker, body.ttl_ms)
            .await?,
    ))
}

#[derive(Deserialize)]
struct RenewLease {
    lease: Lease,
    ttl_ms: u64,
}

async fn renew_lease<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    Json(body): Json<RenewLease>,
) -> Result<Json<Lease>, ApiError> {
    let run_id = parse_run(&id)?;
    if body.lease.run_id != run_id {
        return Err(ApiError::BadRequest("the lease is for another run".into()));
    }
    Ok(Json(
        state.store.renew_lease(&body.lease, body.ttl_ms).await?,
    ))
}

#[derive(Deserialize)]
struct AppendEvents {
    lease: Lease,
    expected_next: Seq,
    payloads: Vec<Payload>,
}

/// A node worker's append, under its lease. A terminal event ends the run
/// for the office too: plans are read out and the owner is told.
async fn append_events<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    Json(body): Json<AppendEvents>,
) -> Result<Json<Vec<Event>>, ApiError> {
    let run_id = parse_run(&id)?;
    if body.lease.run_id != run_id {
        return Err(ApiError::BadRequest("the lease is for another run".into()));
    }
    let terminal = body.payloads.iter().any(Payload::is_terminal);
    let events = state
        .store
        .append(&body.lease, body.expected_next, body.payloads)
        .await?;
    if terminal {
        let state = Arc::clone(&state);
        tokio::spawn(async move { state.after_run(run_id).await });
    }
    Ok(Json(events))
}

// ----- machines -----

#[derive(Deserialize, Default)]
struct CreateEnrollment {
    #[serde(default)]
    labels: BTreeMap<String, String>,
}

async fn create_enrollment<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    body: Option<Json<CreateEnrollment>>,
) -> (StatusCode, Json<Value>) {
    let labels = body.map(|Json(b)| b.labels).unwrap_or_default();
    let token = state.enrollment_token(labels);
    (
        StatusCode::CREATED,
        Json(json!({
            "token": token,
            "expires_in_secs": machines::ENROLLMENT_TTL_MS / 1000,
        })),
    )
}

#[derive(Deserialize)]
struct Enroll {
    token: String,
    name: String,
    #[serde(default)]
    labels: BTreeMap<String, String>,
    #[serde(default)]
    capabilities: Vec<String>,
    #[serde(default)]
    version: String,
}

/// Unauthenticated on purpose: the one-time enrollment token in the body
/// is the credential.
async fn enroll_machine<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Json(body): Json<Enroll>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let token = state.enroll_machine(
        &body.token,
        &body.name,
        body.labels,
        body.capabilities,
        body.version,
    )?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "name": body.name.trim(), "machine_token": token })),
    ))
}

async fn machines_list<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
) -> Json<Value> {
    let now = now_ms().0;
    let machines: Vec<Value> = state
        .machines_list()
        .into_iter()
        .map(|m| {
            json!({
                "name": m.name,
                "labels": m.labels,
                "capabilities": m.capabilities,
                "version": m.version,
                "online": m.online(now),
                "last_seen": m.last_seen,
                "in_flight": m.in_flight,
                "created_at": m.created_at,
            })
        })
        .collect();
    Json(json!({ "machines": machines, "text": state.machines_text() }))
}

async fn revoke_machine<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(name): Path<String>,
) -> Result<StatusCode, ApiError> {
    if state.revoke_machine(&name) {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound(format!("no machine named {name}")))
    }
}

#[derive(Deserialize, Default)]
struct HeartbeatBody {
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    capabilities: Option<Vec<String>>,
    #[serde(default)]
    in_flight: Vec<String>,
}

async fn machine_heartbeat<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(name): Path<String>,
    body: Option<Json<HeartbeatBody>>,
) -> Result<Json<Value>, ApiError> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    if state.machine_heartbeat(&name, body.version, body.capabilities, body.in_flight) {
        Ok(Json(json!({ "ok": true })))
    } else {
        Err(ApiError::NotFound(format!(
            "no machine named {name}; enroll it first"
        )))
    }
}

#[derive(Deserialize, Default)]
struct WorkQuery {
    /// Seconds to wait for work before answering with none (at most 50).
    #[serde(default)]
    wait: u64,
}

async fn machine_work<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(name): Path<String>,
    Query(q): Query<WorkQuery>,
) -> Result<Json<Value>, ApiError> {
    let runs = state
        .work_for(&name, Duration::from_secs(q.wait.min(50)))
        .await?;
    Ok(Json(json!({ "runs": runs })))
}

async fn list_artifacts<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Artifact>>, ApiError> {
    Ok(Json(state.artifacts(parse_run(&id)?).await?))
}

async fn get_artifact<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path((id, path)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let (bytes, content_type) = state.artifact_bytes(parse_run(&id)?, &path).await?;
    let name = path.rsplit('/').next().unwrap_or("file").to_owned();
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", name.replace('"', "")),
            ),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
struct SendMail {
    /// The run asking; its rules and approvals apply. None: the owner.
    #[serde(default)]
    run: Option<RunId>,
    #[serde(flatten)]
    mail: MailRequest,
}

async fn send_mail<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Json(body): Json<SendMail>,
) -> Result<Json<MailOutcome>, ApiError> {
    Ok(Json(state.send_mail(body.run, body.mail).await?))
}

async fn report<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
) -> Result<String, ApiError> {
    Ok(state.report(None).await)
}

async fn roles<S: EventStore + 'static>(State(state): State<Arc<ApiState<S>>>) -> Json<Vec<Role>> {
    Json(state.roles().to_vec())
}

async fn routines_list<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
) -> Json<Value> {
    Json(json!({
        "routines": state.options.routines,
        "report": state.options.report,
        "summary": state.routines_summary(),
    }))
}

async fn routine_run<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(name): Path<String>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let (run, reply) = state.fire_routine_run(&name).await?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "run": run, "message": reply })),
    ))
}

#[derive(Deserialize)]
struct CreatePlan {
    /// A configured repository label.
    repo: String,
    goal: String,
    /// A role for the planner; default: the broker's choice.
    #[serde(default)]
    role: Option<String>,
}

async fn create_plan<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Json(body): Json<CreatePlan>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let run_id = state
        .start_plan(&body.repo, &body.goal, body.role.as_deref())
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(json!({ "plan": run_id, "run": run_id })),
    ))
}

async fn plans_list<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
) -> Json<Vec<Plan>> {
    Json(state.plans_list())
}

async fn get_plan<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
) -> Result<Json<Plan>, ApiError> {
    let plans = lock(&state.plans);
    plans
        .find(&id)
        .cloned()
        .map(Json)
        .map_err(ApiError::NotFound)
}

async fn approve_plan<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let reply = state.approve_plan(&id).await?;
    let plan = lock(&state.plans).find(&id).cloned().ok();
    Ok(Json(json!({ "message": reply, "plan": plan })))
}

async fn capacity<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
) -> Json<Vec<ProviderCapacity>> {
    Json(state.options.capacity.snapshot())
}

async fn list_runs<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
) -> Result<Json<Vec<RunView>>, ApiError> {
    Ok(Json(state.list_runs().await?))
}

async fn get_run<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
) -> Result<Json<RunView>, ApiError> {
    Ok(Json(state.run_view(parse_run(&id)?).await?))
}

async fn resume_run<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<RunView>), ApiError> {
    let run_id = parse_run(&id)?;
    let current = state.run_view(run_id).await?;
    if !matches!(
        current.status,
        RunStatus::Open | RunStatus::WaitingApproval { .. }
    ) {
        return Err(ApiError::Conflict("only open runs can be resumed".into()));
    }
    state.spawn_drive(run_id);
    state.work_notify.notify_waiters();
    Ok((StatusCode::ACCEPTED, Json(current)))
}

async fn post_message<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<MessageBody>,
) -> Result<(StatusCode, Json<MessageAck>), ApiError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let (ack, new) = state
        .send_message(parse_run(&id)?, &body.text, body.now, key)
        .await?;
    let code = if new {
        StatusCode::ACCEPTED
    } else {
        StatusCode::OK
    };
    Ok((code, Json(ack)))
}

async fn post_follow_up<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<MessageBody>,
) -> Result<(StatusCode, Json<RunView>), ApiError> {
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let (run, new) = state.follow_up(parse_run(&id)?, &body.text, key).await?;
    let code = if new {
        StatusCode::CREATED
    } else {
        StatusCode::OK
    };
    Ok((code, Json(state.run_view(run).await?)))
}

async fn cancel_run<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    body: Option<Json<CancelBody>>,
) -> Result<StatusCode, ApiError> {
    let reason = body.map(|b| b.0.reason).unwrap_or_default();
    state.cancel(parse_run(&id)?, "owner", &reason).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn permission<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    Json(input): Json<PermissionInput>,
) -> Result<Json<PermissionOutcome>, ApiError> {
    Ok(Json(state.permission(parse_run(&id)?, input).await?))
}

/// The permission endpoint in the harness hook's own dialect: takes what
/// the harness writes to the hook's stdin, answers what the hook must
/// print. Lets a sandbox use `curl` as its hook.
async fn permission_hook<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    Json(input): Json<PermissionInput>,
) -> Result<Json<Value>, ApiError> {
    let outcome = state.permission(parse_run(&id)?, input).await?;
    Ok(Json(hook_output(&outcome)))
}

/// Claude Code's and Codex's `PreToolUse` hook output for an outcome.
pub fn hook_output(outcome: &PermissionOutcome) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": if outcome.decision == "allow" { "allow" } else { "deny" },
            "permissionDecisionReason": outcome.reason,
        }
    })
}

async fn decide<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    Json(body): Json<DecideBody>,
) -> Result<Json<Value>, ApiError> {
    let allow = match body.decision.as_str() {
        "allow" => true,
        "deny" => false,
        other => {
            return Err(ApiError::BadRequest(format!(
                "decision must be allow or deny, not {other}"
            )));
        }
    };
    let run_id = state.decide(&id, allow, "owner", body.note, true).await?;
    Ok(Json(
        json!({ "run": run_id, "approval_id": id, "decision": body.decision }),
    ))
}

#[derive(Deserialize)]
struct FromSeq {
    from: Option<u64>,
}

async fn list_events<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    Query(q): Query<FromSeq>,
) -> Result<Json<Vec<Event>>, ApiError> {
    let run_id = parse_run(&id)?;
    Ok(Json(
        state.store.read(run_id, Seq(q.from.unwrap_or(0))).await?,
    ))
}

async fn stream_events<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    Query(q): Query<FromSeq>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    let run_id = parse_run(&id)?;
    // Last-Event-ID wins over ?from: it is what a reconnecting client sends.
    let from = match headers.get("last-event-id").and_then(|v| v.to_str().ok()) {
        Some(last) => last
            .trim()
            .parse::<u64>()
            .map(|seq| seq + 1)
            .map_err(|_| ApiError::BadRequest("Last-Event-ID must be an event seq".into()))?,
        None => q.from.unwrap_or(0),
    };
    // Subscribe before the first read, so no write can slip in between.
    let changes = state.store.subscribe(run_id);
    // Fail with 404 now rather than inside the stream.
    state.store.read(run_id, Seq(from)).await?;

    let feed = Feed {
        store: Arc::clone(&state.store),
        run_id,
        next: Seq(from),
        pending: VecDeque::new(),
        changes,
        finished: false,
    };
    let stream = futures_util::stream::unfold(feed, |mut feed| async move {
        let event = feed.next_event().await?;
        let frame = SseEvent::default()
            .event("event")
            .id(event.seq.0.to_string())
            .json_data(&event)
            .unwrap_or_else(|e| SseEvent::default().event("error").data(e.to_string()));
        Some((Ok(frame), feed))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Reads a run's log from `next` onwards, waiting for new events between
/// reads. The log is the source; change notifications only wake it up, and
/// a periodic re-read covers writes made by other processes.
struct Feed<S> {
    store: Arc<Notifying<S>>,
    run_id: RunId,
    next: Seq,
    pending: VecDeque<Event>,
    changes: tokio::sync::watch::Receiver<u64>,
    finished: bool,
}

impl<S: EventStore> Feed<S> {
    async fn next_event(&mut self) -> Option<Event> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return Some(event);
            }
            if self.finished {
                return None;
            }
            let events = self.store.read(self.run_id, self.next).await.ok()?;
            if let Some(last) = events.last() {
                self.next = last.seq.next();
                self.finished = last.payload.is_terminal();
                self.pending.extend(events);
                continue;
            }
            tokio::select! {
                changed = self.changes.changed() => {
                    // The sender is dropped once the run finishes; one more
                    // read picks up the terminal event.
                    if changed.is_err() {
                        self.changes = self.store.subscribe(self.run_id);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
                () = tokio::time::sleep(Duration::from_secs(2)) => {}
            }
        }
    }
}

/// The office-wide stream: a `run` frame with the run's view whenever a run
/// is created or gets new events, after a first pass over every run (or,
/// with `Last-Event-ID`, every run updated since then) closed by one
/// `ready` frame. Frame ids are the view's `updated_at`; a reconnect may
/// repeat a run, never miss one. Views are snapshots, so a client keeps
/// the latest per run id.
async fn office_events<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    headers: HeaderMap,
) -> Result<Sse<impl Stream<Item = Result<SseEvent, Infallible>>>, ApiError> {
    let since = match headers.get("last-event-id").and_then(|v| v.to_str().ok()) {
        Some(last) => Some(last.trim().parse::<u64>().map_err(|_| {
            ApiError::BadRequest("Last-Event-ID must be an updated_at from this stream".into())
        })?),
        None => None,
    };
    // Subscribe before the first scan, so no write can slip in between.
    let feed = OfficeFeed {
        changes: state.store.subscribe_all(),
        store: Arc::clone(&state.store),
        sent: HashMap::new(),
        pending: VecDeque::new(),
        since,
        scanned: false,
        ready_sent: false,
    };
    let stream = futures_util::stream::unfold(feed, |mut feed| async move {
        let frame = match feed.next_frame().await? {
            OfficeFrame::Ready => SseEvent::default().event("ready").data("{}"),
            OfficeFrame::Run(view) => SseEvent::default()
                .event("run")
                .id(view.updated_at.to_string())
                .json_data(&view)
                .unwrap_or_else(|e| SseEvent::default().event("error").data(e.to_string())),
        };
        Some((Ok(frame), feed))
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

enum OfficeFrame {
    Run(Box<RunView>),
    Ready,
}

/// Turns the store's "this run changed" hints into run views, sending a
/// run only when its log grew since it was last sent.
struct OfficeFeed<S> {
    store: Arc<Notifying<S>>,
    changes: tokio::sync::broadcast::Receiver<RunId>,
    /// The `next_seq` of each run as last sent (or skipped by `since`).
    sent: HashMap<RunId, u64>,
    pending: VecDeque<RunView>,
    since: Option<u64>,
    scanned: bool,
    ready_sent: bool,
}

impl<S: EventStore> OfficeFeed<S> {
    async fn next_frame(&mut self) -> Option<OfficeFrame> {
        use tokio::sync::broadcast::error::RecvError;
        loop {
            if let Some(view) = self.pending.pop_front() {
                return Some(OfficeFrame::Run(Box::new(view)));
            }
            if !self.scanned {
                self.scanned = true;
                let since = self.since.take();
                self.scan(None, since).await;
                continue;
            }
            if !self.ready_sent {
                self.ready_sent = true;
                return Some(OfficeFrame::Ready);
            }
            tokio::select! {
                first = self.changes.recv() => match first {
                    Ok(run_id) => {
                        // Writes come in bursts (a harness streaming its
                        // turn); gather them briefly and send each run once.
                        let mut ids = HashSet::from([run_id]);
                        let window = tokio::time::sleep(Duration::from_millis(100));
                        tokio::pin!(window);
                        let mut lagged = false;
                        loop {
                            tokio::select! {
                                next = self.changes.recv() => match next {
                                    Ok(id) => { ids.insert(id); }
                                    Err(RecvError::Lagged(_)) => { lagged = true; break; }
                                    Err(RecvError::Closed) => return None,
                                },
                                () = &mut window => break,
                            }
                        }
                        if lagged {
                            self.scan(None, None).await;
                        } else {
                            self.scan(Some(ids), None).await;
                        }
                    }
                    Err(RecvError::Lagged(_)) => self.scan(None, None).await,
                    Err(RecvError::Closed) => return None,
                },
                // Writes by other processes (a `shiioo run` on the same
                // store) are not announced; a periodic pass finds them.
                () = tokio::time::sleep(Duration::from_secs(10)) => self.scan(None, None).await,
            }
        }
    }

    /// Queues the views of `only` (or of every run) whose log grew since
    /// they were last sent, oldest change first. With `since`, runs last
    /// updated before it are marked as sent instead.
    async fn scan(&mut self, only: Option<HashSet<RunId>>, since: Option<u64>) {
        let ids = match only {
            Some(ids) => ids.into_iter().collect(),
            None => self.store.list_runs().await.unwrap_or_default(),
        };
        let mut views = Vec::new();
        for run_id in ids {
            let Ok(log) = self.store.read(run_id, Seq(0)).await else {
                continue;
            };
            let view = view_of(run_id, &log);
            if self.sent.insert(run_id, view.next_seq) == Some(view.next_seq) {
                continue;
            }
            if since.is_none_or(|t| view.updated_at >= t) {
                views.push(view);
            }
        }
        views.sort_by_key(|v| v.updated_at);
        self.pending.extend(views);
    }
}

pub(crate) fn parse_run(id: &str) -> Result<RunId, ApiError> {
    uuid::Uuid::parse_str(id)
        .map(RunId)
        .map_err(|_| ApiError::BadRequest(format!("not a run id: {id}")))
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Errors as RFC 9457 problem details. Internal store errors (lease
/// conflicts and the like) are never shown to clients in detail.
#[derive(Debug)]
pub enum ApiError {
    Unauthorized,
    NotFound(String),
    BadRequest(String),
    Conflict(String),
    /// The rules or the owner said no.
    Forbidden(String),
    /// A store error a node worker must see as such (fencing, an expired
    /// lease, a held lease, a sequence conflict): carried in JSON form.
    Store(StoreError),
    /// Something the request asked for could not be done (a failed git
    /// command, for instance). Unlike `Internal`, the detail is shown.
    Failed(String),
    Internal(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Unauthorized => f.write_str("unauthorized"),
            ApiError::Store(e) => write!(f, "{e}"),
            ApiError::Forbidden(d) => write!(f, "forbidden: {d}"),
            ApiError::NotFound(d)
            | ApiError::BadRequest(d)
            | ApiError::Conflict(d)
            | ApiError::Failed(d)
            | ApiError::Internal(d) => f.write_str(d),
        }
    }
}

impl From<StoreError> for ApiError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::RunNotFound(id) => ApiError::NotFound(format!("run {id} not found")),
            other @ (StoreError::Fenced { .. }
            | StoreError::LeaseExpired(_)
            | StoreError::LeaseHeld { .. }
            | StoreError::SeqConflict { .. }
            | StoreError::RunClosed(_)
            | StoreError::InvalidPayload(_)) => ApiError::Store(other),
            other => ApiError::Internal(other.to_string()),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, slug, title, detail) = match self {
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Missing or invalid token",
                None,
            ),
            ApiError::NotFound(d) => (StatusCode::NOT_FOUND, "not-found", "Not found", Some(d)),
            ApiError::BadRequest(d) => (
                StatusCode::BAD_REQUEST,
                "bad-request",
                "Bad request",
                Some(d),
            ),
            ApiError::Conflict(d) => (StatusCode::CONFLICT, "conflict", "Conflict", Some(d)),
            ApiError::Forbidden(d) => (StatusCode::FORBIDDEN, "forbidden", "Forbidden", Some(d)),
            ApiError::Store(e) => {
                let status = match e {
                    StoreError::RunNotFound(_) => StatusCode::NOT_FOUND,
                    _ => StatusCode::CONFLICT,
                };
                let body = json!({
                    "type": "https://shiioo.dev/problems/store",
                    "title": "Store",
                    "status": status.as_u16(),
                    "detail": e.to_string(),
                    "store": e.to_json(),
                });
                return (status, Json(body)).into_response();
            }
            ApiError::Failed(d) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed",
                "Operation failed",
                Some(d),
            ),
            ApiError::Internal(d) => {
                eprintln!("internal error: {d}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "Internal error",
                    None,
                )
            }
        };
        let body = json!({
            // Placeholder domain until the project's domain is decided.
            "type": format!("https://shiioo.dev/problems/{slug}"),
            "title": title,
            "status": status.as_u16(),
            "detail": detail,
        });
        (
            status,
            [(header::CONTENT_TYPE, "application/problem+json")],
            body.to_string(),
        )
            .into_response()
    }
}
