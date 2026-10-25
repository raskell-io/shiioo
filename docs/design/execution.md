# Execution

How runs move from created to finished: run states, the scheduler, the worker loop, leases, tool dispatch, recovery, cancellation, subagents, limits and failure handling. The event format and storage rules are in [event-log.md](event-log.md). Built today (`crates/shiioo-runtime`): the store rules (leases, fencing, append, `dangling_tool_calls`), the native worker loop, the harness loop (Claude Code, Codex), lease heartbeat and loss handling, recovery on the local machine, cancellation at step boundaries, turn and time limits. Planned: a scheduler with parking, subagents, budgets beyond time, and policy checks inside the native loop (today only harness runs go through the rules, via their hook). The multi-process parts (queue tables, gateways) are *(deferred, [ADR-0010](../adr/0010-single-binary-server-and-nodes.md))* until there is more than one machine.

## 1. Run states *(partly built)*

A run's state is not stored anywhere; it is a fold over its log. Today the fold is `shiioo_runtime::status::run_status` and yields `Open | WaitingApproval | NeedsAttention | Completed | Failed | Cancelled` (used by the API, the dashboard and the bot). The finer states below (Pending, Parked) arrive with the scheduler.

```
                 ┌───────────── input / approval / tool / timer ─────────────┐
                 ▼                                                            │
Pending ──► Running ──► Waiting(input | approval | tool | timer) ──► Parked ──┘
                 │                                │
                 └──────────────┬─────────────────┘
                                ▼
                 Completed | Failed | Cancelled   (terminal)
```

| State | Derived from | Lease |
|---|---|---|
| Pending | `RunCreated` / `ForkedFrom` and no `LeaseAcquired` yet | none |
| Running | latest `LeaseAcquired` or `RunWoken`, no wait recorded since | held |
| Waiting(x) | the run needs `x` before it can continue, worker still holds the lease (short waits, such as a tool in flight) | held |
| Parked | `RunParked { waiting_for }` is the latest lifecycle event | none |
| Completed / Failed / Cancelled | `RunCompleted` / `RunFailed` / `RunCancelled` | none, run closed |

**Waiting versus Parked.** A worker waits while holding the lease when the wait is expected to be short: a tool call in flight, a model call. Anything that waits on a person or a long timer (user input, approval, `sleep 1h`, a child run) is parked: the worker writes `RunParked`, stops renewing, and moves on. A parked run costs no worker time and no lease traffic, so thousands of idle conversations are free.

*(planned)* The fold moves into `shiioo-core` as a pure function over events, `RunState::fold(&[Event])`, used by workers, the scheduler and projections alike, and property-tested.

## 2. The scheduler *(planned)*

Today there is no scheduler: the server starts a background worker for a run when it is created or resumed (`shiioo-api::ApiState::spawn_drive`, one task per run, retried on a lost lease), and `shiioo run` drives a run in the foreground. A run that stops without finishing waits for `POST /v1/runs/{id}/resume` or `shiioo resume`.

The scheduler decides **which runs are runnable** and hands them to workers. A run is runnable when it is not terminal, not leased, and has unprocessed input:

| Trigger | Becomes runnable when |
|---|---|
| New run | `RunCreated` or `ForkedFrom` written |
| User input | `UserMessage` after the last `RunParked { input }` |
| Approval | `ApprovalDecided` for the pending call |
| Child finished | `ChildRunFinished` for the child the parent waits on |
| Timer | `wake_at` of a `RunParked { timer }` has passed |
| Tool result | a long-running tool's result arrives for a parked run |
| Recovery | a lease expired on a non-terminal, non-parked run (its worker died) |

**Several machines (Postgres)** *(deferred, ADR-0010)*. A `run_queue` table holds one row per runnable run:

```sql
run_queue (run_id uuid PK, project_id, ready_at, reason, attempts)
```

