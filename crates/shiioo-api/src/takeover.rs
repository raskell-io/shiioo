//! Taking over a run from the desk (ADR-0013): the owner sits at the
//! harness's own terminal, in the run's worktree, resumed from its session,
//! with the rules still judging every tool call.
//!
//! `POST /v1/runs/{run}/takeover` starts (or joins) the takeover and returns
//! a single-use ticket; `GET /v1/runs/{run}/terminal?ticket=…` redeems it
//! for a WebSocket that carries the terminal: binary frames both ways for
//! its bytes, JSON text frames for control (`resize` from the client;
//! `status` and `ended` from the office). The process belongs to the
//! takeover, not to a connection: closing the tab detaches, and a takeover
//! nobody is attached to ends after `ApiOptions::takeover_detach`.
//! `POST /v1/runs/{run}/hand-back` ends it, leaving the run parked or, with
//! `resume`, waking it with the owner's note.

use crate::{ApiError, ApiState, RunView, lock};
use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use shiioo_core::{EventStore, Payload, RunId, Seq, StoreError};
use shiioo_runtime::harness::terminal::Terminal;
use shiioo_runtime::harness::transcript::Homes;
use shiioo_runtime::{RunOutcome, RunStatus, Runtime, TakeoverControl, WorkerError, open_approval};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, watch};

/// How long a ticket can wait before its WebSocket opens.
const TICKET_TTL: Duration = Duration::from_secs(30);

/// How long a viewer waits for the terminal (the worker stopping, the
/// harness starting) before giving up.
const READY_WAIT: Duration = Duration::from_secs(45);

/// Takeovers this office process is running, and their tickets.
#[derive(Default)]
pub struct Takeovers {
    live: Mutex<HashMap<RunId, Arc<Live>>>,
    tickets: Mutex<HashMap<String, (RunId, Instant)>>,
}

/// One takeover under way.
pub struct Live {
    terminal: watch::Receiver<Option<Arc<Terminal>>>,
    end: watch::Sender<Option<String>>,
    /// Set when it is over: why, or what went wrong.
    over: watch::Receiver<Option<String>>,
    attached: AtomicUsize,
    idle_since: Mutex<Option<Instant>>,
    /// The note to wake the run with after the hand back, when the owner
    /// asked it to carry on.
    resume: Mutex<Option<String>>,
}

impl Takeovers {
    pub fn is_live(&self, run: RunId) -> bool {
        lock(&self.live).contains_key(&run)
    }

    fn issue(&self, run: RunId) -> String {
        let ticket = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        let mut tickets = lock(&self.tickets);
        tickets.retain(|_, (_, at)| at.elapsed() < TICKET_TTL);
        tickets.insert(ticket.clone(), (run, Instant::now()));
        ticket
    }

    /// The takeover a ticket opens, once: a used, expired or foreign
    /// ticket opens nothing.
    fn redeem(&self, ticket: &str, run: RunId) -> Option<Arc<Live>> {
        let (for_run, at) = lock(&self.tickets).remove(ticket)?;
        if for_run != run || at.elapsed() >= TICKET_TTL {
            return None;
        }
        lock(&self.live).get(&run).cloned()
    }
}

