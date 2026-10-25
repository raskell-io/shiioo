# The conformance suite

`tests/common/mod.rs` holds one generic test body per behaviour, written against `EventStore` and `ManualClock`. Each backend's test file provides a `setup()` that returns `Option<(store, clock)>` and calls `conformance_suite!(setup)`, which expands to one `#[tokio::test]` per case. Returning `None` from `setup` skips the whole suite, which is how the Postgres file behaves without a database URL.

The suite is the contract. A new backend passes it or it is not a backend; a change to a `RunHead` rule should come with a new or changed case here.

## Fixtures

| Constant | Value | Meaning |
|---|---|---|
| `TTL` | 30 000 ms | Lease length used throughout. |
| `START_MS` | 1 700 000 000 000 | Where every `ManualClock` starts. |

Helpers `model_step(n)` and `decision()` build a `ModelResponseRecorded` and a `PolicyDecision` payload.

## Cases

| Case | What it pins down |
|---|---|
| `log_is_a_valid_hash_chain` | After create, lease and two appends, `read(run, Seq(0))` returns four events that `verify_chain` accepts, with `RunCreated` at seq 0 and `LeaseAcquired` at seq 1. |
| `live_lease_blocks_other_workers` | A second worker gets `LeaseHeld` while the first lease is unexpired. |
| `stale_worker_is_fenced_after_takeover` | After the clock passes the TTL and another worker acquires, the old lease's `append` and `renew_lease` fail with `Fenced`; the new lease has epoch 2. |
| `expired_lease_cannot_write_even_without_takeover` | An expired lease gets `LeaseExpired` even when nobody has taken over. |
| `renewal_keeps_the_epoch` | `renew_lease` extends `expires_at` without changing `epoch`, and a later append still succeeds. |
| `stale_view_gets_seq_conflict` | Appending with an `expected_next` that is behind the head returns `SeqConflict` with the actual `next_seq`, and nothing is written. |
| `concurrent_appends_exactly_one_wins` | Two tasks append at the same `seq` through the same lease at once; exactly one succeeds, the other gets `SeqConflict`, and the log is a valid chain. |
| `takeover_after_crash_recovers_tool_calls_safely` | A worker writes a side-effecting intent, a pure intent with its result, and an idempotent intent, then "dies". After takeover, `dangling_tool_calls` yields `MarkUnknown` for the side-effecting call and `Rerun` for the idempotent one; the new worker records `ToolOutcomeUnknown` and one dangling call remains. |
| `finished_run_is_closed` | After a terminal payload, `append` returns `RunClosed` and `acquire_lease` returns `RunClosed`. |
| `failed_batch_writes_nothing` | A batch with a terminal payload that is not last is rejected as `InvalidPayload`, the log is unchanged, and a retry at the same `seq` succeeds. |
| `workers_cannot_forge_store_events` | `append` of a `RunCreated` or `LeaseAcquired` payload is rejected as `InvalidPayload`. |
| `unknown_run_is_not_found` | `read` and `acquire_lease` on a random run id return `RunNotFound`. |
| `read_from_offset` | `read(run, Seq(n))` returns exactly the events from `n` on; an offset past the end returns an empty vector. |
| `external_input_needs_no_lease` | `append_external` succeeds while another worker holds the lease, stamps the current epoch, and makes the lease holder's next append a `SeqConflict` that succeeds on retry at the new `next_seq`. |
| `external_append_rejects_execution_events` | `append_external` with a batch that mixes a decision and a `ToolResult` is `InvalidPayload`, and nothing from the batch is written. |
| `external_append_rejects_closed_runs` | `append_external` on a finished run is `RunClosed`. |
| `lists_runs` | `list_runs` contains every run created in the test. |

## Backend-specific extras

- `tests/redb.rs`: `state_survives_reopen`. Writes, drops the store, reopens the same file; the log compares equal, verifies, and the old lease is still live and still fenceable after the TTL passes. The test wraps `RedbStore` in a `TempStore` that keeps the temporary directory alive.
- `tests/postgres.rs`: `events_table_rejects_updates_and_deletes`. Runs `UPDATE`, `DELETE` and `TRUNCATE` against `events` directly and expects each to fail with the trigger's message.

## Running against Postgres

```sh
createdb shiioo_test
SHIIOO_TEST_DATABASE_URL=postgres://localhost/shiioo_test cargo test -p shiioo-store --features postgres
```

Without the variable the file prints `skipping: SHIIOO_TEST_DATABASE_URL not set` and every case passes vacuously. With `CI` set and the variable missing, `setup` panics so a misconfigured pipeline cannot hide a skipped suite.

## Adding a case

1. Write `pub async fn new_case<S: EventStore>(store: &S, clock: &ManualClock) -> Result<(), StoreError>` in `tests/common/mod.rs`.
2. Add its name to the list inside `conformance_suite!`. Every backend picks it up on the next `cargo test`.
3. If the behaviour is backend-specific (file reopen, SQL triggers), put it in that backend's test file instead.

Related: [`backends.md`](backends.md), [event log design](../../../docs/design/event-log.md).
