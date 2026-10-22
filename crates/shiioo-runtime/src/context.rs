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
    for ev in log {
        match &*ev.payload {
            Payload::ModelResponseRecorded { response, .. } => t.turns.push(Turn {
                assistant: response["content"].as_array().cloned().unwrap_or_default(),
                results: Vec::new(),
            }),
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
    t
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
