# Event log

The per-run, append-only event log is Shiioo's only state ([ADR-0001](../adr/0001-event-log-is-the-only-state.md)). This document defines the event format, the hash chain, the payload catalogue, the storage backends, blobs, schema evolution, archival and how logs are read and streamed. How workers use the log to execute runs is in [execution.md](execution.md).

## 1. Event format *(built)*

`shiioo_core::Event`:

```rust
pub struct Event {
    pub run_id: RunId,              // UUID v4
    pub seq: Seq,                   // u64, contiguous from 0 within the run
    pub epoch: Epoch,               // u64, lease epoch the event was written under
    pub recorded_at: Timestamp,     // u64, Unix milliseconds, from the store's clock
    pub schema: u16,                // payload schema version (SCHEMA_VERSION = 1)
    pub prev_hash: Option<EventHash>, // None only at seq 0
    pub hash: EventHash,            // SHA-256, hex in JSON
    pub payload: Payload,           // tagged enum, `"type": "snake_case_name"`
}
```

| Field | Rule |
|---|---|
| `seq` | Assigned by the store. Event `n` is the `n`th event of the run. No gaps, no reuse. Also the SSE event id. |
| `epoch` | `Epoch(0)` only for `RunCreated`. Every other event carries the epoch of the lease it was written under. Never decreases along the log. |
| `recorded_at` | The store's clock at append time, not the worker's. On Postgres it is the database clock. |
| `schema` | The payload schema version the writer used. See §6. |
| `prev_hash` | The `hash` of event `seq - 1`. |

### Hash construction *(built)*

`hash = SHA-256` over, in order:

```
"shiioo/event/v1\0"            domain separator
run_id                          16 bytes (UUID)
seq, epoch, recorded_at         u64 big-endian each
schema                          u16 big-endian
0x00 | 0x01 ‖ prev_hash         presence flag, then 32 bytes if present
serde_json bytes of payload
```

The hash covers the payload's **stored bytes**, not a re-encoding of the decoded value. `Event::payload` is an `EventPayload`, which carries the exact JSON text (`raw()`) next to the decoded `Payload` (via `Deref`). New events are encoded once, at seal time, with serde (struct fields in declaration order, `Value` objects as sorted maps, `float_roundtrip` on); from then on only those bytes are stored, hashed and verified.

`verify_chain(&[Event])` checks, for a full log from `Seq(0)`: one run id, contiguous `seq`, correct `prev_hash` links, non-decreasing epochs, and that every recomputed hash matches. Errors: `ChainError::{WrongRun, Gap, BrokenLink, Tampered, EpochRegressed}`.

Because verification hashes stored bytes, it never depends on the current build's encoder: bytes in a non-canonical form (an older writer, another language) still verify, and any change to them, even whitespace only, is `Tampered` (see §6).

## 2. Write rules *(built)*

All rules live in `shiioo_core::RunHead`, so every backend enforces the same ones. A backend loads the head under a lock, calls one method, and persists the returned events plus the new head atomically.

```rust
pub struct RunHead {
    pub run_id: RunId,
    pub next_seq: Seq,
    pub last_hash: Option<EventHash>,
    pub closed: bool,
    pub lease: Option<Lease>, // latest lease granted; its epoch stays current after expiry
}
```

| Operation | Checks | Writes | Errors |
|---|---|---|---|
| `create_run(spec)` | — | `RunCreated` at seq 0, epoch 0 | — |
| `acquire_lease(run, worker, ttl)` | run open; no other worker holds an unexpired lease | `LeaseAcquired` at the new epoch (`current + 1`, always new, even for the same worker id) | `RunNotFound`, `RunClosed`, `LeaseHeld` |
| `renew_lease(lease, ttl)` | presented epoch is current and unexpired | head only (no event) | `Fenced`, `LeaseExpired` |
| `append(lease, expected_next, payloads)` | batch valid; lease current and unexpired; run open; `expected_next == next_seq` | the batch, all or nothing | `InvalidPayload`, `Fenced`, `LeaseExpired`, `RunClosed`, `SeqConflict` |
| `append_external(run, payloads)` | every payload `is_external()`; run open | the batch at the current epoch, no lease needed | `InvalidPayload`, `RunClosed` |
| `read(run, from)` | — | — | `RunNotFound` |
| `list_runs()` | — | — | — (every run id, unordered) |

