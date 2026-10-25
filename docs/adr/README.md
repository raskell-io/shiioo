# Architecture decision records

Each ADR records one decision: the context, what we chose, what we gave up, and when to revisit it. ADRs are never edited after they are accepted, except to change their status. A decision that changes gets a new ADR that supersedes the old one.

| # | Decision | Status |
|---|---|---|
| [0001](0001-event-log-is-the-only-state.md) | The per-run event log is the only state | Accepted |
| [0002](0002-leases-with-fencing.md) | Run ownership through leases with fencing tokens | Accepted |
| [0003](0003-redb-default-postgres-for-clusters.md) | redb by default, Postgres for clusters | Accepted, cluster part superseded by 0010 |
| [0004](0004-rest-and-sse-api.md) | REST + resumable SSE as the only public API | Accepted |
| [0005](0005-cedar-policies-and-biscuit-credentials.md) | Cedar for policies, Biscuit for agent credentials | Accepted |
| [0006](0006-daemon-dials-out-over-websocket.md) | Machines run a daemon that dials out over WebSocket | Accepted; transport and framing superseded by 0012, the dial-out rule stands |
| [0007](0007-rewrite-under-apache-2.md) | Rewrite Shiioo from scratch under Apache-2.0 | Accepted |
| [0008](0008-personal-workforce-not-managed-cloud.md) | The product is a personal agent workforce, not a managed agent cloud | Accepted |
| [0009](0009-harness-first-runtimes.md) | Harnesses are the primary runtime; the native loop serves local models | Accepted |
| [0010](0010-single-binary-server-and-nodes.md) | One binary: a server, nodes that dial out, an embedded store | Accepted |
| [0011](0011-containers-through-the-owners-engine.md) | Isolation through the owner's container engine, egress through a proxy sidecar, secrets sealed locally | Accepted |
| [0012](0012-nodes-are-workers-over-the-offices-api.md) | Nodes are workers over the office's HTTP API | Accepted |

## Template

```markdown
# NNNN. Title

- Status: Proposed | Accepted | Superseded by NNNN
- Date: YYYY-MM-DD

## Context
What forces are at play, and why a decision is needed now.

## Decision
What we chose, stated so it can be checked.

## Consequences
What gets easier, what gets harder, what we gave up.

## Revisit when
The signal that would make us reconsider.
```
