use crate::model::{ToolResultBlock, Transcript, Turn};
use crate::spec::RunSpec;
use shiioo_core::{Event, Payload};

/// Folds a run's log into the conversation to send to the model.
pub(crate) fn transcript(spec: &RunSpec, log: &[Event]) -> Transcript {
    let mut t = Transcript {
        system: spec.agent.system.clone(),
        task: spec.task.clone(),
        turns: Vec::new(),
    };
    let late = late_messages(log);
    let mut deferred: Vec<(usize, String)> = Vec::new();
    for (i, ev) in log.iter().enumerate() {
        match &*ev.payload {
            Payload::ModelResponseRecorded { response, .. } => t.turns.push(Turn {
                assistant: response["content"].as_array().cloned().unwrap_or_default(),
                results: Vec::new(),
                messages: Vec::new(),
            }),
            Payload::UserMessage { author, content } => {
                let text = if author == "owner" {
                    content.clone()
                } else {
                    format!("{author}: {content}")
                };
                match late.get(&i) {
                    // Sent while the model was busy: it belongs after the
                    // response that did not see it.
                    Some(&turn) => deferred.push((turn, text)),
                    None => match t.turns.last_mut() {
                        Some(turn) => turn.messages.push(text),
                        // Before the first response, a message adds to the task.
                        None => {
                            t.task.push_str("\n\n");
                            t.task.push_str(&text);
                        }
                    },
                }
            }
            Payload::ToolResult {
                call_id,
                output,
                is_error,
            } => push_result(
                &mut t,
                ToolResultBlock {
                    call_id: call_id.0.clone(),
                    content: output
                        .as_str()
                        .map_or_else(|| output.to_string(), str::to_owned),
                    is_error: *is_error,
                },
            ),
            Payload::ToolOutcomeUnknown { call_id } => push_result(
                &mut t,
                ToolResultBlock {
                    call_id: call_id.0.clone(),
                    content: "The machine stopped while this call was running. Whether it took \
                              effect is unknown; check before repeating it."
                        .into(),
                    is_error: true,
                },
            ),
            _ => {}
        }
    }
    // They were sent before anything that arrived after that response.
    for (turn, text) in deferred.into_iter().rev() {
        if let Some(turn) = t.turns.get_mut(turn) {
            turn.messages.insert(0, text);
        }
    }
    t
}

/// Messages recorded before a response whose request did not include
/// them (`context_seq` at or before the message): log index → the index
/// of that response's turn.
fn late_messages(log: &[Event]) -> std::collections::HashMap<usize, usize> {
    let mut late = std::collections::HashMap::new();
    let mut turn = 0;
    let mut pending: Vec<usize> = Vec::new();
    for (i, ev) in log.iter().enumerate() {
        match &*ev.payload {
            Payload::UserMessage { .. } => pending.push(i),
            Payload::ModelResponseRecorded { context_seq, .. } => {
                let seen = context_seq.map_or(i, |s| s.0 as usize);
                for m in pending.drain(..) {
                    if m >= seen {
                        late.insert(m, turn);
                    }
                }
                turn += 1;
            }
            _ => {}
        }
    }
    late
}

/// A later result for the same call (for example a human resolving an
/// unknown outcome) replaces the earlier one.
fn push_result(t: &mut Transcript, block: ToolResultBlock) {
    if let Some(turn) = t.turns.last_mut() {
        match turn.results.iter_mut().find(|r| r.call_id == block.call_id) {
            Some(existing) => *existing = block,
            None => turn.results.push(block),
        }
    }
}
