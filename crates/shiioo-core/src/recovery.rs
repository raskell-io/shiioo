use crate::event::{Event, Payload, ToolClass};
use crate::ids::CallId;

/// What a worker that takes over a run must do with a tool call whose
/// intent was recorded but whose outcome never was.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recovery {
    /// Pure or idempotent: run it again (idempotent calls reuse their key).
    Rerun { call_id: CallId, class: ToolClass },
    /// Side-effecting: record `ToolOutcomeUnknown` and reconcile or ask a
    /// human. Never retried automatically.
    MarkUnknown { call_id: CallId },
}

/// Finds tool intents with no matching result or unknown-outcome event, in
/// the order they were recorded.
pub fn dangling_tool_calls(events: &[Event]) -> Vec<Recovery> {
    let mut open: Vec<(CallId, ToolClass)> = Vec::new();
    for ev in events {
        match &*ev.payload {
            Payload::ToolIntent { call_id, class, .. } => {
                open.push((call_id.clone(), class.clone()));
            }
            Payload::ToolResult { call_id, .. } | Payload::ToolOutcomeUnknown { call_id } => {
                open.retain(|(id, _)| id != call_id);
            }
            _ => {}
        }
    }
    open.into_iter()
        .map(|(call_id, class)| match class {
            ToolClass::SideEffecting => Recovery::MarkUnknown { call_id },
            class => Recovery::Rerun { call_id, class },
        })
        .collect()
}
