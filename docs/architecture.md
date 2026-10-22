# Architecture

## Core rule

The per-run, append-only event log is the only state. Workers hold nothing they cannot rebuild from it. Streams, recovery, audit, fork and replay are all views over that log.

Each event carries:

| Field | Meaning |
|---|---|
| `seq` | Position in the run, contiguous from 0. Also the SSE `Last-Event-ID`, so clients resume streams straight from the log. |
| `epoch` | The lease epoch it was written under (the fencing token). |
| `prev_hash`, `hash` | SHA-256 chain over every field, making the log tamper-evident. |
| `schema` | Payload schema version. Old versions are upcast on read, after the chain is verified. |

## Execution guarantees

1. **Leases with fencing.** A worker leases a run and gets a strictly larger `epoch` than any earlier lease. Every append presents the lease, and the store rejects it unless its epoch is current and unexpired. A stalled worker that wakes up after a takeover gets `Fenced` and stops. Each acquisition is recorded as `LeaseAcquired`, so takeovers show up in the log.
2. **Optimistic append.** An append names the `seq` its first event will get. A stale view gets `SeqConflict`. Batches are all-or-nothing.
3. **Model output recorded before acting.** The full model response is appended before any tool in it runs, which makes replay deterministic.
4. **Explicit side effects.** Tools declare a `ToolClass`: `Pure`, `Idempotent { key }` or `SideEffecting`. Every call is an intent event first and a result event after. On takeover, `dangling_tool_calls` turns unmatched intents into `Rerun` (pure and idempotent) or `MarkUnknown` (side-effecting). Unknown outcomes go to a reconcile hook or a human, and are never retried blindly.
5. **Closed runs stay closed.** Nothing may be appended after `RunCompleted` or `RunFailed`.
6. **Time is injected.** `Clock` is a trait, so simulations control time. The Postgres backend decides lease expiry with the database clock, so clock skew between workers can't extend a lease. The embedded backend runs on one machine, so it uses that machine's clock.

## Components (target)

```
clients (CLI, dashboard, SDKs, Slack/GitHub)
        │  REST + resumable SSE (OpenAPI)
┌───────▼─────────────── shiioo server (one binary, roles by flag) ──────────┐
│ api │ gateway (daemon + SSE conns) │ workers (agent loop) │ scheduler    │
└───────┬───────────────────────────────────────────────┬────────────────────┘
        │ sqlx                                          │ QUIC/WebSocket, mTLS, outbound
   redb file (on-device) or Postgres (cluster)       shiioo daemon on machines
   LISTEN/NOTIFY)  + object storage (blobs,          and sandboxes (tools run here)
   snapshots, cold runs as Parquet)
```

- **One binary, roles by flag:** `shiioo server --roles api,worker,scheduler,gateway`. Scale out by running more of a role.
- **Storage backends** behind one `EventStore` trait, all passing the same conformance suite:
  - **redb** (default): a pure-Rust, crash-safe, ACID embedded store in a single file, with no server to run. For on-device and single-node use. redb allows one writer per file, which serializes writes across runs; readers never block.
  - **Postgres** (`postgres` feature): for clusters with many workers. It also serves as the job queue (`SKIP LOCKED`) and the notification bus (`LISTEN/NOTIFY`). No Redis. NATS is optional later.
  - **Turso** (the Rust rewrite of SQLite, beta at v0.7 as of 2026-07, with MVCC concurrent writes) is a candidate to revisit once it's stable. It could give one embedded engine with concurrent writers.
- **Policies** in Cedar. **Agent credentials** as Biscuit tokens, so a subagent gets a strictly narrower token than its parent.
- **Observability:** `tracing` + OpenTelemetry, one span per step, trace ID = run ID.

## Testing

- `shiioo-store/tests/common`: the conformance suite every backend must pass (`tests/memory.rs`, `tests/redb.rs`, `tests/postgres.rs`).
- Next: deterministic simulation of the whole cluster (`turmoil`/`madsim`) with partitions, worker kills and clock skew. Plus `loom` for concurrency primitives and property tests on the state machine.
- A CI chaos job: kill workers at random, then assert every run finishes and no side-effecting call ran twice.

## Roadmap

1. **Kernel spike** *(in progress)*: event model, hash chain, fencing and recovery are done, with memory, redb and Postgres backends. Still to do: agent loop with one provider, shell and file tools, local daemon, SSE, simulation harness. Exit test: kill -9 a worker mid-turn and the run still finishes correctly.
2. **MVP:** multiple providers (ported capacity broker), MCP tools, Cedar policies, approvals, secrets, CLI, basic dashboard, TypeScript SDK, Docker Compose self-hosting.
3. **The USP:** workspace snapshots (overlayfs/btrfs, Firecracker), then fork, rewind and diff, then replay as evals, then ACP harness adapters (Claude Code, Codex).
4. **Trust:** signed audit checkpoints anchored outside the system, Biscuit delegation for subagents, Firecracker sandbox pools.
5. **Cloud and ecosystem:** multi-tenancy, triggers, integrations, cold storage, metering, and the "virtual company" template from legacy Shiioo.
