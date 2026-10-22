//! Shiioo's HTTP API (`docs/design/api.md`, ADR-0004).
//!
//! - `POST /v1/runs` starts a run and a worker for it.
//! - `GET  /v1/runs/{id}` folds the log into a status.
//! - `GET  /v1/runs/{id}/events?from=N` returns events from `seq` N.
//! - `GET  /v1/runs/{id}/events/stream` streams events as SSE. Each SSE id
//!   is the event's `seq`; reconnecting with `Last-Event-ID` resumes right
//!   after it, served from the log, so a client never misses or repeats an
//!   event. The stream ends after the run's terminal event.
//! - `POST /v1/runs/{id}/resume` starts a worker for an open run again.
//!
//! Everything except `GET /v1/health` needs `Authorization: Bearer <token>`.

use axum::extract::{Path, Query, Request, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::Stream;
use serde::{Deserialize, Serialize};
use serde_json::json;
use shiioo_core::{Event, EventStore, RunId, Seq, StoreError};
use shiioo_runtime::{AgentSpec, RunOutcome, RunSpec, RunStatus, Worker, run_status};
use shiioo_store::Notifying;
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type WorkerFactory<S> = Arc<dyn Fn() -> Worker<Notifying<S>> + Send + Sync>;

pub struct ApiState<S> {
    store: Arc<Notifying<S>>,
    workers: WorkerFactory<S>,
    token: String,
    /// The agent used when a request doesn't specify one.
    default_agent: AgentSpec,
    /// `Idempotency-Key` → run, so a retried create doesn't start a second
    /// run. In memory for now: it does not survive a restart.
    idempotency: Mutex<HashMap<String, RunId>>,
    /// Runs this process is driving, so a resume doesn't start a second
    /// worker in the same process (the lease would refuse it anyway).
    active: Mutex<HashSet<RunId>>,
}

impl<S: EventStore + 'static> ApiState<S> {
    pub fn new(
        store: Arc<Notifying<S>>,
        workers: WorkerFactory<S>,
        token: impl Into<String>,
        default_agent: AgentSpec,
    ) -> Arc<Self> {
        Arc::new(Self {
            store,
            workers,
            token: token.into(),
            default_agent,
            idempotency: Mutex::new(HashMap::new()),
            active: Mutex::new(HashSet::new()),
        })
    }

    /// Drives a run in the background until it finishes, needs attention,
    /// or fails. A lost lease is retried a few times: on one machine that
    /// means the lease expired (for example the laptop slept).
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
        });
    }
}

pub fn router<S: EventStore + 'static>(state: Arc<ApiState<S>>) -> Router {
    let authenticated = Router::new()
        .route("/v1/runs", post(create_run::<S>))
        .route("/v1/runs/{id}", get(get_run::<S>))
        .route("/v1/runs/{id}/events", get(list_events::<S>))
        .route("/v1/runs/{id}/events/stream", get(stream_events::<S>))
        .route("/v1/runs/{id}/resume", post(resume_run::<S>))
        .route_layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_token::<S>,
        ));
    Router::new()
        .route(
            "/v1/health",
            get(|| async { Json(json!({ "status": "ok" })) }),
        )
        .merge(authenticated)
        .with_state(state)
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
}

#[derive(Serialize)]
struct RunView {
    id: RunId,
    /// The `seq` the next event will get.
    next_seq: u64,
    status: RunStatus,
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
        return Ok((StatusCode::OK, Json(view(&state, run_id).await?)));
    }
    let spec = RunSpec {
        task: body.task,
        agent: body.agent.unwrap_or_else(|| state.default_agent.clone()),
    };
    let spec = serde_json::to_value(&spec).map_err(|e| ApiError::Internal(e.to_string()))?;
    let run_id = state.store.create_run(spec).await?;
    if let Some(key) = key {
        lock(&state.idempotency).insert(key, run_id);
    }
    state.spawn_drive(run_id);
    Ok((StatusCode::CREATED, Json(view(&state, run_id).await?)))
}

async fn get_run<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
) -> Result<Json<RunView>, ApiError> {
    Ok(Json(view(&state, parse_run(&id)?).await?))
}

async fn resume_run<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<RunView>), ApiError> {
    let run_id = parse_run(&id)?;
    let current = view(&state, run_id).await?;
    if current.status != RunStatus::Open {
        return Err(ApiError::Conflict("only open runs can be resumed".into()));
    }
    state.spawn_drive(run_id);
    Ok((StatusCode::ACCEPTED, Json(current)))
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

async fn view<S: EventStore>(state: &ApiState<S>, run_id: RunId) -> Result<RunView, ApiError> {
    let log = state.store.read(run_id, Seq(0)).await?;
    Ok(RunView {
        id: run_id,
        next_seq: log.len() as u64,
        status: run_status(&log),
    })
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
    Internal(String),
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
