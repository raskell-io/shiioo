# The worker loop

`Worker::drive(run_id)` is the only entry point that does work. It takes a
lease on the run, reads the run's event log, and keeps appending events until
the run closes or the worker loses its lease. This document follows that path
through `src/worker.rs` and names every outcome.

## 1. Lease and heartbeat

1. `store.acquire_lease(run_id, worker_id, ttl_ms)` returns a lease with an
   `epoch`. Every event the worker appends carries this epoch; the store
   rejects appends from an older epoch (`Fenced`), which is what makes a
   stalled worker harmless after takeover (ADR-0002).
   - If the store answers `RunClosed`, the run already has a terminal event.
     The worker reads the log and returns the matching outcome
     (`closed_outcome`): `RunCompleted` → `Completed`, `RunFailed` →
     `Failed`, `RunCancelled` → `Cancelled`.
2. The log is read once. If an observer is attached, it sees the last event
   (the API uses this to seed capacity from old `CapacityObserved` events).
3. The `spec` is parsed from `RunCreated` (`parse_spec`). A spec that does not
   deserialise is `WorkerError::Spec`.
4. A `Heartbeat` task renews the lease every `ttl_ms / 3` (default TTL
   30 000 ms, so every 10 s). When a renewal fails it flips a `watch`
   channel to `lost = true`; the harness path selects on that channel and
   kills the child process.
