# Invariants of the event log

`shiioo-core` holds the rules every backend and every runtime must obey. This page states them precisely, points at the code that enforces each, and ends with the checklist for adding a payload variant. The product-level view is in [`docs/design/event-log.md`](../../../docs/design/event-log.md); decisions in [ADR-0001](../../../docs/adr/0001-event-log-is-the-only-state.md) and [ADR-0002](../../../docs/adr/0002-leases-with-fencing.md).

## The event

| Field | Type | Rule |
|---|---|---|
| `run_id` | `RunId` (UUID v4) | Every event of a log carries the same run id. |
| `seq` | `Seq(u64)` | Contiguous from 0. `Seq(0)` is always `RunCreated`. Also the SSE event id. |
| `epoch` | `Epoch(u64)` | The lease epoch the event was written under. `Epoch(0)` means "before any lease" (`RunCreated` only). Never decreases along a log. |
| `recorded_at` | `Timestamp` (ms) | From the store's `Clock` (the database clock on Postgres). Informational; not part of any rule. |
| `schema` | `u16` | `SCHEMA_VERSION` of the writer (1 today). Readers reject newer schemas (`PayloadError::NewerSchema`). |
| `prev_hash` | `Option<EventHash>` | `None` at `Seq(0)`, otherwise the previous event's `hash`. |
| `hash` | `EventHash` (SHA-256) | See below. |
| `payload` | `EventPayload` | The decoded `Payload` plus the exact JSON text it was hashed over. |

### Hashing (`event.rs`)

```
SHA-256( "shiioo/event/v1\0"
       ‖ run_id (16 bytes) ‖ seq (u64 BE) ‖ epoch (u64 BE) ‖ recorded_at (u64 BE) ‖ schema (u16 BE)
       ‖ 0x00                    if prev_hash is None
       ‖ 0x01 ‖ prev_hash        otherwise
       ‖ payload bytes exactly as stored )
```

The hash covers the payload's **stored bytes**, not a re-encoding. `EventPayload::encode` serializes a `Payload` once, at seal time; from then on backends store, return and verify those bytes (`EventPayload::raw`). `EventPayload::from_raw(schema, raw)` decodes without changing them. Consequence: an old event verifies forever, regardless of how later builds would encode it, and any change to the bytes, even whitespace, is `ChainError::Tampered`.

### `verify_chain` (`chain.rs`)

Given a complete log from `Seq(0)`, in order:

| Error | Condition |
|---|---|
| `WrongRun` | an event's `run_id` differs from the first event's |
| `Gap` | `seq` is not the event's index |
| `BrokenLink` | `prev_hash` is not the previous event's `hash` |
| `EpochRegressed` | `epoch` is lower than the previous event's |
| `Tampered` | the recomputed hash differs from `hash` |

Verification needs the whole prefix; a slice starting after `Seq(0)` fails with `Gap`.

## The head (`head.rs`)

`RunHead` is the only place the write rules live. Every backend loads the head under its own lock (a mutex, a redb write transaction, a Postgres `FOR UPDATE`), calls one `RunHead` method, and persists the returned events and the updated head atomically. Backends never re-implement a check.

| Method | Rule |
|---|---|
| `create(run_id, spec, now)` | New head; seals `RunCreated` at `Seq(0)`, `Epoch(0)`. |
| `grant_lease(worker, ttl, now)` | Refused on a closed run (`RunClosed`). Refused while another worker's lease is unexpired (`LeaseHeld`). Otherwise a new lease with `epoch = current + 1`, **even for the same worker**, and a `LeaseAcquired` event. |
| `check_lease(lease, now)` | `Fenced` if the lease's epoch is not the current one (or the run id differs); `LeaseExpired` if the current lease has run out. An expired lease keeps the epoch until someone takes over, so a stale holder stays fenced. |
| `renew_lease(lease, ttl, now)` | `check_lease`, then a later expiry with the same epoch. Writes no event. |
| `append(lease, expected_next, payloads, now)` | `validate_batch` (no `RunCreated`/`LeaseAcquired`; a terminal event only last), `check_lease`, `RunClosed` if closed, `SeqConflict` if `expected_next != next_seq`; then seals all payloads or none. |
| `append_external(payloads, now)` | Non-empty; every payload `is_external`; not closed; sealed under the **current** epoch without any lease. |

`SeqConflict` is not an error to a lease holder: it means someone appended external input; the holder re-reads from its known length and retries (`shiioo-runtime`'s `Session::append`).

## Payload classes (`event.rs`)

| Class | Variants | Written by |
|---|---|---|
| Store-only | `RunCreated`, `LeaseAcquired` | the store itself; refused in any `append` |
| Execution | `ModelResponseRecorded`, `ToolIntent`, `ToolResult`, `ToolOutcomeUnknown`, `HarnessSessionStarted`, `HarnessResult`, `CapacityObserved`, `RunCompleted`, `RunFailed`, `RunCancelled` | the lease holder, through `append` |
| External (`is_external`) | `UserMessage`, `PolicyDecision`, `ApprovalRequested`, `ApprovalDecided`, `CancelRequested`, `InterruptRequested`, `MailSent` | the server, through `append_external`, no lease |
| Terminal (`is_terminal`) | `RunCompleted`, `RunFailed`, `RunCancelled` | after one of these nothing is appended, and no lease is granted |

## Time (`clock.rs`)

`Clock` is a trait so tests and the simulator control time (`ManualClock`); production uses `SystemClock`, and the Postgres backend reads the database clock inside the transaction so that skew between workers cannot extend a lease.

## Adding a payload variant

1. Add it to `Payload` in `event.rs`. Field types must be `serde` round-trippable; `serde_json::Value` is fine for provider-shaped data.
2. Decide its class: add it to `is_external` if the server writes it without a lease; to `is_terminal` if it closes the run. Everything else is an execution event written by the lease holder.
3. Add a row to the catalogue in [`docs/design/event-log.md`](../../../docs/design/event-log.md): fields, writer, meaning.
4. Teach the printers: `shiioo_api::describe` (also used by the CLI), `shiioo_sim::compact`, and `describe()` in `crates/shiioo-api/ui/app.js`. Both Rust matches are exhaustive, so the compiler finds them.
5. If the runtime must react to it, do so in `shiioo-runtime/src/worker.rs` (for example `cancel_requested`) or `status.rs` (`run_status`, `open_approval`).
6. `SCHEMA_VERSION` stays 1: adding a variant keeps every stored event decodable. Bump it only when an existing variant's bytes must be read differently, and add an upcaster in `EventPayload::from_raw` that produces the typed value without touching the raw bytes.
