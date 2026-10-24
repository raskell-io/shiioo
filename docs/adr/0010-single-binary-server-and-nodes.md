# 0010. One binary: a server, nodes that dial out, an embedded store

- Status: Accepted
- Date: 2026-10-24
- Supersedes the cluster parts of [ADR-0003](0003-redb-default-postgres-for-clusters.md) (Postgres stays available but is no longer a roadmap item)

## Context

The system runs on one always-on Mac today and may grow to a few more machines (inference boxes, laptops, CI hosts). Running Postgres or a container stack for Shiioo itself adds operations work the owner doesn't want, and the write rates (a few events per second per run) don't need it. The owner asked for a single Rust binary with a server/client shape like HashiCorp Nomad.

## Decision

- **One executable, `shiioo`, three roles.** `shiioo server` holds the event log (redb), the scheduler, rules and approvals, capacity, the API and the Telegram bot. `shiioo node` runs harnesses, tools and sandboxes on a machine and **dials out** to the server over WebSocket + TLS (over Tailscale in practice). `shiioo up` runs both in one process. The CLI is the same binary.
- **The embedded store is the store.** redb on the server is the only source of truth. Nodes keep only their call journals. The Postgres backend remains behind a feature flag for anyone who wants it, with no further investment.
- **Sandboxes are optional layers for agents**, never a requirement for running Shiioo. The `worktree` level needs nothing installed.

## Consequences

- Installing Shiioo is copying one file and running `shiioo up`.
- The server is a single point of failure. That is acceptable for one owner: the log is durable, a restart resumes every run, and the server's data directory is one file to back up.
- Scaling out means adding nodes, not servers. Work scheduling is per node capacity (harness slots, inference endpoints), announced by each node when it connects.
- The daemon ↔ gateway protocol from [ADR-0006](0006-daemon-dials-out-over-websocket.md) is the node ↔ server protocol; "daemon" is renamed "node".

## Revisit when

More than a handful of nodes, or several owners, make a replicated server worth its cost. Even then the first step is a second server with log replication, not Postgres.
