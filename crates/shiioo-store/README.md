# shiioo-store

Event store backends. Each one persists the per-run log and head defined by `shiioo-core` and implements its `EventStore` trait. None of them contains rules of its own: every write loads the run's `RunHead`, calls one method on it, and persists what comes back inside a single transaction. A shared conformance suite runs the same seventeen cases against every backend.

## Where it sits

- Depends on: `shiioo-core`, `serde`, `serde_json`, `uuid`, `tokio` (`sync` only, for the notifier), and optionally `redb` or `sqlx`.
- Depended on by: the `shiioo` binary (opens the store the server uses), `shiioo-runtime` (through the `EventStore` trait only), `shiioo-sim` (`MemoryStore` and `RedbStore`).

## Backends

| Type | Feature | Use | Clock |
|---|---|---|---|
| `MemoryStore<C>` | always | Reference implementation, tests, simulation. One `Mutex` around all runs; nothing is persisted. | Injected `C: Clock` |
| `RedbStore` | `redb` (default) | The embedded single-file backend for a Mac Studio or any single node. One writer at a time across the whole file, readers never block, every call runs on Tokio's blocking pool. | `SystemClock`, or injected via `with_clock` |
| `PostgresStore` | `postgres` | Cluster backend. Writers lock the run's row with `SELECT ... FOR UPDATE`; the log table is protected by append-only triggers. | Database clock unless injected |
| `Notifying<S>` | always | Wrapper that wakes in-process subscribers after each successful write to a run. | Delegates |

`MIGRATOR` (feature `postgres`) is the embedded `sqlx` migrator; `PostgresStore::migrate` applies it.

## Public API

| Item | Notes |
|---|---|
| `MemoryStore::new(clock)` | Generic over the clock; tests pass a `ManualClock`. |
| `RedbStore::open(path)`, `RedbStore::with_clock(path, clock)` | Create or open the file and the two tables. `RedbStore` is `Clone` (shared `Arc<Database>`). |
| `PostgresStore::new(pool)`, `::with_clock(pool, clock)`, `.migrate()` | Takes a `sqlx::PgPool`. Without an injected clock, `now()` is read from the database inside the write transaction. |
| `Notifying::new(inner)`, `.inner()`, `.subscribe(run_id) -> watch::Receiver<u64>` | The receiver's value is a write counter; only its change matters. |
| `impl EventStore for …` | All four types. |

## Using it

```rust
use shiioo_core::{EventStore, Payload, Seq};
use shiioo_store::RedbStore;

let store = RedbStore::open("shiioo.redb")?;
let run = store.create_run(serde_json::json!({ "goal": "demo" })).await?;
let lease = store.acquire_lease(run, "worker-1".into(), 30_000).await?;
store
    .append(
        &lease,
        Seq(2),
        vec![Payload::ModelResponseRecorded { model: "m".into(), response: serde_json::json!({}) }],
    )
    .await?;
let log = store.read(run, Seq(0)).await?;
assert_eq!(shiioo_core::verify_chain(&log), Ok(()));
```

`Seq(2)` because the store itself wrote `RunCreated` at 0 and `LeaseAcquired` at 1. The same shape appears in `tests/redb.rs` (`state_survives_reopen`).

## Testing

```sh
cargo test -p shiioo-store                               # memory + redb
SHIIOO_TEST_DATABASE_URL=postgres://… cargo test -p shiioo-store --features postgres
```

- `tests/memory.rs` and `tests/redb.rs` each call `conformance_suite!(setup)`. The redb file adds `state_survives_reopen`.
- `tests/postgres.rs` skips with a message when `SHIIOO_TEST_DATABASE_URL` is unset, and fails instead when `CI` is also set. It adds `events_table_rejects_updates_and_deletes`. The migration is applied to whatever database the URL names; use a scratch database.
- Each case gets its own store and `ManualClock` starting at `START_MS` with a `TTL` of 30 s, both from `tests/common/mod.rs`.

See [`docs/conformance.md`](docs/conformance.md) for what each case pins down.

## Invariants a backend must keep

- One transaction per write: load head, apply the `RunHead` rule, persist events and head, commit. A failed batch leaves nothing behind.
- Read the clock after taking the write lock, so a lease cannot be judged live by a stale timestamp.
- Store the payload's bytes exactly as sealed and hand them back unchanged (`json` column, raw JSON in redb). The hash covers those bytes.
- Never let a caller write `RunCreated` or `LeaseAcquired`; the head rejects them and the suite checks it.
- `read(run, from)` returns events in `seq` order starting at `from`; `list_runs` returns every run id ever created.

## Further reading

- [`docs/backends.md`](docs/backends.md): storage layout and trade-offs per backend.
- [`docs/conformance.md`](docs/conformance.md): the shared test suite.
- [Architecture overview](../../docs/architecture.md), [event log design](../../docs/design/event-log.md), [deployment design](../../docs/design/deployment.md).
- [ADR-0003 redb as the default store](../../docs/adr/0003-redb-default-postgres-for-clusters.md), [ADR-0010 Single binary, server and nodes](../../docs/adr/0010-single-binary-server-and-nodes.md).