Writers that make a run runnable (`append_external`, a child finishing, a parked timer) insert into `run_queue` in the same transaction, then `NOTIFY shiioo_ready`. Workers claim work with:

```sql
DELETE FROM run_queue
WHERE run_id = (SELECT run_id FROM run_queue
                WHERE ready_at <= now()
                ORDER BY ready_at
                FOR UPDATE SKIP LOCKED LIMIT 1)
RETURNING run_id;
```

then call `acquire_lease`. If the lease fails (`LeaseHeld`, `RunClosed`), the claim was stale and the worker drops it. A sweeper re-enqueues runs whose leases expired without a terminal or parked event (the recovery case). The queue is an optimization; correctness comes from the lease. A run enqueued twice is harmless: the second `acquire_lease` fails or finds nothing to do.

**On-device (redb), the product.** The same logic with an in-process priority queue and a `tokio::sync::Notify`. On startup the scheduler rebuilds the queue by scanning run heads, since the queue itself isn't persisted.

**Timers.** Parked runs with `wake_at` are kept in a timer wheel (on-device) or as `run_queue` rows with a future `ready_at` (several machines).

## 3. The worker loop *(native and harness loops built; `ModelRequested`, policy and budgets in the native loop, deltas and parking planned)*

One iteration handles one runnable run until it parks, finishes, or loses its lease. Target shape:

```
claim run ──► acquire_lease ──► read log ──► fold state ──► recover dangling tool calls
   ┌──────────────────────────────────────────────────────────────────────────┘
   ▼
 turn:
   build context (instructions, history, tool schemas, memory, budget left)
   check budget                                   (deny → RunFailed or park for approval)
   append ModelRequested { digests }
   call provider ── stream deltas to subscribers (ephemeral, not events)
   append ModelResponseRecorded + BudgetCharged
   no tool calls?  → append RunCompleted, or RunParked { input } for a conversation
   for each tool call:
       evaluate policy → append PolicyDecision
         deny             → tool result is the denial; the model sees it next turn
         require approval → append ApprovalRequested, RunParked { approval }; stop
       check budget
       append ToolIntent { class }
       dispatch (server → node, or in-worker for remote tools)
       append ToolResult (+ BudgetCharged)
   loop to next turn
```

**Today** (`Session::run` in `shiioo-runtime/src/worker.rs`): the native loop builds the transcript from the log, calls the provider, appends `ModelResponseRecorded`, then per tool call appends `ToolIntent`, executes on the `LocalMachine` and appends `ToolResult`, until a response without tool calls completes the run or `max_turns` fails it. There is no `ModelRequested`, no policy check and no budget in the native loop yet. The harness loop (`Session::run_harness`) starts the harness through a `HarnessDriver`, records what it prints, and lets the harness's hook route every tool call through the server's rules and the owner ([governance.md](governance.md)).

Rules:

