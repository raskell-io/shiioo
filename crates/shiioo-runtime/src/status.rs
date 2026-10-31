use serde::Serialize;
use serde_json::Value;
use shiioo_core::{CallId, Event, Payload};

/// Where a run stands, folded from its log.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RunStatus {
    /// Not finished: being worked on, or waiting for a worker.
    Open,
    Completed {
        output: String,
    },
    Failed {
        reason: String,
    },
    /// Stopped until a person resolves something.
    NeedsAttention {
        reason: String,
    },
    /// Waiting for the owner to allow or deny a tool call.
    WaitingApproval {
        approval_id: String,
        summary: String,
    },
    Cancelled {
        reason: String,
    },
    /// The owner is at the keyboard (`TakeoverStarted` without
    /// `TakeoverEnded`). No headless worker drives it.
    TakenOver {
        by: String,
    },
    /// Holds no lease and waits for `waiting_for` (`input`: a message).
    /// A run that was parked and has since received input is `Open` again.
    Parked {
        waiting_for: String,
    },
}

pub fn run_status(log: &[Event]) -> RunStatus {
    match log.last().map(|e| &*e.payload) {
        Some(Payload::RunCompleted { output }) => RunStatus::Completed {
            output: as_text(output),
        },
        Some(Payload::RunFailed { reason }) => RunStatus::Failed {
            reason: reason.clone(),
        },
        Some(Payload::RunCancelled { reason }) => RunStatus::Cancelled {
            reason: reason.clone(),
        },
        _ => match (open_approval(log), unresolved_unknown(log)) {
            (Some((approval_id, summary)), _) => RunStatus::WaitingApproval {
                approval_id,
                summary,
            },
            (None, Some(reason)) => RunStatus::NeedsAttention { reason },
            (None, None) if let Some(by) = taken_over(log) => {
                RunStatus::TakenOver { by: by.to_owned() }
            }
            (None, None) => match parked(log) {
                // Parked only so a takeover can take the lease: not
                // waiting for the owner.
                Some((_, waiting_for))
                    if waiting_for != "takeover" && wake_reason(log).is_none() =>
                {
                    RunStatus::Parked {
                        waiting_for: waiting_for.to_owned(),
                    }
                }
                _ => RunStatus::Open,
            },
        },
    }
}

/// Who is at the keyboard, while a takeover is under way.
pub fn taken_over(log: &[Event]) -> Option<&str> {
    for ev in log.iter().rev() {
        match &*ev.payload {
            Payload::TakeoverEnded { .. } => return None,
            Payload::TakeoverStarted { by } => return Some(by),
            _ => {}
        }
    }
    None
}

/// A `TakeoverRequested` that no takeover has answered yet: the worker
/// should stop and park so the takeover can take the lease.
pub fn takeover_pending(log: &[Event]) -> bool {
    for ev in log.iter().rev() {
        match &*ev.payload {
            Payload::TakeoverStarted { .. } | Payload::TakeoverEnded { .. } => return false,
            Payload::TakeoverRequested { .. } => return true,
            _ => {}
        }
    }
    false
}

/// The index of the `RunParked` the run is parked at, and what it waits
/// for, unless a worker has written `RunWoken` since.
pub fn parked(log: &[Event]) -> Option<(usize, &str)> {
    for (i, ev) in log.iter().enumerate().rev() {
        match &*ev.payload {
            Payload::RunWoken { .. } => return None,
            Payload::RunParked { waiting_for, .. } => return Some((i, waiting_for)),
            _ => {}
        }
    }
    None
}

/// Why a parked run should be taken up again, if it should: `input` when
/// a message arrived after it parked, `cancel` when the owner cancelled it
/// (only a worker can close it). `None` for a run that is not parked.
pub fn wake_reason(log: &[Event]) -> Option<&'static str> {
    let (at, _) = parked(log)?;
    let since = &log[at..];
    if since
        .iter()
        .any(|e| matches!(*e.payload, Payload::CancelRequested { .. }))
    {
        return Some("cancel");
    }
    since
        .iter()
        .any(|e| matches!(*e.payload, Payload::UserMessage { .. }))
        .then_some("input")
}

/// The oldest approval request without a decision, if any.
pub fn open_approval(log: &[Event]) -> Option<(String, String)> {
    let mut open: Vec<(String, String)> = Vec::new();
    for ev in log {
        match &*ev.payload {
            Payload::ApprovalRequested {
                approval_id,
                summary,
                ..
            } => open.push((approval_id.clone(), summary.clone())),
            Payload::ApprovalDecided { approval_id, .. } => {
                open.retain(|(id, _)| id != approval_id);
            }
            _ => {}
        }
    }
    open.into_iter().next()
}

/// A side-effecting call marked unknown that nobody has resolved yet.
pub(crate) fn unresolved_unknown(log: &[Event]) -> Option<String> {
    let mut open: Vec<&CallId> = Vec::new();
    for ev in log {
        match &*ev.payload {
            Payload::ToolOutcomeUnknown { call_id } => open.push(call_id),
            Payload::ToolResult { call_id, .. } => open.retain(|id| *id != call_id),
            _ => {}
        }
    }
    open.first().map(|id| {
        format!(
            "tool call {} has an unknown outcome; check its effects, then record its result",
            id.0
        )
    })
}

pub(crate) fn as_text(v: &Value) -> String {
    v.as_str().map_or_else(|| v.to_string(), str::to_owned)
}
