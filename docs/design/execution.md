# Execution

How runs move from created to finished: run states, the scheduler, the worker loop, leases, tool dispatch, recovery, cancellation, subagents, limits and failure handling. The event format and storage rules are in [event-log.md](event-log.md). Most of this document is *(planned)*: the store rules it relies on (leases, fencing, append, `dangling_tool_calls`) are *(built)*.

## 1. Run states *(planned)*

A run's state is not stored anywhere; it is a fold over its log.

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

The fold lives in `shiioo-core` as a pure function over events, `RunState::fold(&[Event])`. It is used by workers, the scheduler and projections alike, and property-tested.

## 2. The scheduler *(planned)*

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

**Cluster (Postgres).** A `run_queue` table holds one row per runnable run:

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

**On-device (redb).** The same logic with an in-process priority queue and a `tokio::sync::Notify`. On startup the scheduler rebuilds the queue by scanning run heads, since the queue itself isn't persisted.

**Timers.** Parked runs with `wake_at` are kept in a timer wheel (on-device) or as `run_queue` rows with a future `ready_at` (cluster).

## 3. The worker loop *(spike built in `shiioo-runtime`; policy, budgets and parking planned)*

One iteration handles one runnable run until it parks, finishes, or loses its lease.

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
       dispatch (gateway → daemon, or in-worker for remote tools)
       append ToolResult (+ BudgetCharged)
   loop to next turn