5. `Session::run` does the work. `Fenced`, `LeaseExpired` and
   `MachineError::Fenced` are all turned into `Ok(RunOutcome::LostLease)`;
   callers (`shiioo-api::spawn_drive`, the CLI's `drive`) retry a few times
   and then give up.

## 2. Session::run

```
cancel requested?  ──yes──►  append RunCancelled  ──►  Cancelled
        │no
        ▼
parked?  ──yes──►  message since?  ──yes──►  append RunWoken { input }
        │no              │no
        │                ▼
        │           Parked (nothing written)
        ▼
spec.runtime is Harness?  ──yes──►  run_harness (section 4)
        │no
        ▼
unresolved ToolOutcomeUnknown?  ──yes──►  NeedsAttention(reason)
        │no
        ▼
recover()  ──►  act_on(latest ModelResponseRecorded)  ──►  native loop (section 3)
```

`cancel_if_requested` looks for a `CancelRequested` event with no
`RunCancelled` after it and appends `RunCancelled { reason: "cancelled by
<by>" }`, with `: <reason>` appended when the request carried one. The check
runs before any work and again at the top of every native turn, and the
harness path checks it after every line the harness prints. Cancellation is
therefore noticed at the run's next step, not instantly.

**Parked runs.** `drive` reads the log before taking the lease: a run whose
latest `RunParked` has no `RunWoken` after it and no `UserMessage` since
(`status::parked`, `status::wake_reason`) returns `Parked` without taking a
lease, so a spurious wake-up costs nothing. With a message, the worker takes
the lease (the parker's ended when it parked, see `RunHead::append`) and
writes `RunWoken { reason: "input" }` before anything else.

## 3. Native runtime

The native loop is Shiioo's own agent loop: model provider plus Shiioo tools
on the machine. One iteration:

1. Cancel check (above).
2. If the log already holds `max_turns` model responses, append
   `RunFailed { reason: "stopped after N turns (max_turns)" }`.
3. `context::transcript(spec, log)` rebuilds the conversation: each
   `ModelResponseRecorded` becomes an assistant turn, each `ToolResult`
   attaches to its call (a later result for the same call replaces an
   earlier one), a `ToolOutcomeUnknown` becomes an error block telling
   the model the machine stopped while that call was running, and each
   `UserMessage` becomes text in the next user turn, after the tool
   results (`Turn::messages`; before the first response it is added to the
   task). A message recorded before a response whose request did not
   include it (`context_seq` at or before it: sent while the model was
   busy) is placed after that response, not before it.
4. `provider.complete(ModelRequest { model, transcript, tools })`.
5. Append `ModelResponseRecorded { model, response, context_seq }`, where
   `context_seq` is the log length the transcript was built from.
   Crash point `AfterModelResponse`.
6. `act_on(response)`:
   - `StopReason::Refusal` → `RunFailed("the model declined the request")`.
   - `StopReason::MaxTokens` → `RunFailed("the response hit max_tokens before finishing")`.
   - No tool calls → `settle`: `RunCompleted { output: response.text() }`,
     or `RunParked { waiting_for: "input" }` when `spec.park_on_answer` (a
     conversation). `settle` first catches up, and if a `UserMessage` came
     after the latest response's `context_seq` it appends nothing and the
     loop runs another turn, so the model answers the owner first. The
     append names the seq it expects, so a message can't slip in between
     the check and the write.
   - Otherwise `call_tool` for every `ToolCall` that has no `ToolIntent`
     yet.

`call_tool`:

1. Append `ToolIntent { call_id, tool, input, class }`. The class comes from
   `Tool::class(input)`, or `Pure` when the tool is not available (so the
   error result below can be replayed freely). Crash point `AfterToolIntent`.
2. `dispatch`: if the tool is not in `agent.tools` or not in the machine's
   toolbox, the output is the error `tool '<name>' is not available to this
   agent`; else `machine.execute(Invocation { run_id, epoch, call_id, tool,
   input })`. Crash point `DuringToolExecution` makes the machine die after
   journaling `started` and before `finished`; `AfterToolExecuted` dies after
   the machine returned but before the result is logged.
3. Append `ToolResult { call_id, output, is_error }`. Crash point
   `AfterToolResult`.

### Recovery

`recover()` runs before the first turn of every `drive`. For every
`ToolIntent` without a `ToolResult` (`dangling_tool_calls`) it asks the
machine `status(run, epoch, call)`:

| Journal says | Class | Action |
| --- | --- | --- |
| `Finished(output)` | any | Append the journaled `ToolResult`; the tool does not run again. |
| `NotReceived` | any | Dispatch normally. |
| `Started` | `Pure` or `Idempotent` | `machine.rerun`: reset the journal entry and execute again. |
| `Started` | `SideEffecting` | Append `ToolOutcomeUnknown` and return `NeedsAttention("tool call <id> (<tool>) may or may not have run; check its effects, then record its result")`. |

`NeedsAttention` is not a terminal event. The run stays open; driving it
again returns the same answer until a `ToolResult` for that call is appended
by the owner (there is no API for that yet; the status exists so a human can
see it).

## 4. Harness runtime

`run_harness` spawns an external harness and turns its output into events.
Details of each adapter are in [harness-adapters.md](harness-adapters.md);
this is the loop around them.

1. **Working directory.** With `HarnessSpec.repo` set, `workspace::ensure(root,
   repo, run_id)` creates or reuses the worktree `<worktrees_root>/<run_id>`
   on branch `shiioo/<first 8 chars of run id>`. A machine without a
   worktrees root fails the run with `this machine has no worktree directory
   for repo runs`. Without a repo the harness runs in `machine.workspace()`.
2. **Start budget.** `unfinished_starts` = `HarnessSessionStarted` events
   since the last `HarnessResult` (resumes after a crash, not deliveries of
   messages). At `MAX_HARNESS_STARTS` (5) the run fails with `harness
   stopped without finishing N times`.
3. **Marker and prompt.** `SandboxStarted` is appended before every process
   starts, and marks what that process is told (`harness_prompt`): messages
   before it are in its prompt, messages after it are unread. First start:
   the task, plus any messages sent before it. After the session finished
   cleanly (a `HarnessResult` since its start): the owner's messages since
   the previous start. After a crash: `CONTINUE_PROMPT` ("The process
   running you was stopped before you finished. Check the current state of
   the workspace, then continue the task to completion."), followed by
   "While you were stopped, the owner wrote:" and the messages, if any.
   Every later start passes the last recorded session id, so the harness
   resumes its own conversation.
4. **Command.** `driver_for(spec, agent, cwd)` picks the adapter (Codex also
   gets its `--version` probed) and `driver.command(spec, agent, cwd, prompt,
   resume, hooks)` builds the `tokio::process::Command`. `hooks` is the
   `HookEndpoint` plus run id when the worker was built `with_hooks`.
5. **Deadline.** `max_minutes` counts from the first event's `recorded_at`
   across all starts; without it the deadline is a year away.
6. **Loop.** `select!` over the next stdout line, the deadline and the
   lease-lost watch:
   - line → `driver.interpret(line)` → append each payload; when the
     interpretation carries an outcome (`RunCompleted`/`RunFailed`) return
     it; then `cancel_if_requested` → kill and return `Cancelled`.
   - a successful finish is not appended directly: the `HarnessResult` is,
     then `settle` appends `RunCompleted` (or `RunParked { input }` when
     `park_on_answer`) only if no message came after this process's
     `SandboxStarted` (`harness_seen`). Otherwise the process is let exit,
     `run_harness` returns `None`, and `Session::run` calls it again: a new
     sandbox, a new marker, and the session resumed with the messages as
     its prompt.
   - deadline → kill, `RunFailed("time budget of N minutes exceeded")`.
   - lease lost → kill, `LostLease`.
7. **End of output without an outcome.** `HarnessError::ExitedEarly { status,
   stderr }` (stderr capped at 2000 chars) bubbles up as
   `WorkerError::Harness`. The run stays open; the next `drive` counts one
   more start and resumes.

Every stdout line the adapter understands becomes one or more events, so the
log is a faithful record of the harness session: `HarnessSessionStarted`,
`ModelResponseRecorded`, `ToolIntent`/`ToolResult` pairs, `CapacityObserved`,
`HarnessResult`, and the terminal `RunCompleted`/`RunFailed`.

## 5. Appending and the owner's events

`append(payload)` tries up to four times. On `SeqConflict { actual > next }`
it `catch_up`s (reads the log from its current length) and `absorb`s the new
events, passing each to the observer. This is how owner input written through
`append_external` (cancel requests, approval decisions, permission events
from the hook) lands in the worker's view without a restart. If the log
keeps moving for all four attempts the worker fails with
`WorkerError::Spec("the log kept changing while appending")`.

The permission events (`PermissionRequested`, `ApprovalRequested`,
`ApprovalDecided`, `PolicyDecision`) are appended by `shiioo-api`, not by
the worker. The worker never blocks on them; the harness process does, inside
its hook.

## 6. Outcomes at a glance

| `RunOutcome` | Terminal event | Reached by |
| --- | --- | --- |
| `Completed(output)` | `RunCompleted` | model answered without tool calls; harness result with no error |
| `Failed(reason)` | `RunFailed` | refusal, max_tokens, max_turns, harness error subtype, start budget, time budget |
| `Cancelled(reason)` | `RunCancelled` | a `CancelRequested` seen at the next step |
| `NeedsAttention(reason)` | none (run stays open) | a side-effecting tool whose outcome the journal cannot settle |
| `LostLease` | none | fence or expired lease; another worker owns the run |
| `Parked(waiting_for)` | none (`RunParked`; the run stays open) | a `park_on_answer` run answered; or `drive` on a parked run with no new message |

Crash injection (`Faults { crash_at: Some(CrashPoint) }`) exists only for
tests; `tests/crash_recovery.rs` kills a worker at every point and checks the
successor completes the run with each tool executed exactly once.

See also: [execution design](../../../docs/design/execution.md),
[event log design](../../../docs/design/event-log.md),
[ADR-0001](../../../docs/adr/0001-event-log-is-the-only-state.md),
[ADR-0002](../../../docs/adr/0002-leases-with-fencing.md).