/// What `POST /v1/runs/{run}/takeover` answers.
#[derive(Debug, Serialize, Deserialize)]
pub struct TakeoverTicket {
    /// Single use; redeem it at `terminal` within `expires_in` seconds.
    pub ticket: String,
    pub expires_in: u64,
    /// `/v1/runs/{run}/terminal?ticket=…`
    pub terminal: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct TakeoverBody {
    /// The terminal's size when the harness starts.
    #[serde(default)]
    pub cols: Option<u16>,
    #[serde(default)]
    pub rows: Option<u16>,
}

#[derive(Debug, Default, Deserialize)]
pub struct HandBackBody {
    /// Wake the run headless after the hand back (with `note`, or a plain
    /// "carry on"). Without it the run stays parked for the next message.
    #[serde(default)]
    pub resume: bool,
    #[serde(default)]
    pub note: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TicketQuery {
    pub ticket: String,
}

impl<S: EventStore + 'static> ApiState<S> {
    /// Starts a takeover of `run_id`, or joins the one under way, and
    /// returns a ticket for its terminal.
    pub async fn take_over(
        self: &Arc<Self>,
        run_id: RunId,
        size: (u16, u16),
    ) -> Result<TakeoverTicket, ApiError> {
        let ticket = |state: &Self| {
            let ticket = state.takeovers.issue(run_id);
            TakeoverTicket {
                terminal: format!("/v1/runs/{run_id}/terminal?ticket={ticket}"),
                ticket,
                expires_in: TICKET_TTL.as_secs(),
            }
        };
        if self.takeovers.is_live(run_id) {
            return Ok(ticket(self));
        }
        let log = self.store.read(run_id, Seq(0)).await?;
        if log.last().is_some_and(|e| e.payload.is_terminal()) {
            return Err(ApiError::Conflict(
                "the run has finished; follow it up, then take over the new run".into(),
            ));
        }
        let spec = crate::run_spec(&log)
            .ok_or_else(|| ApiError::Internal("the run has no spec".into()))?;
        let Runtime::Harness(harness) = &spec.runtime else {
            return Err(ApiError::Conflict(
                "only Claude Code and Codex runs can be taken over".into(),
            ));
        };
        if spec.machine.is_some() {
            return Err(ApiError::Conflict(
                "the run works on another machine; taking over there is not built yet".into(),
            ));
        }
        if harness.isolation.level() != "worktree" {
            return Err(ApiError::Conflict(format!(
                "the run is isolated in a {}; taking over there is not built yet",
                harness.isolation.level()
            )));
        }

        // The request the stopping process waited on is moot: the owner
        // is about to do it (or not) themselves.
        if let Some((approval_id, _)) = open_approval(&log) {
            self.decide(
                &approval_id,
                false,
                "owner",
                Some("taken over at the keyboard".into()),
                true,
            )
            .await
            .ok();
        }
        self.store
            .append_external(
                run_id,
                vec![Payload::TakeoverRequested { by: "owner".into() }],
            )
            .await?;

        let (term_tx, terminal) = watch::channel(None);
        let (end, end_rx) = watch::channel(None);
        let (over_tx, over) = watch::channel(None);
        let (ready, ready_rx) = oneshot::channel();
        let live = Arc::new(Live {
            terminal,
            end,
            over,
            attached: AtomicUsize::new(0),
            idle_since: Mutex::new(Some(Instant::now())),
            resume: Mutex::new(None),
        });
        lock(&self.takeovers.live).insert(run_id, Arc::clone(&live));
        tokio::spawn(async move {
            if let Ok(t) = ready_rx.await {
                term_tx.send_replace(Some(t));
            }
        });
        self.watch_detached(Arc::clone(&live));

        let state = Arc::clone(self);
        let homes = self
            .options
            .harness_homes
            .clone()
            .unwrap_or_else(Homes::from_env);
        tokio::spawn(async move {
            let worker = (state.workers)();
            let control = TakeoverControl {
                ready,
                end: end_rx,
                size,
                homes,
            };
            let outcome = worker.take_over(run_id, "owner", control).await;
            lock(&state.takeovers.live).remove(&run_id);
            let resume = lock(&live.resume).take();
            let how = match &outcome {
                Ok(RunOutcome::Parked(_)) => "ended".to_owned(),
                Ok(other) => format!("{other:?}"),
                Err(e) => format!("failed: {e}"),
            };
            if let Err(e) = &outcome {
                eprintln!("run {run_id}: takeover: {e}");
            }
            over_tx.send_replace(Some(how));
            // Carry on headless: the note is the message that wakes it.
            if let (Ok(RunOutcome::Parked(_)), Some(note)) = (&outcome, resume) {
                state.send_message(run_id, &note, false, None).await.ok();
            }
        });
        Ok(ticket(self))
    }

