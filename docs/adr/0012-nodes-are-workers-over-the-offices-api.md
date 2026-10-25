# 0012. Nodes are workers over the office's HTTP API

- Status: Accepted; supersedes the transport and framing of [0006](0006-daemon-dials-out-over-websocket.md), keeps its dial-out rule
- Date: 2026-10-25

## Context

Phase 6 adds other machines to the office: a DGX Spark for local models and GPU work, a second Mac, a Linux box with containers. [ADR-0006](0006-daemon-dials-out-over-websocket.md) designed a node as a *tool executor*: the server's worker calls the model, and tool invocations travel to the node over a WebSocket with protobuf frames and a journal on the node.

Since then [ADR-0009](0009-harness-first-runtimes.md) made harnesses the primary runtime. A harness run is one long process (Claude Code, Codex) that executes its own tools where it runs; there is no tool-by-tool traffic to carry, and the worker's work is to start the process in the right worktree, record what it prints, and hold the lease. The thing that needs to be on the other machine is therefore the *worker*, not a tool executor.

## Decision

1. **A node is a worker on another machine.** `shiioo node` runs the same `Worker` as the office, against a `RemoteStore` that implements the store trait over the office's HTTP API: `POST /v1/runs/{id}/lease`, `…/lease/renew`, `…/events` (append under the lease), `GET …/events`, `GET /v1/runs`. Store errors travel in a JSON form that keeps their kind, so a node worker is fenced, expires and conflicts exactly like a local one. The office's record stays the only state; the node keeps a journal, worktrees, sandboxes and its own secrets, nothing about runs.
2. **The node dials out.** It long-polls `GET /v1/machines/{name}/work` for runs addressed to it and heartbeats `POST /v1/machines/{name}/heartbeat`; the office never connects to it. Over Tailscale that is HTTPS to the office with no port opened anywhere, which was the point of ADR-0006.
3. **Runs are addressed by a machine selector** in the spec (`machine: "spark"` or `"gpu=true"`), set from the phone (`/run on:spark …`), the API, a routine or a role. The office does not drive such a run; the matching node takes its lease. A run with no selector runs in the office.
4. **Enrollment is a one-time token** the owner makes (`/enroll <name>`, `shiioo machines token`, `POST /v1/machines/enrollments`), exchanged on the node for a long-lived machine token (`shiioo node enroll`). Both are stored hashed in `.shiioo/machines.json`; revoking a machine deletes its record and its token stops working at once. A machine's token is accepted everywhere the office's is: the owner's machines are the office.
5. **Repositories are resolved by label.** A run's spec carries the office's repository path and its label; a node maps the label to its own clone through its `[repos]`. A node without that label fails the run with a clear reason instead of guessing.
6. **The permission hook works unchanged**: the node's `shiioo hook pre-tool-use` posts to the office with the machine token; the rules and the owner's approvals apply to a run on a node as to one in the office. Isolation on a node follows the run's spec (the office's policy) with the node's own engine.

## Consequences

- No protobuf, no WebSocket framing, no second protocol to version: the node speaks the API the dashboard and the hook already speak. ADR-0006's framing, journal queries and output deltas are not needed for harness runs; if the native loop ever runs tools on a remote machine, that part of ADR-0006 can return as an addition.
- Latency per append is one HTTP round trip over the tailnet, a few milliseconds; a harness emits a few events a second at most.
- A node is as trusted as the office: its token can read every run and answer approvals. That matches one owner's machines (ADR-0008). Per-machine scopes come back with multi-owner use.
- Files a run produces on a node stay on the node; the office lists and sends deliverables only for runs it drove itself (the node has the worktree). A node-side artifact endpoint is the next step when it is needed.
- The office must be reachable from the node by URL: in practice `tailscale serve` on the office machine.

## Revisit when

- The native loop needs tools on a remote machine (then ADR-0006's tool invocation frames are the addition).
- More than one owner shares an office (then machine tokens get scopes).
- Per-run deliverables from nodes are wanted on the phone (then the node serves them, or uploads them to the office at the end of a run).
