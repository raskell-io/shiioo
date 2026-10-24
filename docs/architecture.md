# Architecture overview

Shiioo is an agent platform: it runs AI agents on any model and any machine, with tools, secrets, policies, approvals, budgets and live streaming. Its pitch is **Virtual Company OS**: you set direction, agents do the work, and every decision is on the record. This document is the map. Each area has a design doc under [`design/`](design/), and the decisions behind it are recorded as [ADRs](adr/).

> **Status.** The execution kernel (event model, hash chain, leases with fencing, crash recovery) and three storage backends exist and are tested. Everything else here is the target design. Sections say *(built)* or *(planned)*.

## 1. The one rule

**The per-run, append-only event log is the only state.** Workers, gateways and schedulers hold nothing they cannot rebuild from the logs. Live streams, crash recovery, audit, fork and replay are all views over that log ([ADR-0001](adr/0001-event-log-is-the-only-state.md)).

Everything below follows from this rule. When a design question comes up, the first question is "which events does this write, and who may write them?"

## 2. Guarantees

These hold for every backend and are enforced in `shiioo-core`, not in each backend. Details: [design/event-log.md](design/event-log.md), [design/execution.md](design/execution.md).

| Guarantee | Mechanism | Status |
|---|---|---|
| A write from a stalled or partitioned worker is never accepted after someone else took over | Leases with a strictly increasing `epoch` (fencing token), checked on every append ([ADR-0002](adr/0002-leases-with-fencing.md)) | built |
| No lost or reordered events | `seq` contiguous from 0, optimistic append with `expected_next`, all-or-nothing batches | built |
| Tampering, gaps and reordering are detectable | SHA-256 hash chain over every event | built |
| A side effect is never repeated blindly after a crash | Tool intent before execution, result after; unmatched intents become `Rerun` or `ToolOutcomeUnknown` by `ToolClass` | built (rules), planned (runtime) |
| Replay is deterministic | The full model response is recorded before any tool in it runs | built (event), planned (replay engine) |
| Finished runs stay finished | Nothing appends after a terminal event | built |
| Clients never miss output | SSE stream resumes from `seq` (`Last-Event-ID`) | planned |

## 3. Domain model

The API uses technical names. The "virtual company" vocabulary is a layer on top of them, see the [glossary](glossary.md).

```
Organization (tenant, billing, SSO)
└── Project                     ≈ a "company"
    ├── Agent                   ≈ an "employee": model, instructions, tools, skills, policy, budget
    ├── Run                     one agent working on one task; has its own event log
    │   └── child Runs          subagents, with narrower credentials than the parent
    ├── Machine                 a host or sandbox where tools execute (runs `shiioo daemon`)
    ├── Policy                  Cedar policies: what needs approval, what's forbidden
    ├── ApprovalBoard           who approves what, with quorum rules
    ├── Budget                  token, cost and time limits per agent, run or project
    ├── Secret                  encrypted; injected at the tool boundary, never into model context
    └── Trigger                 cron, webhook, GitHub/Slack event → starts a run
```

- **Run**: the unit of execution, logging, leasing, forking and audit. A run can be short (one task) or long-lived (a conversation that waits for user messages). A run that is waiting holds no lease and costs nothing.
- **Turn**: one model call plus the tool calls it requested. Turns are the natural checkpoint for snapshots and forks.
- **Step**: any event in a run. `seq` numbers steps.

## 4. Components

```
  phone (Telegram, web dashboard over Tailscale) · GitHub · CLI
                 │  REST + resumable SSE  (/v1), Telegram long-polling
┌────────────────▼──────────────── shiioo server ─────────────────────────────┐
│  api        auth, validation, writes commands as events, serves streams      │
│  scheduler  finds runnable runs, fires triggers and timers, places work      │
│  worker     leases a run, executes turns: context → model → rules → tools    │
│  gateway    holds node connections and SSE subscribers, routes tool calls    │
│  telegram   delegation, approvals and notifications for the owner            │
└───────┬──────────────────────────────┬─────────────────────────┬─────────────┘
        │ EventStore                   │ BlobStore               │ WebSocket + TLS, dialed out
   redb file (the only source     local disk (object           shiioo node on each machine
   of truth; Postgres optional)   storage optional)            runs harnesses, tools and
                                                               sandboxes; owns workspaces
```

- **One binary, `shiioo`, three roles** ([ADR-0010](adr/0010-single-binary-server-and-nodes.md)). `shiioo up` runs server and node in one process (the whole system on one Mac); `shiioo server` and `shiioo node --server …` split them across machines; the CLI is the same binary. Like Nomad: the server decides, nodes execute and dial out.
- **Workers are stateless.** Any worker can run any turn of any run. A worker crash costs at most the current turn, which the next lease holder recovers from the log.
- **Work runs on nodes, not in workers.** For harness runs the node starts the harness and streams its steps back; for tool calls the worker records the intent, the gateway forwards the call to the node, the node executes and returns the result, the worker records it.
- **Nodes dial out.** No inbound ports on machines; works behind NAT, and over Tailscale needs no public address at all.

Details: [design/execution.md](design/execution.md), [design/tools-and-machines.md](design/tools-and-machines.md), [design/runtimes-and-providers.md](design/runtimes-and-providers.md).

## 5. Storage

All backends implement one `EventStore` trait and pass one conformance suite ([ADR-0003](adr/0003-redb-default-postgres-for-clusters.md)).