- **Write before acting.** The intent is durable before dispatch; the model response is durable before any tool runs. A crash at any point leaves a log that says exactly what was and wasn't done.
- **On takeover, finish the last response before calling the model.** After recovering dangling intents, the worker acts on the latest recorded model response: tool calls it requested that have no intent yet are run, and a response with no tool calls completes the run. Without this, a crash between recording a response and recording its first intent would silently drop those tool calls (the crash tests caught exactly this).
- **Every append presents the lease and `expected_next`.** A `Fenced` or `LeaseExpired` error means another worker owns the run: stop at once, drop all in-flight work, write nothing. A `SeqConflict` with a longer log means the owner appended external input (a decision, a cancellation): re-read the tail and retry, up to four times (`Session::append`, `catch_up`).
- **Parallel tool calls** *(planned)*. When a model response contains several calls, intents are appended as one batch, then dispatched concurrently, and results appended as they arrive. Order of results in the log is arrival order; each result names its `call_id`. Today the native loop runs calls one after another.
- **Turn and time limits** *(built)*. `AgentSpec.max_turns` fails a native run and is passed to harnesses; `HarnessSpec.max_minutes` fails a harness run whose total time across resumes exceeds it (the deadline counts from the run's first event). Both append `RunFailed { reason }`.

## 4. Leases in the worker *(built)*

- Default TTL **30 s**, renewed every **TTL / 3** (10 s) by a background task (`Heartbeat` in `worker.rs`) that stops when the session ends.
- If a renewal fails, the heartbeat flags the lease as lost (a `watch` channel): the harness loop kills its process at once and returns `LostLease`; a native loop finds out at its next write, which fails with `Fenced` or `LeaseExpired`. *(planned)* A `CancellationToken` so in-flight model calls are dropped immediately too.
- The worker never trusts its own clock for expiry; the store decides ([ADR-0002](../adr/0002-leases-with-fencing.md)). A worker that is paused by GC or a VM freeze and wakes up after expiry gets `Fenced` or `LeaseExpired` on its first write and stops.
- *(planned)* Long tool calls (a 20-minute build) do not hold the worker: the worker records the intent, dispatches, and parks with `RunParked { tool }`. The node reports the result to the server, which enqueues the run. Today the worker waits, renewing its lease.

## 5. Tool dispatch *(built for the local machine; remote nodes planned)*

Tool calls run on a Machine: today the in-process `LocalMachine`, later a remote `shiioo node` (earlier drafts called it a daemon). Tools that need no machine (HTTP APIs, remote MCP servers) run in the worker. Details: [tools-and-machines.md](tools-and-machines.md).

- Each dispatch carries the `call_id` and the lease epoch. The machine keeps a **local journal** of `call_id → started / finished(outcome)`, written (fsynced) before it starts the tool and after it finishes. A re-sent `call_id` returns the recorded outcome instead of running the tool again, and an invocation from an older epoch than the run has seen is `Fenced` (`LocalMachine::execute`).
- Pure and idempotent tools may be retried freely on transport errors. Side-effecting tools are sent once; a transport error after sending leads to reconciliation (§6), not a retry.
- *(planned)* Tool results above the blob threshold are uploaded by the node and returned as a `BlobRef`; today outputs are truncated to head and tail.

## 6. Recovery after takeover *(built for the local machine)*

When a worker acquires a lease, it reads the log and calls `dangling_tool_calls`, which returns, in log order:

- `Recovery::Rerun { call_id, class }` for `Pure` and `Idempotent` intents with no outcome: dispatch again (idempotent calls reuse their key).
- `Recovery::MarkUnknown { call_id }` for `SideEffecting` intents with no outcome.

For `MarkUnknown`, the worker first **asks the node**: "what happened to `call_id`?"

| Node answers | Worker writes |
|---|---|
| finished, here is the outcome | `ToolResult` with that outcome (`ToolReconciled { source: node }` recorded alongside, *planned*) |
| started, still running | waits or parks with `RunParked { tool }` |
| never received | safe to dispatch now: the call never left the server |
| unreachable / journal lost | `ToolOutcomeUnknown`, then route to the tool's reconcile hook or a human |

**Today** (`Session::recover`, `LocalMachine`), every dangling intent consults the machine's journal, whatever its class: finished → its journaled `ToolResult`; never received → dispatched now (the machine's epoch fence refuses the old worker's late copy); started but not finished → pure and idempotent calls run again, side-effecting ones get `ToolOutcomeUnknown` and the run stops with `NeedsAttention`. On a single machine "started" always means the machine stopped mid-call, so there is no "still running" case yet, and the result comes from the journal directly as a `ToolResult` rather than with `ToolReconciled`.

A run with an unresolved `ToolOutcomeUnknown` does not continue on its own (`RunStatus::NeedsAttention`). *(planned)* The reconcile hook (for example "check whether the branch was pushed") or a person resolves it with `ToolReconciled`, and the run becomes runnable again.

Harness runs recover differently: Shiioo did not run their tools, so dangling intents are observations. The next lease holder resumes the harness's own session (`--resume` / `exec resume`) with a continuation prompt, and the harness sorts out its interrupted call itself; after five such starts the run fails ([runtimes-and-providers.md](runtimes-and-providers.md)).

## 7. Cancellation *(built at step boundaries)*

- The server appends `CancelRequested` (an external write, no lease needed): `POST /v1/runs/{id}/cancel`, `/cancel` in Telegram, the dashboard's Cancel button.
- The worker checks for it when it starts a session, at every native turn, and after every harness output line (`Session::cancel_if_requested`); it then kills a harness process and appends `RunCancelled`. A harness deep in one long command is stopped when that command ends, not instantly.
- *(planned)* A `tokio_util::sync::CancellationToken` per run so in-flight model calls and tool executions stop immediately; child runs get child tokens, so cancelling a parent cancels its subtree.
- A run that nobody is driving is cancelled by whichever worker picks it up next; it appends `RunCancelled` without running a turn.

## 8. Subagents *(planned)*

- A parent spawns a child by appending `ChildRunStarted { child_run_id }` and creating the child run, whose `RunCreated.spec` names the parent run and the parent's call.
- The child's permissions are a subset of the parent's: same or fewer tools, machines, budget ([governance.md](governance.md)). *(deferred, ADR-0008)* Biscuit attenuation from [ADR-0005](../adr/0005-cedar-policies-and-biscuit-credentials.md) returns if several owners share an office.
- The child's budget is reserved from the parent's when the child starts and returned unused when it ends.
- The parent parks with `RunParked { waiting_for: child }` (a kind of `tool` wait), or keeps working if it spawned the child asynchronously.
- When the child reaches a terminal event, the scheduler appends `ChildRunFinished` to the parent and enqueues it.
- Depth and fan-out limits per agent prevent runaway recursion.

## 9. Limits and backpressure *(planned)*

| Limit | Enforced by | On breach |
|---|---|---|
| Turns per run *(built)* | worker (`max_turns`) | `RunFailed` |
| Time per harness run *(built)* | worker (`max_minutes`, across resumes) | `RunFailed` |
| Budget (tokens, cost) per run or employee *(planned)* | worker, before every model and tool call | park for approval or fail, per policy |
| Concurrent running runs in the office *(planned)* | scheduler | run stays queued |
| Subscription windows per provider *(built)* | the employee broker, when a task is assigned (`shiioo-api::employees`) | the next employee takes it, or the task is refused with the reasons |
| Subscriber buffer *(planned)* | bounded channel per SSE subscriber | slow subscriber dropped, resumes from `seq` |
| Node in-flight calls *(planned)* | node semaphore | server queues, then rejects with a retryable error |

Every channel between tasks is bounded. Nothing in the system queues work in an unbounded in-memory buffer.

## 10. Failure modes

| Failure | What happens | Why it's safe |
|---|---|---|
| Worker crashes during a model call | Lease expires (≤ 30 s); the next worker finds no response recorded and calls the model again | No tool ran; a repeated model call costs tokens, nothing else |
| Worker crashes after `ToolIntent`, before dispatch | Next worker's recovery asks the machine, which never received it, and dispatches | The machine's journal confirms nothing ran |
| Worker crashes during a tool call | The machine finishes and journals the outcome; next worker reconciles from the journal. If the machine died too, `ToolOutcomeUnknown` for side-effecting calls | Outcome recorded once, tool ran once, never a blind retry |
| Worker crashes during a harness run | The run stays open; the next worker resumes the harness's session with `--resume` / `exec resume` | The harness's own transcript holds its state; Shiioo's log holds everything it printed |
| Worker loses its lease during a harness run | The heartbeat flags the loss; the harness process is killed at once | The new owner's harness is the only one acting |
| Worker pauses (GC, VM freeze) past its lease | Another worker takes over; the paused one gets `Fenced` on its next write and stops | Fencing ([ADR-0002](../adr/0002-leases-with-fencing.md)) |
| Server restarts | Runs lose their workers; `shiioo up` resumes them on the next drive exactly as after a crash. Pending approvals time out and deny; their request stays in the log | Nothing is acted on without a durable record |
| Node disconnects mid-call *(planned)* | Tool keeps running locally; result is delivered on reconnect | Journal plus `call_id` dedup |
| Node machine dies *(planned)* | Reconciliation finds the journal unreachable → `ToolOutcomeUnknown` → hook or human | Never a blind retry |
| Database failover (Postgres, deferred) | In-flight transactions fail; workers retry their append with the same `expected_next` | A committed append makes the retry fail with `SeqConflict`; the worker re-reads and continues |
| Clock skew between machines | No effect on leases: expiry uses the database clock | Worker clocks aren't consulted |
| Duplicate delivery (queue, notify, retry) | Second `acquire_lease` fails, or append gets `SeqConflict`, or the machine dedups by `call_id` | Every path is idempotent by `seq` or `call_id` |
| Disk full / store unavailable | Appends fail; worker stops the turn and releases nothing it doesn't own | Nothing is acted on without a durable record |

## 11. Simulation testing *(single-process harness built; distributed harness planned)*

### Built: `shiioo-sim`

`crates/shiioo-sim` drives the real runtime (`Worker`, `LocalMachine` with a file journal, `MemoryStore` or `RedbStore`) through seeded scenarios. Each seed picks a script of 0–5 tool-calling turns (1–3 calls each, mixing side-effecting, pure and idempotent tools) followed by a final answer, then drives the run with a sequence of worker lives, each a single-threaded Tokio runtime so scheduling is reproducible:

- a life may crash at any `CrashPoint` (the dead worker keeps its lease; the clock then advances past the TTL);
- two workers may race for the same run (memory store only; tools yield mid-call so the rival is polled while the lease is held, and must get `LeaseHeld`);
- the machine may be restarted, reopening its journal from disk;
- a worker that crashed right after recording an intent is treated as stalled: after another worker has taken over, its invocation is replayed against the machine and must be `Fenced` without running the tool.

Invariants checked after every life and at the end: `verify_chain` passes and epochs never decrease; one `LeaseAcquired` event per lease the workers actually acquired; at most one `ToolResult` per call (unless it resolves a `ToolOutcomeUnknown`); every side-effecting call executed at most once, as counted by the tool; the run either completes with the scripted answer, every call having exactly one result and every side-effecting call having run exactly once, or stops with `NeedsAttention` only when a crash hit a side-effecting call mid-execution and the machine's journal says it started but never finished.

`cargo test -p shiioo-sim` runs 200 seeds; `SHIIOO_SIM_SEEDS=N` runs more, and `SHIIOO_SIM_SEED=n … -- --nocapture` reproduces one. A failure prints the seed, the reproduction command, the scenario and the log. As a check on the harness itself, making recovery rerun side-effecting calls that started but never finished makes about 5% of seeds fail with a double charge.

### Planned: distributed simulation

Once there are separate processes (server and remote nodes), the harness moves onto `turmoil`/`madsim` to add what a single process can't express: network partitions between components, message delay and duplication between server and node, clock skew between hosts, store write failures and slow disks, Postgres failover. Additional invariants then:

1. No event was accepted from a lease that wasn't current (cross-checked against the simulator's own record of grants).
2. Every `ToolIntent` eventually has exactly one outcome: `ToolResult`, `ToolOutcomeUnknown` followed by a resolution, or the run is terminal.
3. Every run whose inputs stop eventually reaches a terminal or parked state (liveness, with a bounded number of faults).
4. A subscriber that reconnects with `Last-Event-ID` receives every event exactly once, in order.
