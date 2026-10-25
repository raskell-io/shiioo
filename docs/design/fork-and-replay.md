# Fork, rewind, diff and replay

This is Shiioo's differentiator: any run can be rewound to a turn, forked with a different prompt, model or policy, compared with its fork, and replayed against a new model as a regression test. It works because the log is complete ([ADR-0001](../adr/0001-event-log-is-the-only-state.md)) and the workspace is snapshotted at turn boundaries.

Everything here is *(planned)* except the branch diff noted in §4. It depends on the event log ([event-log.md](event-log.md)), the runtime ([execution.md](execution.md)) and workspace snapshots taken by the node ([tools-and-machines.md](tools-and-machines.md)). It is the last phase of the roadmap ([positioning.md](../positioning.md#roadmap)).

## 1. What makes it possible

| Requirement | Provided by |
|---|---|
| Every input the agent saw is in the log | user messages, model requests and responses, tool results, policy decisions are all events |
| Every model output is recorded before it is acted on | `ModelResponseRecorded` precedes any `ToolIntent` ([execution.md](execution.md)) |
| The workspace state at a turn can be restored | `SnapshotTaken` at each turn boundary |
| The config a run ran under is known | `RunCreated` records agent, policy set and skill versions ([governance.md](governance.md)) |
| Divergence can be detected cheaply | `ModelRequested` carries a digest of the canonical model request |

## 2. Turn boundaries and snapshots

A **turn boundary** is the point after all tool results of a turn are recorded and before the next model call. It is the only place a run can be forked: the conversation is consistent (no pending tool calls) and the workspace is quiescent.

At each boundary, if the run has a workspace on a machine, the node takes a snapshot and the worker appends:

```rust
SnapshotTaken {
    snapshot: SnapshotId,       // content address of the snapshot manifest
    machine: MachineId,
    method: SnapshotMethod,     // Git | Btrfs | Zfs | Overlay | Firecracker
    bytes_added: u64,           // incremental size, for cost accounting
}
```

Snapshot methods, cheapest first:

| Method | Covers | Notes |
|---|---|---|
| `Git` | a repo working tree, including untracked files | default for code workspaces; commit to a hidden ref (`refs/shiioo/<run>/<seq>`), deduplicated by git, portable to any machine |
| `Btrfs` / `Zfs` | a whole directory | copy-on-write subvolume snapshot; instant; local to that host |
| `Overlay` | a whole directory | an overlayfs upper layer per turn; works on any Linux |
| `Firecracker` | disk and memory of a microVM | full fidelity including running processes; largest |

Snapshots are uploaded to the blob store when the run is forked from another machine or archived; otherwise they stay on the machine. Snapshotting is skipped for runs without a workspace, and can be disabled per agent.

## 3. Fork and rewind

A fork is a **new run** whose first event points into its parent:

```rust
ForkedFrom {
    parent_run_id: RunId,     // the parent
    parent_seq: Seq,          // the turn boundary forked from (inclusive)
    parent_hash: EventHash,   // hash of the parent's event at parent_seq; pins the exact history
    changes: ForkChanges,     // what differs: model, instructions, policy set, an edited user message, …
}
```

- **The prefix is shared, not copied.** Reading a forked run's log reads the parent's events `0..=seq` first, then the fork's own. The fork's first own event, `ForkedFrom`, is written at `seq + 1` with `prev_hash` set to the hash of the parent's event at `seq`. Seqs and the hash chain therefore continue unbroken, the combined history verifies end to end with the same `verify_chain`, and SSE resume works on the fork exactly as on any run.
- **The workspace is restored** from the `SnapshotTaken` at or before `seq`, on the same machine or on any machine of the office for `Git` snapshots.
- **Rewind** is a fork with no changes, after which the user typically edits the next message. The original run is never modified; it stays in the record.
- Forks of forks are allowed. Reads follow the chain of `ForkedFrom` links. To keep reads cheap, an archived run's log can be materialized (prefix copied) once it is no longer the newest member of a long chain.

API: `POST /v1/runs/{id}/fork { "at_seq": 42, "changes": { "model": "…" } }` ([api.md](api.md)).

## 4. Diff

*(built)* `GET /v1/runs/{id}/diff` shows a run's branch against the repository's `HEAD` (`git diff HEAD...shiioo/<run>`), in the dashboard's Diff panel. *(planned)* Two runs that share an ancestor can be compared from their fork point:

| Dimension | How |
|---|---|
| Transcript | align turns after the fork point; show model text and tool calls side by side, with a unified diff per turn |
| Tool calls | which calls were made in one branch but not the other, and which had different inputs or results |
| Files | diff of the final (or any turn's) snapshots: `git diff` between hidden refs, or a tree diff for filesystem snapshots |
| Outcome | terminal event, final output |
| Cost and time | tokens, cost and wall-clock per turn and in total, from `BudgetCharged` and event timestamps |
| Governance | policy decisions and approvals that differed |

Runs without a shared ancestor (for example two replays of the same recording) are diffed from their first event.

## 5. Replay

Replay re-executes a **recorded run** with something changed: a different model, prompt, agent definition or policy set. The recorded run is the *source*; the replay is a new run with `ReplayOf { source, mode }` as its first event.

### 5.1 Serving recorded tool outputs

During replay, the model is called live. Tools are not, as long as the replay stays on the recorded path:

- A tool call **matches** when it is the next unconsumed call in the source with the same tool name and the same **canonical input digest** (SHA-256 of the input with keys sorted, whitespace normalized, and fields declared volatile by the tool removed).
- A matching call gets the recorded result and no execution happens. This makes replay of side-effecting runs safe and fast.
- Model divergence is detected earlier, from `ModelRequested` digests: once the replay's model request differs from the source's at the same turn, later tool calls may diverge too.

### 5.2 Divergence modes

| Mode | When a tool call doesn't match |
|---|---|
| `strict` | fail the replay at the first divergence; used to check determinism |
| `stop-on-divergence` | end the replay there and report how far it matched |
| `live-on-divergence` | execute the call for real (subject to the replay's own policies), and continue live |

`live-on-divergence` needs a workspace: the replay forks the source's snapshot at the divergence point. Side-effecting tools default to `require approval` in replays, so an eval can't push to a real repository by accident.

### 5.3 Policy-only replay

Replaying under a new policy set without calling any model re-evaluates every recorded `PolicyDecision` and reports which calls would now be denied or need approval. This is cheap and is the recommended check before applying a policy change ([governance.md](governance.md)).

## 6. Evals

An **eval suite** is a saved selection of recorded runs:

```toml
[suite]
name = "triage-regressions"
select = { agent = "triage", tags = ["golden"], since = "2026-09-01" }
mode = "stop-on-divergence"

[[checks]]
kind = "exact"           # final output equals the source's
[[checks]]
kind = "regex"
pattern = "label: (bug|feature)"
[[checks]]
kind = "llm_judge"
model = "…"
rubric = "Did the agent apply the same label for the same reason?"
[[checks]]
kind = "custom"          # a WASM or HTTP checker
uri = "…"
```

Running a suite replays each run against a **candidate** (model, prompt or agent version) and produces a report: pass/fail per check, divergence point, cost and latency deltas, and links to the per-run diffs. Any production run can be tagged `golden` from the dashboard, which is how real usage turns into a regression suite.

## 7. Limits of determinism

Replay is exact only for what is recorded. Be honest about the rest:

| Source of nondeterminism | Handling |
|---|---|
| Model sampling | live model calls in replay will differ; that is the point of an eval. `temperature = 0` reduces but does not remove it |
| Time | tools that read the clock declare `time` as a volatile input field; the recorded time is served back in matched calls |
| Randomness | tools that generate IDs or random values must take a seed from the runtime, recorded in the intent |
| The external world | a live call to a website or API in `live-on-divergence` mode sees today's world, not the recorded one |
| Unrecorded state on the machine | anything outside the snapshotted workspace (system packages, caches, `/tmp`) is not restored; Firecracker snapshots close this gap |

Tools declare their determinism in their definition:

```rust
struct ToolDeterminism {
    deterministic: bool,            // same input + same workspace => same output
    volatile_input_fields: Vec<String>,
    needs_seed: bool,
}
```

**Harness runs** (Claude Code, Codex and others through ACP, see [runtimes-and-providers.md](runtimes-and-providers.md)) have weaker guarantees. Shiioo records what the harness reports (messages, tool calls, results) but the harness makes its own model calls and may run tools Shiioo does not see. Forking a harness run restores the workspace and transcript and starts a new harness session from there; tool-output replay works only for tool calls the harness routes through Shiioo.

## 8. Storage cost and garbage collection

- Snapshots are content-addressed and incremental (git objects, CoW blocks), so a turn usually adds kilobytes, not a copy of the workspace.
- Each snapshot has a reference count: the run that took it plus every fork whose `ForkedFrom.seq` needs it, plus every eval suite that pins it.
- Retention: keep all snapshots of runs younger than N days; keep only the final snapshot plus pinned ones for older runs; delete unreferenced snapshots after a grace period.
- The event log itself is never garbage-collected by this process. Archival moves old logs to cold storage ([event-log.md](event-log.md)) but keeps them verifiable.

## 9. Events summary

| Event | Meaning |
|---|---|
| `SnapshotTaken` | workspace captured at a turn boundary |
| `ModelRequested` | digest of the canonical model request, before the call |
| `ForkedFrom` | first event of a fork: parent run, fork point, changes |
| `ReplayOf` | first event of a replay: source run, divergence mode |
| `ReplayDiverged` | the replay left the recorded path at this turn |