| Backend | Use | Status |
|---|---|---|
| **redb** (default) | On-device and single node. Pure Rust, single file, ACID, crash-safe, no server. One writer per file | built |
| **Postgres** (`postgres` feature) | Clusters. Also the run queue (`FOR UPDATE SKIP LOCKED`) and wakeup bus (`LISTEN/NOTIFY`). Lease expiry uses the database clock | built |
| **Memory** | Reference implementation and simulation tests | built |
| **Object storage** | Blobs: large payloads, attachments, workspace snapshots, archived runs | planned |

Turso (the Rust SQLite rewrite) is a candidate to replace redb once it's stable, for concurrent writers in one embedded file. Details: [design/event-log.md](design/event-log.md).

## 6. Governance and trust

The governance features are what turn an agent runtime into a company you can trust. Ported and extended from shiioo-legacy ([ADR-0005](adr/0005-cedar-policies-and-biscuit-credentials.md)).

- **Policies** in Cedar decide per tool call: allow, deny, or require approval. Every decision is an event with the policy version that made it.
- **Approval boards** with quorum rules (for example two of three maintainers for a production deploy).
- **Budgets** enforced by the server before each model and tool call, not trusted to the agent.
- **Change management**: agents can propose changes to their own agent definitions or policies, which go through approval like any other risky action.
- **Credentials**: Biscuit tokens. A subagent gets a token attenuated from its parent's, so it can never do more than its parent.
- **Provable audit**: the hash chain, plus signed checkpoints anchored outside the system.

Details: [design/governance.md](design/governance.md), [design/security.md](design/security.md).

## 7. The differentiator: forkable, replayable runs

Because the log is complete and the workspace is snapshotted at turn boundaries, any run can be:

- **rewound** to a turn and **forked** with a different prompt, model or policy;
- **diffed** against its fork, transcript and files;
- **replayed** against a new model, with recorded tool outputs served back, as a regression eval.

Details: [design/fork-and-replay.md](design/fork-and-replay.md).

## 8. Crate map

| Crate | Responsibility | Status |
|---|---|---|
| `shiioo-core` | Event model, hash chain, `RunHead` rules, leases, recovery, `EventStore` trait. No I/O | built |
| `shiioo-store` | `MemoryStore`, `RedbStore`, `PostgresStore` | built |
| `shiioo-blob` | Content-addressed blob store over `object_store` | planned |
| `shiioo-runtime` | The agent loop: context building, turns, tool dispatch, recovery on takeover; Anthropic provider; built-in tools; local machine with call journal and epoch fencing | built (spike) |
| `shiioo-providers` | Model adapters and the capacity broker (key pooling, failover, cost) | planned |
| `shiioo-tools` | Tool trait, built-in tools, MCP client, ACP harness adapters | planned |
| `shiioo-policy` | Cedar evaluation, approvals, quorum, budgets, change management | planned |
| `shiioo-auth` | OIDC, API keys, Biscuit tokens, RBAC | planned |
| `shiioo-protocol` | Daemon ↔ gateway wire protocol (protobuf) | planned |
| `shiioo-api` | REST + SSE handlers, OpenAPI | built: runs, events, resumable SSE, token auth, problem+json; planned: OpenAPI, the remaining resources |
| `shiioo-sim` | Deterministic whole-system simulation tests | planned |
| `shiioo` | The binary: `up`, `serve`, `daemon`, CLI | built: `run`, `resume`, `log`, `serve`; planned: `up`, `daemon`, the rest |

Dependency direction: `core` ← everything; nothing depends on `api` or the binary. `core` stays free of I/O so the simulator can drive it.

## 9. Testing strategy

- **Conformance suite** for every `EventStore` backend *(built)*.
- **Deterministic simulation** (`shiioo-sim`, on `turmoil`/`madsim`): whole clusters with partitions, worker kills, clock skew and slow disks, with invariants checked after every step *(planned)*.
- **`loom`** for in-process concurrency primitives, **property tests** for the run state machine *(planned)*.
- **Chaos job in CI**: real processes, random `kill -9`; assert every run finishes and no side-effecting call ran twice *(planned)*.

## 10. Roadmap

The product roadmap lives in [positioning.md](positioning.md#roadmap). In kernel terms:

0. **Kernel spike** *(done)*: event model, chain over stored bytes, fencing, recovery, memory/redb/Postgres backends, the native agent loop with the Anthropic provider, built-in tools, an in-process machine with a call journal and epoch fencing, the HTTP API with resumable SSE, the `shiioo` CLI, and the deterministic simulation harness (`crates/shiioo-sim`). The exit test passes: a worker killed at any point of a turn is taken over and the run finishes with no side effect repeated.
1. **Harness runtime** ([ADR-0009](adr/0009-harness-first-runtimes.md)): Claude Code driven headless on a node, its steps recorded as events, a git worktree and branch per run, the dashboard over Tailscale.
2. **Rules, approvals, Telegram**: allow / ask / deny per tool, repo and action; harness permission requests routed to the owner's phone; notifications.
3. **Capacity**: Codex and local-model employees, the broker across subscriptions and endpoints, budgets, GitHub-issue intake.
4. **Isolation levels**: `container` and `openshell` on nodes, secrets injection, egress rules.
5. **Planner, routines, roles**; then remote nodes, asset and mail machines; then replay and forks.

## Document index

See [docs/README.md](README.md).
