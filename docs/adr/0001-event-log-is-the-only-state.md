# 0001. The per-run event log is the only state

- Status: Accepted
- Date: 2026-10-22

## Context

An agent run lasts seconds to days, calls slow and unreliable models, and causes side effects in the outside world. Workers crash, deploys restart processes, and networks partition. Users also need to see exactly what an agent saw and did, sometimes months later, and we want to fork and replay runs.

The legacy Shiioo kept run state in memory and flushed events to JSONL files later, so a crash could lose events that had already been acted on. Its "distributed" locks lived in a process-local `HashMap`.

## Decision

Each run has an append-only, hash-chained event log. It is the only source of truth for that run. Any other state (worker memory, caches, run lists, cost totals, search indexes) is a projection that can be rebuilt from the logs and is never consulted to decide what a run does next.

Concretely:

- Every event has a contiguous `seq`, the lease `epoch` it was written under, and the hash of the previous event.
- A worker writes an event *before* acting on it: the model response before running its tools, a tool intent before dispatching it.
- Commands from outside (user messages, approvals, cancellations) enter a run only as events.

## Consequences

- Crash recovery is "read the log and continue". There is no separate state to reconcile.
- Live streaming, audit, fork, rewind, diff and replay are all reads of the same data.
- Every feature has to be expressed as events. That is extra design work up front, and event payloads become a public, versioned format that needs upcasting as it evolves.
- Writes go through one ordered log per run, so a single run can't scale writes beyond one writer. That is fine: a run is one agent's sequence of decisions.
- Logs grow without bound. Archival to object storage is required, and it is the only path that removes events from the hot store.

## Revisit when

Never for the core idea. Revisit the encoding (JSON payloads) if storage size or hashing cost becomes a measured bottleneck.
