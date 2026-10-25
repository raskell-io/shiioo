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
//!
//! - `GET  /v1/runs/{id}/diff` is the run's branch diff against the repo's HEAD.
//! - `GET  /` serves the mobile dashboard (`ui/`), which calls the API with the token.
//!
//! Everything except `GET /v1/health` and the dashboard files needs
//! `Authorization: Bearer <token>`.

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shiioo_core::{Event, EventStore, Payload, RunId, Seq, StoreError, Timestamp};
use shiioo_rules::{Decision, PermissionRequest, RuleSet};
use shiioo_runtime::harness::claude_code::HOOK_TIMEOUT_SECS;
use shiioo_runtime::{
    AgentSpec, RunOutcome, RunSpec, RunStatus, Runtime, Worker, branch_for, open_approval,
    run_status,
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
    fn run_ended(&self, run: RunId, task: String, status: RunStatus) -> BoxFuture<'_, ()>;
}

pub struct ApiOptions {
    pub rules: RuleSet,
    /// Repository label → path, for `/run <label> …` and for repo-scoped
    /// rules.
    pub repos: BTreeMap<String, PathBuf>,
    pub notifications: Option<Arc<dyn Notifications>>,
}

impl Default for ApiOptions {
    fn default() -> Self {
        Self {
            rules: RuleSet::defaults(),
            repos: BTreeMap::new(),
            notifications: None,
        }
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
    /// Runs this process is driving, so a resume doesn't start a second
    /// worker in the same process (the lease would refuse it anyway).
    active: Mutex<HashSet<RunId>>,
    /// Approval requests whose hook is waiting for an answer.
    pending: Mutex<HashMap<String, PendingApproval>>,
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
        Arc::new(Self {
            store,
            workers,
            token: token.into(),
            default_agent,
            options,
            idempotency: Mutex::new(HashMap::new()),
            active: Mutex::new(HashSet::new()),
            pending: Mutex::new(HashMap::new()),
        })
    }

    pub fn repos(&self) -> Vec<String> {
        self.options.repos.keys().cloned().collect()
    }

    pub fn repo_path(&self, label: &str) -> Option<PathBuf> {
        self.options.repos.get(label).cloned()
    }

    pub fn default_agent(&self) -> &AgentSpec {
        &self.default_agent
    }

    /// Starts a run and a worker for it.
    pub async fn start_run(self: &Arc<Self>, spec: RunSpec) -> Result<RunId, ApiError> {
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
        let repo = run_spec(&log)
            .and_then(|spec| match spec.runtime {
                Runtime::Harness(h) => h.repo,
                Runtime::Native => None,
            })
            .ok_or_else(|| ApiError::NotFound("this run has no repository".into()))?;
        let range = format!("HEAD...{}", branch_for(run_id));
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

    /// Asks a run to stop. The worker closes it at its next step.
    pub async fn cancel(&self, run_id: RunId, by: &str, reason: &str) -> Result<(), ApiError> {
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
            Ok(_) => Ok(()),
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
        if !lock(&self.active).insert(run_id) {
            return;
        }
        let state = Arc::clone(self);
        tokio::spawn(async move {
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
            if let Some(n) = &state.options.notifications
                && let Ok(view) = state.run_view(run_id).await
            {
                n.run_ended(run_id, view.task, view.status).await;
            }
        });
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
fn run_spec(log: &[Event]) -> Option<RunSpec> {
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
    let task = match log.first().map(|e| &*e.payload) {
        Some(Payload::RunCreated { spec }) => spec["task"].as_str().unwrap_or_default().to_owned(),
        _ => String::new(),
    };
    RunView {
        id: run_id,
        task,
        next_seq: log.len() as u64,
        status: run_status(log),
        updated_at: log.last().map_or(0, |e| e.recorded_at.0),
    }
}

/// One line describing an event, for logs and phone messages.
pub fn describe(ev: &Event) -> String {
    match &*ev.payload {
        Payload::RunCreated { spec } => format!("run created: {}", one_line(&spec["task"], 100)),
        Payload::LeaseAcquired { worker, .. } => {
            format!("lease acquired by {} (epoch {})", worker.0, ev.epoch.0)
        }
        Payload::ModelResponseRecorded { model, response } => {
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
        .route("/v1/runs/{id}/events", get(list_events::<S>))
        .route("/v1/runs/{id}/events/stream", get(stream_events::<S>))
        .route("/v1/runs/{id}/resume", post(resume_run::<S>))
        .route("/v1/runs/{id}/cancel", post(cancel_run::<S>))
        .route("/v1/runs/{id}/permissions", post(permission::<S>))
        .route("/v1/runs/{id}/diff", get(diff_run::<S>))
        .route("/v1/approvals/{id}", post(decide::<S>))
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_token::<S>,
        ));
    Router::new()
        .route(
            "/v1/health",
            get(|| async { Json(json!({ "status": "ok" })) }),
        )
        .route("/", get(ui_index))
        .route("/ui", get(ui_index))
        .route("/ui/app.js", get(ui_js))
        .route("/ui/app.css", get(ui_css))
        .merge(authenticated)
        .with_state(state)
}

// The dashboard: three static files compiled into the binary. The page
// itself is public; everything it shows comes from authenticated calls.
async fn ui_index() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        include_str!("../ui/index.html"),
    )
}

async fn ui_js() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        include_str!("../ui/app.js"),
    )
}

async fn ui_css() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../ui/app.css"),
    )
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
    let spec = RunSpec {
        task: body.task,
        agent: body.agent.unwrap_or_else(|| state.default_agent.clone()),
        runtime: body.runtime.unwrap_or_default(),
    };
    let run_id = state.start_run(spec).await?;
    if let Some(key) = key {
        lock(&state.idempotency).insert(key, run_id);
    }
    Ok((StatusCode::CREATED, Json(state.run_view(run_id).await?)))
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
    Ok((StatusCode::ACCEPTED, Json(current)))
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

fn parse_run(id: &str) -> Result<RunId, ApiError> {
    uuid::Uuid::parse_str(id)
        .map(RunId)
        .map_err(|_| ApiError::BadRequest(format!("not a run id: {id}")))
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
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
    /// Something the request asked for could not be done (a failed git
    /// command, for instance). Unlike `Internal`, the detail is shown.
    Failed(String),
    Internal(String),
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ApiError::Unauthorized => f.write_str("unauthorized"),
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
