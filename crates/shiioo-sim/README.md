# shiioo-sim

Seeded, reproducible crash-recovery simulation for the runtime. Each seed generates a scripted model conversation with side-effecting, pure and idempotent tool calls, picks a store backend, and then drives the run through a sequence of worker "lives" that crash at chosen points, race each other for the lease, survive machine restarts, and replay stale invocations after a takeover. When the run finishes, or stops for a human, the log and the tools' own execution counts are checked against the invariants the design promises. The same seed always produces the same scenario and the same outcome.

This crate is `publish = false`; it exists to be run by `cargo test`.

## Where it sits

- Depends on: `shiioo-core` (events, chain verification), `shiioo-store` (`MemoryStore`, `RedbStore`), `shiioo-runtime` (`Worker`, `LocalMachine`, `ScriptedProvider`, `CrashPoint`, `Faults`), `serde_json`, `tempfile`, `tokio` (`rt`, `time`, `sync`).
- Depended on by: nothing. It is the last line of the test pyramid after the unit tests and the store conformance suite.

## Public API

| Item | What it is |
|---|---|
| `Rng` | splitmix64: `new(seed)`, `next_u64`, `below(n)`, `chance(percent)`, `pick(&[T])`. |
| `Backend` | `Memory` (even seeds) or `Redb` (odd seeds). |
| `ToolKind` | `Charge` (side-effecting), `Read` (pure), `Write` (idempotent, key = call id). `name()` gives the tool name. |
| `PlannedCall { id, tool }`, `Scenario { seed, backend, turns, final_text }` | What a seed decides up front. `Scenario::generate(seed, &mut rng)`. |
| `run_scenario(seed) -> Result<Report, Violation>` | Runs one seed on a fresh single-threaded Tokio runtime in a temporary directory. |
| `Report { seed, lives, crashes, races, restarts, stale_replays, needs_attention }` | Counters for a seed that passed. |
| `Violation { seed, message, log }` | A broken invariant. `Display` prints the message, the reproduce command and one line per event. |
| `compact(&Event) -> String` | The one-line event format used in violation reports. |

## Using it

```rust
match shiioo_sim::run_scenario(42) {
    Ok(report) => println!("{} lives, {} crashes", report.lives, report.crashes),
    Err(violation) => panic!("{violation}"),
}
```

## Testing

```sh
cargo test -p shiioo-sim                                   # seeds 0..200
SHIIOO_SIM_SEEDS=2000 cargo test -p shiioo-sim             # seeds 0..2000
SHIIOO_SIM_SEED=1234 cargo test -p shiioo-sim -- --nocapture   # one seed, with the log on failure
```

| Variable | Effect |
|---|---|
| `SHIIOO_SIM_SEEDS` | Number of seeds to run, starting at 0. Default 200. |
| `SHIIOO_SIM_SEED` | Run exactly this seed. Takes precedence. |

`crash_recovery_holds_for_every_seed` runs every seed, prints one summary line (`N seeds: … lives, … crashes, … races, … machine restarts, … stale replays fenced, … runs stopped for a human`), and fails with every failing seed listed and the first violation printed in full. With 100 seeds or more it also asserts that crashes, races, restarts, stale replays, a stopped run and a completed run all occurred, so a change that quietly disables a fault class fails the suite. `same_seed_same_report` runs seeds 0, 1, 7 and 42 twice and compares the counters.

## What it checks

For every seed, after the run finishes:

- the log is a valid hash chain and epochs never decrease;
- the number of `LeaseAcquired` events equals the number of leases workers actually obtained;
- no tool call has more than one `ToolResult`, unless a `ToolOutcomeUnknown` was recorded first;
- no side-effecting (`Charge`) call executed more than once, as counted by the tool itself;
- a run that completed did so with the scripted final text, has exactly one result per planned call, ran every `Charge` exactly once, ends with `RunCompleted`, and had no side-effecting call cut off mid-execution;
- a run that stopped for a human did so because exactly one side-effecting call was cut off mid-execution, that call is the one marked unknown, the machine's journal still says `Started` for it, and it ran exactly once.

Details of the generator, the fault schedule and each invariant: [`docs/scenarios.md`](docs/scenarios.md).

## Further reading

- [Execution design](../../docs/design/execution.md) for the recovery rules being tested, [tools and machines design](../../docs/design/tools-and-machines.md) for the journal, [architecture overview](../../docs/architecture.md).
