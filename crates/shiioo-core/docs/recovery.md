# Recovery rules

What happens to a tool call whose outcome was never recorded, for example because the worker died between dispatching it and writing `ToolResult`. The rules are pure functions in `recovery.rs`; the runtime applies them (`shiioo-runtime/src/worker.rs`, `Session::recover`) with help from the machine's call journal.

## Tool classes

Every `ToolIntent` carries a `ToolClass`, decided per call by the tool (or, for harness runs, by a table over the harness's tool names):

| Class | Meaning | After an unrecorded outcome |
|---|---|---|
| `Pure` | No effect outside the call (read a file, search, fetch). | Run again. |
| `Idempotent { key }` | Running again with the same key has the same effect as running once (write the same content to the same path). | Run again. |
| `SideEffecting` | Must not be repeated blindly (a shell command, a push, a payment, a sent message). | Never retried automatically. |

## `dangling_tool_calls`

Scans a log in order and keeps every `ToolIntent` whose `call_id` has neither a `ToolResult` nor a `ToolOutcomeUnknown` after it. For each, in log order:

- `Pure` or `Idempotent` → `Recovery::Rerun { call_id, class }`
- `SideEffecting` → `Recovery::MarkUnknown { call_id }`

The function is deterministic over the log; it knows nothing about machines.

## What the runtime does with each

On taking a lease, before anything else, the worker runs the recovery rules and asks the machine's journal (`LocalMachine::status`) about each dangling call:

| Journal says | Worker does |
|---|---|
| finished, here is the output | records that output as `ToolResult` (the call ran exactly once) |
| never received | dispatches it now; the machine's epoch fence refuses any late copy from the old worker |
| started, not finished, `Pure`/`Idempotent` | runs it again (`LocalMachine::rerun`) and records the result |
| started, not finished, `SideEffecting` | records `ToolOutcomeUnknown` and stops the run with `RunOutcome::NeedsAttention` |

A run with an unresolved `ToolOutcomeUnknown` does not continue on its own (`run_status` reports `NeedsAttention`). A later `ToolResult` for the same `call_id`, recorded by a person or a reconcile hook, resolves it; `ToolReconciled` as a distinct event is planned.

The crash tests (`crates/shiioo-runtime/tests/crash_recovery.rs`) kill the worker at every point of a turn and check that a side-effecting tool ran exactly once; the simulator (`crates/shiioo-sim`) does the same over thousands of seeded scenarios.

## Harness runs

For runs driven by Claude Code or Codex, `ToolIntent`/`ToolResult` are observations of what the harness did, not Shiioo's own dispatches. The worker does not run recovery over them; after a crash it resumes the harness's own session, and a tool call that was interrupted is the harness's to sort out. The classes are still recorded so the log reads the same way for both kinds of run.

## External input and leases

Recovery concerns execution events. The owner's decisions and cancellations are appended without a lease (`RunHead::append_external`), and a worker that finds its view stale (`SeqConflict`) catches up by reading the tail; see [invariants.md](invariants.md).
