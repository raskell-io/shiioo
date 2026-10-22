# 0002. Run ownership through leases with fencing tokens

- Status: Accepted
- Date: 2026-10-22

## Context

Exactly one worker should drive a run at a time. Leases with timeouts give liveness (a dead worker's run is picked up again), but a timeout can't tell a dead worker from a slow one. A worker paused by GC, a VM migration or a network partition can wake up after its lease expired and someone else took over. If its writes are accepted, two workers interleave decisions in one run: duplicated tool calls, contradictory messages, a corrupted log.

Distributed locks alone (Redis locks, in-memory locks) do not solve this, because the stale holder doesn't know it lost the lock.

## Decision

- A worker must hold a lease to append to a run. Each lease grant gets an `epoch` strictly greater than every earlier one for that run.
- The store checks the presented epoch against the current one on **every** append and renewal, in the same transaction as the write. Stale epochs get `Fenced`; expired leases get `LeaseExpired`.
- An expired lease stays the current epoch until someone takes over, so its holder stays fenced; it must re-acquire, which bumps the epoch and forces it to re-read the log.
- Every lease grant is itself an event (`LeaseAcquired`), so takeovers are visible in the log.
- In a cluster, lease expiry is decided by the database clock, not the worker's.
- Appends also name the `seq` they expect to write (`SeqConflict` otherwise), so a worker with a stale view of the log can't append even within its own lease.

The rules live in `shiioo-core::RunHead`; every backend calls them inside its own transaction and runs the same conformance tests.

## Consequences

- Split-brain writes are impossible by construction, regardless of pauses, partitions or clock skew between workers.
- A worker that loses its lease must stop immediately and discard in-flight work. Work done after the lease was lost (for example a model call) is wasted, but never recorded.
- Side effects that happen outside the store (a tool already executing on a machine) are not fenced by this. They are handled by the tool intent/result protocol and the daemon's call journal (see [design/execution.md](../design/execution.md)).
- Every write costs a lock on the run's head (a row lock in Postgres, the single writer in redb).

## Revisit when

Per-run write throughput becomes a bottleneck, which would point to a design problem elsewhere first.