```

Rules:

- **Write before acting.** The intent is durable before dispatch; the model response is durable before any tool runs. A crash at any point leaves a log that says exactly what was and wasn't done.
- **On takeover, finish the last response before calling the model.** After recovering dangling intents, the worker acts on the latest recorded model response: tool calls it requested that have no intent yet are run, and a response with no tool calls completes the run. Without this, a crash between recording a response and recording its first intent would silently drop those tool calls (the crash tests caught exactly this).
- **Every append presents the lease and `expected_next`.** A `Fenced` or `LeaseExpired` error means another worker owns the run: stop at once, drop all in-flight work, write nothing.
- **Parallel tool calls.** When a model response contains several calls, intents are appended as one batch, then dispatched concurrently, and results appended as they arrive. Order of results in the log is arrival order; each result names its `call_id`.
- **Turn limits.** A max-turns limit and a wall-clock limit per run; hitting one appends `RunFailed { reason }` or parks for a human, per agent configuration.

## 4. Leases in the worker *(planned; store side built)*

- Default TTL **30 s**, renewed every **TTL / 3** (10 s) by a background task tied to the run's `CancellationToken`.
- If a renewal fails, or the renewal task can't run before the lease would expire (missed deadline), the worker **cancels the run's token immediately**: in-flight model calls are dropped, in-flight tool calls are abandoned (their outcome will be reconciled by the next holder), and no further writes are attempted.
- The worker never trusts its own clock for expiry; the store decides ([ADR-0002](../adr/0002-leases-with-fencing.md)). A worker that is paused by GC or a VM freeze and wakes up after expiry gets `Fenced` or `LeaseExpired` on its first write and stops.
- Long tool calls (a 20-minute build) do not hold the worker: the worker records the intent, dispatches, and parks with `RunParked { tool }`. The daemon reports the result through the gateway, which enqueues the run.

## 5. Tool dispatch *(planned)*

Tool calls run either on a Machine, through the gateway and its daemon, or in the worker for tools that need no machine (HTTP APIs, remote MCP servers). Details: [tools-and-machines.md](tools-and-machines.md).

- Each dispatch carries the `call_id`. The daemon keeps a **local journal** of `call_id → status, outcome`, written before it starts the tool and after it finishes. A re-sent `call_id` returns the recorded outcome instead of running the tool again.
- Pure and idempotent tools may be retried freely on transport errors. Side-effecting tools are sent once; a transport error after sending leads to reconciliation (§6), not a retry.
- Tool results above the blob threshold are uploaded by the daemon and returned as a `BlobRef`.

## 6. Recovery after takeover *(rules built; spike runtime built for the local machine)*

When a worker acquires a lease, it reads the log and calls `dangling_tool_calls`, which returns, in log order:

- `Recovery::Rerun { call_id, class }` for `Pure` and `Idempotent` intents with no outcome: dispatch again (idempotent calls reuse their key).
- `Recovery::MarkUnknown { call_id }` for `SideEffecting` intents with no outcome.

For `MarkUnknown`, the worker first **asks the daemon**: "what happened to `call_id`?"

| Daemon answers | Worker writes |
|---|---|
| finished, here is the outcome | `ToolResult` with that outcome (`ToolReconciled { source: daemon }` recorded alongside) |
| started, still running | waits or parks with `RunParked { tool }` |
| never received | safe to dispatch now: the call never left the gateway |
| unreachable / journal lost | `ToolOutcomeUnknown`, then route to the tool's reconcile hook or a human |

**In the spike** (`Worker::recover`, `LocalMachine`), every dangling intent consults the machine's journal, whatever its class: finished → its journaled `ToolResult`; never received → dispatched now (the machine's epoch fence refuses the old worker's late copy); started but not finished → pure and idempotent calls run again, side-effecting ones get `ToolOutcomeUnknown` and the run stops with `NeedsAttention`. On a single machine "started" always means the machine stopped mid-call, so there is no "still running" case yet, and the result comes from the journal directly as a `ToolResult` rather than with `ToolReconciled`.

A run with an unresolved `ToolOutcomeUnknown` does not continue on its own. The reconcile hook (for example "check whether the branch was pushed") or a person resolves it with `ToolReconciled`, and the run becomes runnable again.

## 7. Cancellation *(planned)*

- The api appends `CancelRequested` (an external write, no lease needed) and enqueues the run.
- The worker holding the run checks for it between steps and, for in-flight work, via a `tokio_util::sync::CancellationToken`. Each run has a token; child runs get child tokens, so cancelling a parent cancels its subtree.
- On cancel the worker: aborts the model call, asks daemons to stop running tools (best effort; their outcomes are still recorded if they finish), cancels child runs, appends `RunCancelled`.
- A parked run is cancelled by whichever worker picks it up next; it appends `RunCancelled` without running a turn.

## 8. Subagents *(planned)*

- A parent spawns a child by appending `ChildRunStarted { child_run_id }` and creating the child run, whose `RunCreated.spec` names the parent run and the parent's call.
- The child's credentials are a Biscuit token **attenuated** from the parent's: same or fewer tools, machines, budget ([governance.md](governance.md), [ADR-0005](../adr/0005-cedar-policies-and-biscuit-credentials.md)).
- The child's budget is reserved from the parent's when the child starts and returned unused when it ends.
- The parent parks with `RunParked { waiting_for: child }` (a kind of `tool` wait), or keeps working if it spawned the child asynchronously.
- When the child reaches a terminal event, the scheduler appends `ChildRunFinished` to the parent and enqueues it.
- Depth and fan-out limits per agent prevent runaway recursion.

## 9. Limits and backpressure *(planned)*

| Limit | Enforced by | On breach |
|---|---|---|
| Budget (tokens, cost, wall time) per run, agent, project | worker, before every model and tool call | park for approval or fail, per policy |
| Concurrent running runs per project | scheduler (claim query filters by project counts) | run stays queued |
| Concurrent model calls per provider key | capacity broker ([runtimes-and-providers.md](runtimes-and-providers.md)) | wait or fail over to another key or provider |
| Subscriber buffer | bounded channel per SSE subscriber | slow subscriber dropped, resumes from `seq` |
| Daemon in-flight calls | daemon semaphore | gateway queues, then rejects with a retryable error |

Every channel between tasks is bounded. Nothing in the system queues work in an unbounded in-memory buffer.

## 10. Failure modes

| Failure | What happens | Why it's safe |
|---|---|---|
| Worker crashes during a model call | Lease expires (≤ 30 s); next worker sees `ModelRequested` without a response and calls the model again | No tool ran; a repeated model call costs tokens, nothing else. Budget charges only recorded responses. |
| Worker crashes after `ToolIntent`, before dispatch | Next worker's recovery asks the daemon, which never received it, and dispatches | Daemon journal confirms nothing ran |
| Worker crashes during a tool call | Daemon finishes and journals the outcome; next worker reconciles from the journal | Outcome recorded once, tool ran once |
| Worker pauses (GC, VM freeze) past its lease | Another worker takes over; the paused one gets `Fenced` on its next write and stops | Fencing ([ADR-0002](../adr/0002-leases-with-fencing.md)) |
| Gateway crashes | Daemons reconnect to another gateway; workers' pending dispatches time out and go through reconciliation | Daemon journal survives; results are delivered by `call_id` |
| Daemon disconnects mid-call | Tool keeps running locally; result is delivered on reconnect | Journal plus `call_id` dedup |
| Daemon machine dies | Reconciliation finds the journal unreachable → `ToolOutcomeUnknown` → hook or human | Never a blind retry |
| Database failover (Postgres) | In-flight transactions fail; workers retry their append with the same `expected_next` | A committed append makes the retry fail with `SeqConflict`; the worker re-reads and continues |
| Clock skew between machines | No effect on leases: expiry uses the database clock | Worker clocks aren't consulted |
| Duplicate delivery (queue, notify, gateway retry) | Second `acquire_lease` fails, or append gets `SeqConflict`, or daemon dedups by `call_id` | Every path is idempotent by `seq` or `call_id` |
| Disk full / store unavailable | Appends fail; worker stops the turn and releases nothing it doesn't own | Nothing is acted on without a durable record |

## 11. Simulation testing *(planned)*

`shiioo-sim` runs whole deployments (api, scheduler, N workers, gateways, daemons, `MemoryStore` or Postgres) under `turmoil`/`madsim` with a seeded RNG. Each seed is a reproducible scenario; failing seeds become regression tests.

Faults injected: process kill at any await point, network partitions between any two components, message delay and duplication, clock jumps, store write failures, slow disks.

Invariants checked after every step and at the end:

1. Every log passes `verify_chain`.
2. No two events in a run share a `seq`; epochs never decrease.
3. No event was accepted from a lease that wasn't current (cross-checked against the simulator's own record of grants).
4. Every side-effecting tool, as observed by a fake daemon, executed **at most once** per `call_id`.
5. Every `ToolIntent` eventually has exactly one outcome: `ToolResult`, `ToolOutcomeUnknown` followed by `ToolReconciled`, or the run is terminal.
6. Every run whose inputs stop eventually reaches a terminal or parked state (liveness, with a bounded number of faults).
7. A subscriber that reconnects with `Last-Event-ID` receives every event exactly once, in order.

The kernel spike's exit test is the simplest instance: kill a worker mid-turn and check invariants 1–5.
