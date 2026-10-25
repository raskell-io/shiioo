# Glossary

Shiioo has two vocabularies. The **platform terms** are what the API, code and docs use. The **company terms** are the "Virtual Company OS" layer: a way of setting up and talking about a project, built entirely from platform primitives. Neither hides the other; the dashboard can show either.

## Company terms → platform terms

| Company term | Platform term | Meaning |
|---|---|---|
| Company | Project | A self-contained unit with its own agents, policies, budgets, machines and runs. |
| CEO | Project owner | The human who sets direction and holds final approval. |
| Employee | Agent | A configured agent: model, instructions, tools, skills, policy, budget. |
| Job title / role | Role (`[[roles]]`) | A reusable agent configuration: employee, instructions, tools, model, isolation, limits; named with `@` in `/run`. Versioning is planned. Legacy name: archetype. |
| Department | Agent label / group | A grouping of agents for policies, budgets and reporting. |
| Chief of Staff | The planning run behind `/plan` | A read-only run that turns a goal into work items; the owner's approval makes them issues the intake works. Re-planning from direction documents is planned. |
| Vision, mission, strategy | Direction documents | Files in the project the planner reads. Inputs, not platform objects. |
| Task / assignment | Run | One agent working on one goal, with its own event log. |
| Delegating to a colleague | Child run (subagent) | A run started by another run, with narrower credentials. |
| Company policy | Policy | Cedar rules: what is allowed, denied, or needs approval. |
| Executive review | Approval board | A group of approvers with quorum rules. |
| Department budget | Budget | Token, cost and time limits, enforced by the server. |
| Office / workstation | Machine | The office's own machine, or a node: another host running `shiioo node` that enrolled and takes the runs addressed to it. |
| Deliverable | Artifact | A file a run produced: listed with `/files`, downloadable, sent to the phone when it is an image, audio, video or PDF. |
| Outgoing letter | Mail (`mail.send`) | A mail the office sends for a run after the rules or the owner allowed it; recorded as `MailSent`. |
| System credentials | Secret | Credentials stored encrypted in the office, referenced by name (`secret:<name>`), injected when a harness starts and redacted from the record. |
| Locked office | Isolation level | Where a run executes: `worktree` (the owner's user), `container` (a container per run with an egress allow list), `openshell` (experimental). |
| Recurring duty | Routine (`[[routines]]`) | Starts runs on a typed schedule (`weekdays 07:00`, `every 6h`); external events are planned. |
| Proposal for a policy change | Change request | An agent-proposed diff to agent definitions, policies or skills, approved by a board. |
| The company's records | Event logs + audit | Hash-chained logs of every decision and action. |

## Platform terms

- **Event**: one entry in a run's log. Has `seq`, `epoch`, `recorded_at`, `schema`, `prev_hash`, `hash` and a payload.
- **Event log**: a run's append-only, hash-chained sequence of events. The only source of truth ([ADR-0001](adr/0001-event-log-is-the-only-state.md)).
- **Seq**: an event's position in its run, contiguous from 0. Also the stream resume point.
- **Epoch**: the fencing token of the lease an event was written under ([ADR-0002](adr/0002-leases-with-fencing.md)).
- **Lease**: a worker's time-limited right to append to one run.
- **Fenced**: rejected because the presented lease is no longer the current one.
- **Run head**: the small per-run record every write is checked against: next `seq`, last hash, lease, closed flag.
- **Turn**: one model call plus the tool calls it requested.
- **Tool class**: `Pure`, `Idempotent { key }` or `SideEffecting`; decides what happens to a tool call whose outcome was not recorded.
- **Tool intent / result**: the events written before dispatching a tool call and after it returns.
- **Outcome unknown**: a side-effecting call that may or may not have run; resolved by reconciling with the machine's journal or by a human.
- **Parked**: a run waiting for input, approval or a timer, holding no lease.
- **Worker**: a stateless process that leases runs and executes turns.
- **Server**: the `shiioo` process that holds the record, decides (rules, approvals, capacity), serves the API, dashboard and Telegram bot, and runs the workers. Its gateway role holds node connections and stream subscribers.
- **Scheduler**: finds runnable runs, fires triggers and timers.
- **Node**: `shiioo node`, a worker on another machine that enrolls with the office, dials out over HTTP, takes the runs addressed to it and appends to the office's log through the API ([ADR-0010](adr/0010-single-binary-server-and-nodes.md), [ADR-0012](adr/0012-nodes-are-workers-over-the-offices-api.md)). Early docs called it the daemon. The office's own machine is the node inside the server process (`shiioo up`).
- **Snapshot**: a saved copy of a run's workspace at a turn boundary.
- **Fork**: a new run that starts from another run's state at a given `seq`.
- **Replay**: re-executing a recorded run, serving recorded tool outputs back, typically against a different model or prompt.
- **Harness**: an external agent program (Claude Code, Codex) run headless as a Shiioo runtime; its output is recorded as events and its tool calls go through the hook. Harnesses that only speak the Agent Client Protocol are planned.
- **Employee**: a configured staff entry (`[[employees]]`): a harness or a native-loop model, with the subscription-window limits above which it is skipped for new tasks.
- **Capacity window**: a provider's rate-limit window (Claude: five-hour and seven-day) with its utilization and reset time, observed from harness output as `CapacityObserved`.
- **Hook**: the program a harness runs before each tool call (`shiioo hook pre-tool-use`); it asks the server and prints allow or deny. A dead server means deny.
- **Rules**: the owner's `rules.toml`: allow / ask / deny per tool, command, path and repo; first match wins, chained commands are judged by their most restrictive segment.
- **Approval**: an `ask` verdict turned into a question for the owner (`ApprovalRequested`), answered by a tap or a timeout (`ApprovalDecided`).
- **Office**: the directory the server runs in (`--workspace`), with its data in `.shiioo/`.
- **Worktree**: the git worktree and `shiioo/<run>` branch a repo run works in, under `.shiioo/worktrees/<run>`.
- **Intake**: a source of work other than the owner typing: today labelled GitHub issues.
- **Projection**: a derived, rebuildable view over event logs (run lists, cost totals, search). Never a source of truth.
- **Checkpoint**: a signed statement of a log's head hash, published outside the system to make tampering detectable.