    /// Ends the takeover of `run_id` and returns the run as it is then.
    pub async fn hand_back(
        self: &Arc<Self>,
        run_id: RunId,
        resume: bool,
        note: Option<String>,
    ) -> Result<RunView, ApiError> {
        let Some(live) = lock(&self.takeovers.live).get(&run_id).cloned() else {
            return Err(ApiError::Conflict(
                "nobody is at the keyboard of this run".into(),
            ));
        };
        if resume {
            let note = note
                .map(|n| n.trim().to_owned())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| "Carry on with the task.".into());
            *lock(&live.resume) = Some(note);
        }
        live.end.send_replace(Some("handed_back".into()));
        let mut over = live.over.clone();
        let done = async { over.wait_for(Option::is_some).await.is_ok() };
        tokio::time::timeout(Duration::from_secs(20), done)
            .await
            .ok();
        // The note's message (if any) lands right after; give it a moment.
        tokio::time::sleep(Duration::from_millis(100)).await;
        self.run_view(run_id).await
    }

    /// Ends takeovers left by an office that stopped mid-way: each run is
    /// driven once its old lease has run out, which closes the takeover
    /// and parks it. Nothing runs headless.
    pub async fn end_stale_takeovers(self: &Arc<Self>) {
        let Ok(runs) = self.store.list_runs().await else {
            return;
        };
        for run_id in runs {
            if self.takeovers.is_live(run_id) {
                continue;
            }
            if let Ok(view) = self.run_view(run_id).await
                && matches!(view.status, RunStatus::TakenOver { .. })
            {
                let state = Arc::clone(self);
                tokio::spawn(async move {
                    // The dead office's lease lasts its TTL; try until then.
                    for _ in 0..24 {
                        if state.takeovers.is_live(run_id) {
                            return;
                        }
                        match (state.workers)().drive(run_id).await {
                            Err(WorkerError::Store(StoreError::LeaseHeld { .. })) => {
                                tokio::time::sleep(Duration::from_secs(5)).await;
                            }
                            _ => return,
                        }
                    }
                });
            }
        }
    }

    /// Ends a takeover nobody has been attached to for the detach timeout.
    fn watch_detached(&self, live: Arc<Live>) {
        let limit = self.options.takeover_detach;
        tokio::spawn(async move {
            let mut over = live.over.clone();
            let step = (limit / 4).clamp(Duration::from_millis(50), Duration::from_secs(15));
            loop {
                let done = async { over.wait_for(Option::is_some).await.is_ok() };
                tokio::select! {
                    true = done => return,
                    () = tokio::time::sleep(step) => {}
                }
                let idle = *lock(&live.idle_since);
                if live.attached.load(Ordering::SeqCst) == 0
                    && idle.is_some_and(|t| t.elapsed() >= limit)
                {
                    live.end.send_replace(Some("detached".into()));
                    return;
                }
            }
        });
    }
}

pub(crate) async fn post_takeover<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    body: Option<Json<TakeoverBody>>,
) -> Result<(StatusCode, Json<TakeoverTicket>), ApiError> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let size = (
        body.cols.unwrap_or(120).clamp(20, 500),
        body.rows.unwrap_or(32).clamp(5, 200),
    );
    let ticket = state.take_over(crate::parse_run(&id)?, size).await?;
    Ok((StatusCode::CREATED, Json(ticket)))
}

pub(crate) async fn post_hand_back<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    body: Option<Json<HandBackBody>>,
) -> Result<Json<RunView>, ApiError> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let view = state
        .hand_back(crate::parse_run(&id)?, body.resume, body.note)
        .await?;
    Ok(Json(view))
}

