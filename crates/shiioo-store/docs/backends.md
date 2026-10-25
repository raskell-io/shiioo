# Store backends

How each `EventStore` implementation lays out the log and where its guarantees come from. The rules themselves live in `shiioo_core::RunHead`; this document is about persistence only.

## The common write path

Every backend performs a write the same way:

1. Begin a transaction (or take the lock).
2. Load the run's `RunHead`; `RunNotFound` if there is none.
3. Read the clock. In Postgres this is the database's `now()`, so the order "lock, then time" is what makes lease expiry judgements consistent across workers. The redb backend mirrors the order on purpose.
4. Call one `RunHead` method (`grant_lease`, `renew_lease`, `append`, `append_external`). It returns the sealed events or an error.
5. Insert the events and overwrite the head. Commit. On any error the transaction is dropped and nothing was written.

`create_run` is the exception: it builds a fresh head with `RunHead::create` and persists head plus the `RunCreated` event.

## `MemoryStore<C>`

- `HashMap<RunId, (RunHead, Vec<Event>)>` behind one `std::sync::Mutex`. The lock is the transaction.
- The clock is whatever `C: Clock` the caller passes. Tests and the simulator use `ManualClock`.
- Exists to be the readable reference and the backend the simulator runs most seeds against (even seeds). Not a production store.

## `RedbStore`

Two tables in one redb file:

| Table | Key | Value |
|---|---|---|
| `run_heads` | run id, 16 bytes | JSON-encoded `RunHead` |
| `events` | `(run id, seq)` | JSON-encoded `StoredEvent { epoch, recorded_at, schema, prev_hash, hash, payload }` |

- `payload` is stored as `Box<RawValue>`, so the exact bytes that were hashed go in and come out. Reading decodes them with `EventPayload::from_raw(stored.schema, …)`.
- Keying events by `(run id, seq)` makes a run's log one contiguous key range; `read(run, from)` is a range scan.
- A duplicate `(run id, seq)` on insert is reported as `StoreError::Backend`. It cannot happen through `RunHead`, so seeing it means two heads diverged.
- redb has a single writer per file, so writes to different runs serialise through one lock. Readers run on snapshots and never wait. For one machine with a handful of concurrent runs this is far below any limit; for many nodes use Postgres.
- Every call is wrapped in `tokio::task::spawn_blocking`; the store is `Clone` and shares the `Database` through an `Arc`.
- `list_runs` scans the whole `run_heads` table. There is no secondary index yet; with thousands of finished runs this will show up in the dashboard's run list.
- Durability is redb's: a committed transaction survives a crash or power loss. `state_survives_reopen` checks that the lease epoch, and so fencing, survives a restart.

## `PostgresStore`

Schema in `migrations/0001_event_log.sql`:

| Table | Role |
|---|---|
| `runs` | One row per run: `next_seq`, `last_hash`, `closed`, `lease_worker`, `lease_epoch`, `lease_expires_at_ms`. Check constraints keep lease fields consistent (epoch 0 means no lease). |
| `events` | `(run_id, seq)` primary key, `epoch`, `recorded_at_ms`, `schema`, `prev_hash`, `hash`, `payload json`. `seq = 0` exactly when `prev_hash IS NULL`. |

- Writers lock the run's row with `SELECT ... FOR UPDATE`, so writes to one run are serialised while other runs proceed in parallel.
- `payload` is `json`, not `jsonb`: `jsonb` normalises key order and whitespace, which would break the hash.
- Two triggers on `events` raise an exception on `UPDATE`, `DELETE` and `TRUNCATE`. The log is append-only even for someone with a SQL prompt. `events_table_rejects_updates_and_deletes` checks this. Archiving finished runs will need its own explicit path *(planned)*.
- Time comes from the database unless a clock is injected (tests inject a `ManualClock`). Workers on machines with skewed clocks therefore cannot extend each other's leases.
- `MIGRATOR` embeds the migration directory; `PostgresStore::migrate` runs it. The test file applies it to the database named by `SHIIOO_TEST_DATABASE_URL`.
- Wake-ups for live streams would come from `LISTEN/NOTIFY` in a cluster *(planned)*; today `Notifying` covers the in-process case only.

## `Notifying<S>`

A wrapper, not a store of its own.

- Holds a `watch::Sender<u64>` per run that someone has subscribed to. After each successful `append` or `append_external` the counter is bumped.
- The value is a hint. Subscribers re-read the log themselves, so a coalesced or missed wake-up delays a stream but cannot lose or reorder events.
- Senders are dropped when the run finishes or when no receiver is left. A subscriber that comes back later simply subscribes again and starts from a fresh counter.
- `create_run`, `acquire_lease`, `renew_lease`, `read` and `list_runs` pass straight through.

## Choosing

| Situation | Backend |
|---|---|
| One machine, the usual case | `RedbStore` in the server's data directory. No service to run. |
| Several nodes sharing one log | `PostgresStore` (feature `postgres`). |
| Tests, simulation, examples | `MemoryStore<ManualClock>`. |

See [ADR-0003](../../../docs/adr/0003-redb-default-postgres-for-clusters.md) for why redb is the default and [deployment.md](../../../docs/design/deployment.md) for the node topology.
