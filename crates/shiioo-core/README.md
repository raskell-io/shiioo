# shiioo-core

The execution kernel: the event model and the rules every store backend and every runtime must obey. One run has one append-only, hash-chained event log; this crate defines the events, the per-run `RunHead` that decides whether a write is accepted, the lease and fencing rules, the chain verifier, and the recovery rules for tool calls whose outcome was never recorded. It does no I/O at all, so the same code runs under tests, under the deterministic simulation and in production.

## Where it sits

- Depends on: `serde`, `serde_json`, `sha2`, `hex`, `thiserror`, `uuid`. Nothing async, no network, no files.
- Depended on by: every other crate. `shiioo-store` implements `EventStore`; `shiioo-runtime` writes and folds events; `shiioo-api` reads them; `shiioo-sim` checks the invariants.

## Public API

| Item | What it is |
|---|---|
| `RunId`, `Seq`, `Epoch`, `Timestamp`, `WorkerId`, `CallId` | Newtype ids (`ids.rs`). `Seq` is the event's position (0-based), `Epoch` the fencing token, `Timestamp` Unix milliseconds. |
| `Clock`, `SystemClock`, `ManualClock` | Time source for lease expiry. `ManualClock` only moves when told to; clones share one time. |
| `Lease` | A worker's exclusive right to append to one run until `expires_at`, carrying its `epoch`. |
| `Payload` | The event kinds, `#[serde(tag = "type", rename_all = "snake_case")]`. `is_terminal()`, `is_external()`. |
| `ToolClass` | `Pure`, `Idempotent { key }`, `SideEffecting`: how a tool call behaves if its result is never recorded. |
| `EventPayload` | A `Payload` plus the exact JSON bytes it was hashed over. Derefs to `Payload`; `raw()` gives the bytes. |
| `Event` | `run_id, seq, epoch, recorded_at, schema, prev_hash, hash, payload`. `seal(...)` builds and hashes; `recompute_hash()`. |
| `EventHash`, `SCHEMA_VERSION`, `PayloadError` | SHA-256 digest (hex in JSON); the payload schema this build writes (1); decode errors. |
| `RunHead` | The per-run record every write is checked against: `next_seq`, `last_hash`, `closed`, `lease`. Methods `create`, `grant_lease`, `renew_lease`, `check_lease`, `append`, `append_external`, `current_epoch`. |
| `EventStore`, `StoreError` | The backend trait (`create_run`, `acquire_lease`, `renew_lease`, `append`, `append_external`, `read`, `list_runs`) and its errors (`RunNotFound`, `RunClosed`, `LeaseHeld`, `Fenced`, `LeaseExpired`, `SeqConflict`, `InvalidPayload`, `Backend`). |
| `verify_chain`, `ChainError` | Checks a full log: one run, contiguous seqs, linked hashes, non-decreasing epochs, unmodified bytes. |
| `dangling_tool_calls`, `Recovery` | Tool intents with no result: `Rerun` for pure and idempotent calls, `MarkUnknown` for side-effecting ones. |

## Using it

Backends never re-implement the rules. They load the head, call one `RunHead` method, and persist what comes back, in one transaction:

```rust
use shiioo_core::{Payload, RunHead, RunId, Seq, Timestamp};

let (mut head, created) = RunHead::create(RunId::new(), serde_json::json!({ "task": "x" }), Timestamp(1))?;
let (lease, acquired) = head.grant_lease("w1".into(), 30_000, Timestamp(2))?;
let events = head.append(
    &lease,
    Seq(2),
    vec![Payload::ModelResponseRecorded { model: "m".into(), response: serde_json::json!({}) }],
    Timestamp(3),
)?;
// persist `created`, `acquired`, `events` and `head`; then:
assert_eq!(shiioo_core::verify_chain(&[created, acquired, events[0].clone()]), Ok(()));
# Ok::<(), shiioo_core::StoreError>(())
```

(Taken from how `shiioo-store`'s memory backend uses the head; see `../shiioo-store/src/memory.rs`.)

## Testing

```sh
cargo test -p shiioo-core
```

Unit tests cover the chain verifier, including that hashing covers the stored bytes (a non-canonical but untouched payload verifies; a whitespace-only change is `Tampered`). The rules in `RunHead` are exercised by `shiioo-store`'s conformance suite against every backend, and by `shiioo-sim`.

## Invariants callers must respect

- Every append presents a lease and the `seq` it expects to write. `Fenced` or `LeaseExpired` means stop writing at once; `SeqConflict` means re-read the log and try again.
- Write before acting: the model response before any tool in it runs, the tool intent before dispatching it.
- Only `is_external()` payloads may be written without a lease (`append_external`); `RunCreated` and `LeaseAcquired` are written by the store, never by callers.
- A terminal payload must be the last in its batch, and nothing may follow it.
- Stored payload bytes are never rewritten. Upcasting older schemas produces the typed view only.

Details: the "Guarantees" section of the [architecture overview](../../docs/architecture.md) and the [event log design](../../docs/design/event-log.md).

## Further reading

- Maintainer notes in this crate: [docs/invariants.md](docs/invariants.md) (every rule, the hash construction, how to add a payload variant) and [docs/recovery.md](docs/recovery.md) (tool classes and what happens after an unrecorded outcome).
- [Architecture overview](../../docs/architecture.md), especially "The one rule" and "Guarantees".
- [Event log design](../../docs/design/event-log.md) and [execution design](../../docs/design/execution.md).
- [ADR-0001 The event log is the only state](../../docs/adr/0001-event-log-is-the-only-state.md), [ADR-0002 Leases with fencing](../../docs/adr/0002-leases-with-fencing.md).