/// The terminal's WebSocket. Not behind the token middleware: browsers
/// can't set headers on a WebSocket, so the single-use ticket is the
/// credential.
pub(crate) async fn terminal_socket<S: EventStore + 'static>(
    State(state): State<Arc<ApiState<S>>>,
    Path(id): Path<String>,
    Query(q): Query<TicketQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    let Ok(run) = crate::parse_run(&id) else {
        return ApiError::Unauthorized.into_response();
    };
    let Some(live) = state.takeovers.redeem(&q.ticket, run) else {
        return ApiError::Unauthorized.into_response();
    };
    ws.on_upgrade(move |socket| serve(socket, live))
}

fn control(v: Value) -> Message {
    Message::Text(v.to_string().into())
}

async fn serve(mut socket: WebSocket, live: Arc<Live>) {
    live.attached.fetch_add(1, Ordering::SeqCst);
    *lock(&live.idle_since) = None;
    attached(&mut socket, &live).await;
    if live.attached.fetch_sub(1, Ordering::SeqCst) == 1 {
        *lock(&live.idle_since) = Some(Instant::now());
    }
}

async fn attached(socket: &mut WebSocket, live: &Live) {
    let mut over = live.over.clone();
    let ended = |reason: &str| control(json!({ "type": "ended", "reason": reason }));

    // Wait for the harness: a worker may still be stopping.
    let mut ready = live.terminal.clone();
    if ready.borrow().is_none() {
        socket
            .send(control(json!({ "type": "status", "text": "Stopping the agent and starting its terminal…" })))
            .await
            .ok();
    }
    // Owned values out of the watches: their guards can't cross an await.
    let up = async {
        ready
            .wait_for(Option::is_some)
            .await
            .ok()
            .and_then(|t| t.clone())
    };
    let gone = async {
        over.wait_for(Option::is_some)
            .await
            .ok()
            .and_then(|r| r.clone())
    };
    let terminal = tokio::select! {
        t = up => t,
        reason = gone => {
            socket.send(ended(&reason.unwrap_or_default())).await.ok();
            return;
        }
        () = tokio::time::sleep(READY_WAIT) => None,
    };
    let Some(terminal) = terminal else {
        socket.send(ended("the terminal did not start")).await.ok();
        return;
    };

    let (back, mut output) = terminal.attach();
    if !back.is_empty() && socket.send(Message::Binary(back.into())).await.is_err() {
        return;
    }
    loop {
        tokio::select! {
            chunk = output.recv() => match chunk {
                Ok(chunk) => {
                    if socket.send(Message::Binary(chunk.to_vec().into())).await.is_err() {
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            msg = socket.recv() => match msg {
                Some(Ok(Message::Binary(bytes))) => {
                    terminal.write(&bytes).await.ok();
                }
                Some(Ok(Message::Text(text))) => {
                    let v: Value = serde_json::from_str(text.as_str()).unwrap_or(Value::Null);
                    match v["type"].as_str() {
                        Some("resize") => {
                            let n = |k: &str| v[k].as_u64().and_then(|n| u16::try_from(n).ok());
                            if let (Some(cols), Some(rows)) = (n("cols"), n("rows")) {
                                terminal.resize(cols, rows).await;
                            }
                        }
                        Some("input") => {
                            if let Some(data) = v["data"].as_str() {
                                terminal.write(data.as_bytes()).await.ok();
                            }
                        }
                        _ => {}
                    }
                }
                Some(Ok(_)) => {}
                Some(Err(_)) | None => return,
            },
            reason = async { over.wait_for(Option::is_some).await.ok().and_then(|r| r.clone()) } => {
                let reason = reason.unwrap_or_default();
                // What the harness printed last goes out first.
                while let Ok(chunk) = output.try_recv() {
                    socket.send(Message::Binary(chunk.to_vec().into())).await.ok();
                }
                socket.send(ended(&reason)).await.ok();
                socket.send(Message::Close(None)).await.ok();
                return;
            }
        }
    }
}
