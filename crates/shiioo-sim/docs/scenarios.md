# Scenarios, faults and invariants

How a seed becomes a scenario, which faults each worker life may suffer, and exactly what is checked at the end. Everything is drawn from one splitmix64 `Rng` seeded with the seed, so a seed is a complete description of a run.

## Generation

`Scenario::generate(seed, &mut rng)`:

| Decision | Rule |
|---|---|
| Backend | even seed: `MemoryStore<ManualClock>`; odd seed: `RedbStore` in a fresh temporary directory, same `ManualClock` |
| Tool turns | `rng.below(6)`: 0 to 5 turns that call tools, followed by one final answer |
| Calls per turn | `1 + rng.below(3)`: 1 to 3 |
| Each call | id `c<turn>_<index>`, tool picked uniformly from `charge`, `read`, `write` |
| Final text | `done (seed <seed>)` |

The clock starts at `1 700 000 000 000` ms and only moves when the simulator advances it. The model is a `ScriptedProvider` that replays one `ModelResponse` per turn: `tool_use` blocks with `input: { "call": "<id>" }` and `stop_reason: ToolUse`, then the final answer.

## Tools

| Tool | `ToolClass` | Key |
|---|---|---|
| `charge` | `SideEffecting` | |
| `read` | `Pure` | |
| `write` | `Idempotent { key }` | the call id |

Every tool yields once (`tokio::task::yield_now`) before doing its work, so a racing worker gets polled mid-turn, then records its execution in a shared counter keyed by call id and returns `<tool> <call>`. The counter is the simulator's ground truth for "how often did this really run", independent of what the log says.

The machine is a `LocalMachine` opened on the temporary directory with a `journal.jsonl` file. Reopening it is how a machine restart is simulated.

## Lives

After `create`, the run is driven for at most `MAX_LIVES = 40` lives. For each life:

| Step | Odds | Effect |
|---|---|---|
| Machine restart | 15 % | The `LocalMachine` is reopened from its journal before the life starts. |
| Crash | 60 %, only during the first `FAULTY_LIVES = 25` lives | The worker is given `Faults { crash_at }` with one of the five crash points below. |
| Race | 20 %, faulty lives, memory backend only | Two workers (`w<n>a`, `w<n>b`) drive the run concurrently with `tokio::join!`; the rival crashes with 50 % odds. |

After life 25 no faults are injected, so a run that keeps crashing still has 15 clean lives to finish.

Crash points (`shiioo_runtime::CrashPoint`): `AfterModelResponse`, `AfterToolIntent`, `DuringToolExecution`, `AfterToolExecuted`, `AfterToolResult`.

Each `drive` result is classified by `settle`:

| Result | Meaning |
|---|---|
| `Ok(Completed)` or `Ok(NeedsAttention)` | The run is over; the loop stops after this life's checks. |
| `Ok(LostLease)` | A violation: the clock never moves during a life, so a lease cannot expire under a live worker. |
| `Err(Crashed)` or `Err(Machine(SimulatedCrash))` | The injected crash fired. |
| `Err(Store(LeaseHeld))` | The loser of a race; not a crash. |
| any other error | A violation. |

After every life the chain is verified. After a crash the simulator looks at the last event:

- a `ToolIntent` for a side-effecting call with crash point `DuringToolExecution` means the tool ran and nothing recorded it; the call id is remembered as `hit_mid_execution`;
- a `ToolIntent` with crash point `AfterToolIntent` is remembered as a stale invocation (epoch, call id, tool, input) if none is pending yet;

and then the clock is advanced by `TTL + 1` ms (30 001 ms), because a dead worker never releases its lease.

Stale replay: once another worker has acquired a lease after the stall, with 60 % odds per life the stalled worker "wakes up" and sends its invocation to the machine with its old epoch. The machine must answer `MachineError::Fenced`, and the tool's execution counter must not move. Anything else is a violation on the spot.

Races run only on the memory backend because redb executes on Tokio's blocking pool, where interleaving would depend on OS scheduling and the seed would no longer determine the outcome. Each seed runs on its own single-threaded Tokio runtime for the same reason.

## Invariants checked at the end

`check(outcome)` reads the whole log and collects every problem before failing, so a violation message can list several.

Always:

| Invariant | Violation text |
|---|---|
| `verify_chain` passes | `chain broken: …` |
| `epoch` never decreases between consecutive events | `epochs decreased` |
| `LeaseAcquired` events equal the lease acquisitions workers observed | `N LeaseAcquired events, but workers acquired M leases` |
| at most one `ToolResult` per call, unless a `ToolOutcomeUnknown` for that call came first | `call X has N results` |
| every `charge` call executed at most once | `side-effecting call X ran N times` |

If the outcome is `NeedsAttention`:

| Invariant | Violation text |
|---|---|
| exactly one call has an unknown outcome and it is the one hit mid-execution | `needs attention without a matching mid-execution crash …` |
| the machine's journal reports `CallStatus::Started` for it at the current epoch | `needs attention for X, but the machine says …` |
| it executed exactly once | `unknown call X ran N times, expected 1` |

If the outcome is `Completed(text)`:

| Invariant | Violation text |
|---|---|
| `text` is the scripted final answer | `completed with "…"` |
| no side-effecting call was hit mid-execution | `completed although side-effecting call X ran unrecorded` |
| every planned call has exactly one result | `call X does not have exactly one result` |
| every `charge` executed exactly once | `side-effecting call X ran N times, expected 1` |
| the last event is `RunCompleted` | `completed, but the log does not end with RunCompleted` |

Any other outcome, and a run that is still going after 40 lives, is a violation.

## Reproducing a failure

A `Violation` prints like this:

```
seed 1234: side-effecting call c2_0 ran 2 times (scenario: Scenario { … })
reproduce: SHIIOO_SIM_SEED=1234 cargo test -p shiioo-sim -- --nocapture
log:
  #0   e0 run created
  #1   e1 lease → w0
  #2   e1 model: calls c0_0,c0_1
  #3   e1 intent c0_0 charge SideEffecting
  …
```

Each log line is `compact(&event)`: `#<seq> e<epoch> <what>`, with `what` one of `run created`, `lease → <worker>`, `model: calls a,b` or `model: final answer`, `intent <call> <tool> <class>`, `result <call>` (with ` (error)`), `unknown <call>`, `harness …`, `capacity <provider>`, `message from <author>`, `rules <decision> <tool>`, `approval? <id>`, `approval <id> <decision>`, `cancel by <who>`, `cancelled`, `completed`, `failed: <reason>`.

`crash_recovery_holds_for_every_seed` fails with the list of failing seeds and the first violation in full. Run the single seed with `--nocapture` to see the same output while iterating on a fix; `same_seed_same_report` guards that the seed still means the same thing.

## Coverage guard

With 100 seeds or more the test also asserts that the fault classes actually fired: at least one crash, one race, one machine restart, one fenced stale replay, one run that stopped for a human, and one run that completed. A refactor that silently disables a fault path therefore fails the suite rather than passing vacuously.

## Checking that the harness bites

The simulator only proves something if it can fail. A quick way to confirm that after changing the runtime: make a recovery rule wrong on purpose (for example re-run side-effecting dangling calls instead of marking them unknown), run the default 200 seeds, and expect `side-effecting call … ran 2 times` violations. Revert before committing.

Related: [execution design](../../../docs/design/execution.md) for the rules under test, [tools and machines design](../../../docs/design/tools-and-machines.md) for the journal and fencing, [event log design](../../../docs/design/event-log.md) for the chain.