Batch validation: workers may not append `RunCreated` or `LeaseAcquired` (store-written), and a terminal event must be last in its batch. A terminal event closes the run; later appends and lease requests get `RunClosed`.

Renewals are not logged: they carry no information a reader needs, and logging them would grow every long run by one event per 10 s. Takeovers are logged (`LeaseAcquired`) because they explain epoch changes. Lease semantics are in [ADR-0002](../adr/0002-leases-with-fencing.md) and [execution.md §4](execution.md#4-leases-in-the-worker-built).

## 3. Payload catalogue

`Payload` is `#[serde(tag = "type", rename_all = "snake_case")]` (`crates/shiioo-core/src/event.rs`). "Writer" is the component that appends it; the store rejects writes from anyone without the current lease, except the store itself and the server's external input through `append_external` (see below).

### Built

| Variant | Fields | Writer | Meaning |
|---|---|---|---|
| `RunCreated` | `spec: Value` | store | Seq 0. The run's input: agent id, task, parent link, options. |
| `LeaseAcquired` | `worker, expires_at` | store | A worker took the run; starts epoch `n`. |
| `ModelResponseRecorded` | `model, response: Value, context_seq?` | worker | The complete model response, recorded before any tool in it runs. `context_seq` (native loop) is the log length its request was built from: a message at or after it was not seen by this response, even if recorded before it. |
| `ToolIntent` | `call_id, tool, class: ToolClass, input` | worker | About to dispatch a tool call. `class` is `Pure`, `Idempotent { key }` or `SideEffecting`. |
| `ToolResult` | `call_id, output, is_error` | worker | The tool's outcome. |
| `ToolOutcomeUnknown` | `call_id` | worker | A side-effecting call may or may not have happened. Blocks the run until reconciled. |
| `HarnessSessionStarted` | `harness, version, session_id, cwd, model` | worker | A harness started or resumed a session; its steps follow as the events above ([runtimes-and-providers.md](runtimes-and-providers.md)). |
| `HarnessResult` | `harness, session_id, subtype, is_error, cost_usd, usage, permission_denials` | worker | The harness's own summary, before the terminal event. |
| `CapacityObserved` | `provider, windows` | worker | A provider's rate-limit windows, as seen by a harness. Input for capacity routing. |
| `UserMessage` | `author, content` | server (external) | Input from a person or integration. The native loop shows it to the model and wakes a parked run with it; `POST /v1/runs/{run}/messages` sends it. |
| `RunParked` | `waiting_for: input, wake_at?` | worker | The worker has nothing to do until a message arrives. Appending it ends the writer's lease (`RunHead::append`), so a parked run holds no lease; it must be last in its batch. `timer` with `wake_at` is planned. |
| `RunWoken` | `reason: input` | worker | The first event a worker writes after taking a parked run again. |
| `PolicyDecision` | `call_id?, tool, decision: allow\|deny\|ask, rule?, reason` | server (external, `shiioo-api::ApiState::permission`) | The owner's rules' verdict on a tool call ([governance.md](governance.md)). |
| `ApprovalRequested` | `approval_id, call_id?, tool, input, summary, expires_at?` | server (external) | The owner has been asked; the tool call waits. |
| `ApprovalDecided` | `approval_id, decision: allow\|deny, by, note?` | server (external) | The owner's answer, or `by: timeout`. |
| `CancelRequested` | `by, reason` | server (external) | The worker stops at its next step and writes `RunCancelled`. |
| `TakeoverRequested` | `by` | server (external) | The owner wants the keyboard: a worker driving the run stops its harness and parks with `waiting_for: takeover`. |
| `TakeoverStarted` | `by` | takeover (lease) | The owner is at the harness's own terminal; the takeover holds the lease. |
| `TakeoverEnded` | `reason, session?` | takeover (lease), or the next worker | `handed_back`, `detached`, `exited`, `cancelled`, `office_restarted`, `failed: …`; `session` is what the next process resumes. What happened at the keyboard is imported before it. |
| `InterruptRequested` | `by` | server (external) | Follows a `UserMessage` sent with "Now": the worker stops the harness process at work and resumes its session with the message. Ignored when no process is running. |
| `RunCompleted` | `output` | worker | Terminal. |
| `RunFailed` | `reason` | worker | Terminal. |
| `RunCancelled` | `reason` | worker | Terminal. |

### Planned

| Variant | Fields (sketch) | Writer | Meaning |
|---|---|---|---|
| `ModelRequested` | `provider, model, params_digest, context_digest, turn` | worker | Digest of exactly what was sent. Replay matches on it ([fork-and-replay.md](fork-and-replay.md)). |
| `BudgetCharged` | `call_id?, tokens_in, tokens_out, cost_micros, budget_ids` | worker | Usage after each model or tool call. Today harness usage and cost arrive in `HarnessResult`; a time budget (`HarnessSpec.max_minutes`) fails the run with `RunFailed`. |
| `ChildRunStarted` | `child_run_id, agent, task_digest` | worker | Parent spawned a subagent. |
| `ChildRunFinished` | `child_run_id, outcome, output_ref` | scheduler | Copied into the parent when the child reaches a terminal event. |
| `SnapshotTaken` | `machine, snapshot_id, method, digest` | worker (from the node's reply) | Workspace snapshot at a turn boundary. |
| `ForkedFrom` | `parent_run_id, parent_seq, parent_hash, changes` | store (at create) | First own event of a forked run, instead of `RunCreated`. Written at `parent_seq + 1` with `prev_hash = parent_hash`, so seqs and the hash chain continue from the parent's prefix ([fork-and-replay.md](fork-and-replay.md)). |
| `ReplayOf` | `source_run_id, mode: strict\|live_on_divergence\|stop_on_divergence, changes` | store (at create) | Seq 0 of a replay run ([fork-and-replay.md](fork-and-replay.md)). |
| `ReplayDiverged` | `turn, expected_digest, actual_digest` | worker | A replayed turn no longer matches the recording. |
| `ToolReconciled` | `call_id, outcome, source: node\|hook\|human` | worker | Resolves a `ToolOutcomeUnknown`. |
| `BudgetWarning` / `BudgetExceeded` | `budget_id, used, limit` | worker | Soft limit crossed; hard limit hit (the run parks or fails, per budget) ([governance.md](governance.md)). |
| `ConfigUpdated` | `kind, from_version, to_version` | worker | A change marked `applies_to_running` took effect mid-run ([governance.md](governance.md)). |
| `SecretResolved` | `call_id, secret_name` | worker | A tool call was given a secret. Name only, never the value ([tools-and-machines.md](tools-and-machines.md)). |
| `SecretRedacted` | `call_id, count` | worker | Secret values were removed from a tool output before it was recorded ([security.md](security.md)). |
| `ContextCompacted` | `from_seq, to_seq, summary_ref` | worker | A summary that stands in for a range of events when building context; the originals stay ([runtimes-and-providers.md](runtimes-and-providers.md)). |
| `HarnessMessageRecorded` | `harness, content` | worker | Final message from an external harness run through ACP ([runtimes-and-providers.md](runtimes-and-providers.md)). The built adapters record harness messages as `ModelResponseRecorded` instead. |

*(planned)* Office-level events that are not part of a run (node enrolment and revocation such as `MachineRevoked`, config changes, signed checkpoints, archival) go to an office audit log with the same format and chain rules. (Earlier drafts said "project"; the single-owner product has one office, [ADR-0008](../adr/0008-personal-workforce-not-managed-cloud.md).)

**Writes by the server** *(built)*. `UserMessage`, `PolicyDecision`, `ApprovalRequested`, `ApprovalDecided`, `CancelRequested`, `InterruptRequested`, `TakeoverRequested` and `MailSent` (`Payload::is_external`) come from the server (the API, the Telegram bot, the intake), which does not hold the run's lease. They go through `EventStore::append_external(run, payloads)` (`RunHead::append_external`), which takes no lease, accepts only external variants, refuses closed runs, and serializes with the run's head lock like any other write; the events carry the current epoch. The lease holder's next append then gets `SeqConflict`, re-reads the tail and retries (`Session::append` in `shiioo-runtime/src/worker.rs`, up to four times), which is also how it learns of a cancellation or a decision. Workers stay the only writers of execution events.

## 4. Blobs *(planned)*

Events stay small. Anything over ~64 KiB (tool outputs, file contents, attachments, model responses with images) goes to the blob store, and the event carries a reference:

```rust
pub struct BlobRef {
    pub sha256: [u8; 32],   // content address
    pub size: u64,
    pub media_type: String,
}
```

- Blobs are written **before** the event that references them. A blob without an event is garbage, collected later; an event without its blob cannot happen.
- Because `sha256` is inside the hashed payload, the hash chain covers blob content transitively. Anyone verifying the log can fetch and check each blob.
- `shiioo-blob` wraps `object_store` (S3, R2, GCS, Azure, local disk). On-device, blobs go to a directory next to the redb file.
- Blobs live beside the office's data; today large tool outputs are truncated to head and tail (`shiioo-runtime/src/tools.rs`) instead of stored.

## 5. Backends

### redb *(built, default)*

A single file, no server ([ADR-0003](../adr/0003-redb-default-postgres-for-clusters.md)).

| Table | Key | Value |
|---|---|---|
| `run_heads` | run id (16 bytes) | JSON-encoded `RunHead` |
| `events` | `(run id, seq: u64)` | JSON `StoredEvent { epoch, recorded_at, schema, prev_hash, hash, payload: RawValue }` |

- Every operation is one redb write transaction, run on Tokio's blocking pool (`spawn_blocking`): load head, apply the `RunHead` rule, insert events, write head, commit. Errors drop the transaction.
- The `(run, seq)` key makes `read(run, from)` a range scan.
- `payload` is kept as `RawValue`, the exact JSON text that was hashed.
- Inserting over an existing key aborts the transaction, a backstop for the `seq` rule.
- **One writer per file**: writes to different runs queue behind each other. Readers never block. Fine for one office on one machine, which is the product ([ADR-0010](../adr/0010-single-binary-server-and-nodes.md)).
- Lease expiry uses the local clock; on one machine there is no skew to worry about.
- `RunHead` is serialized as JSON, so its field names are part of the file format. Renaming a field needs a migration.

### Postgres *(built, `postgres` feature; no further investment, ADR-0010)*

```sql
runs   (run_id uuid PK, next_seq, last_hash bytea, closed,
        lease_worker, lease_epoch, lease_expires_at_ms, created_at)
events (run_id, seq, epoch, recorded_at_ms, schema, prev_hash, hash,
        payload json,               -- json, not jsonb: keeps the hashed text
        PRIMARY KEY (run_id, seq))
```

- Each write is a transaction that locks the run's head row (`SELECT … FOR UPDATE`). Writes to one run serialize; different runs proceed in parallel.
- Lease expiry and `recorded_at` use the database clock (`clock_timestamp()`), so skew between worker machines can't extend a lease.
- A trigger rejects `UPDATE`, `DELETE` and `TRUNCATE` on `events`.
- Check constraints mirror the core rules (`prev_hash` present iff `seq > 0`, hashes are 32 bytes, lease fields consistent).
- *(deferred, ADR-0010)* Batch inserts with `UNNEST`; partitioning of `events`; `LISTEN/NOTIFY` wakeups. The backend stays behind the feature flag and the conformance suite, nothing more.

### Memory *(built)*

`MemoryStore`: one mutex over a map of runs. The reference implementation and the backend for simulation tests.

### Conformance *(built)*

`crates/shiioo-store/tests/common` holds the suite every backend runs: chain validity, lease blocking, fencing after takeover, expiry without takeover, renewal, sequence conflicts, concurrent appends to the same slot, crash recovery of tool calls, closed runs, failed batches writing nothing, forged store events, unknown runs, offset reads. redb adds a reopen test; Postgres adds the append-only trigger test.

### Future: Turso

Turso, the Rust rewrite of SQLite, would give one embedded engine with concurrent writers (MVCC). It was beta (v0.7) in mid-2026. Revisit when it's stable; it would slot in behind the same trait and suite.

## 6. Schema evolution *(planned)*

- Every event records the `schema` version its writer used. Stored bytes are **never rewritten**: the hash covers them.
- Readers verify the chain first, then **upcast** old payloads to the current types with pure functions, one step per version (`v1 → v2 → …`).
- Adding a payload variant or an optional field does not need a version bump. Renaming, removing or changing the meaning of a field does.
- A reader that meets a newer `schema` than it knows refuses the run instead of guessing. Rolling upgrades therefore upgrade readers (api, workers) before any writer emits the new version; see [deployment.md](deployment.md).

## 7. Reading and streaming

- `read(run, from: Seq)` returns events with `seq >= from` in order *(built)*.
- *(built)* Wake-ups: the `Notifying<S>` wrapper (`crates/shiioo-store/src/notify.rs`) announces every successful write to a run through a per-run `tokio::sync::watch` channel. Announcements are hints only: a stream always reads the events themselves, so a missed or coalesced hint delays nothing but a wake-up. *(deferred, ADR-0010)* `LISTEN/NOTIFY` for a Postgres office.
- *(built)* The server serves SSE with `id: <seq>` (`GET /v1/runs/{id}/events/stream`): it subscribes, reads from `from`, and alternates reading and waiting, with a 2 s re-read as a backstop. A reconnecting client sends `Last-Event-ID` and gets everything after it, so no output is ever missed ([api.md](api.md), [ADR-0004](../adr/0004-rest-and-sse-api.md)).
- *(planned)* Token-by-token model output streamed as **ephemeral deltas**, which are not events and carry no `seq`. The complete response is recorded as `ModelResponseRecorded`. A client that reconnects mid-response sees the final event, not the deltas it missed.

## 8. Projections *(planned; two folds exist)*

Today the run list is `list_runs()` plus a read of each log (`shiioo-api::ApiState::list_runs`), and the capacity view is an in-memory fold of `CapacityObserved` events (`shiioo-api::CapacityTracker`, seeded from the logs at startup). Both are rebuildable from the logs, which is the point. Proper projections: run lists, cost per agent, full-text search and dashboards are **projections**: tables fed by consuming events, keyed by `(run_id, seq)` so they are idempotent. They are never a source of truth. Any projection can be dropped and rebuilt from the logs, and a bug in one can never corrupt a run.

Each projection stores the last `(run_id, seq)` it applied per run. On Postgres they live in the same database; on-device they live in separate redb tables.

## 9. Retention and archival *(planned)*

Finished runs stay hot for a configurable time (default 30 days), then are archived (nothing is archived today; the hot store keeps everything):

1. Verify the chain.
2. Write the run's events to object storage as a segment (JSONL, or Parquet for analytics), plus a **signed checkpoint**: run id, last `seq`, last `hash`, signer key id.
3. Write an `ArchivedRun` entry to the office's audit log (itself a hash-chained log).
4. Replace the run's hot events with a **tombstone head**: run id, final `seq` and `hash`, archive location, checkpoint signature.

Archival is the **only** path that removes events from the hot store. It runs through a dedicated, audited code path (on Postgres, as a role allowed to bypass the append-only trigger for archived runs only). Reading an archived run fetches the segment and verifies it against the tombstone. Legal holds block archival.

## 10. Open questions

- The office audit log (rules changes, node enrolment): reuse the run log machinery with an "office run", or a separate log type?
- Whether `ModelResponseRecorded` should always store the response as a blob, so the event stays fixed-size.
- Segment format for archives: JSONL is simpler to verify, Parquet is better for analytics. Possibly both.
