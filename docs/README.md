# Shiioo documentation

Shiioo is a **Virtual Company OS**: a staff of AI agents that work on your own machines around the clock, which you direct and supervise from your phone.

## Start here

- [What Shiioo is](positioning.md): the product definition, principles, topology, and the roadmap. Read this first.
- [Architecture overview](architecture.md): the one rule, the guarantees, components, storage, crate map and roadmap.
- [Glossary](glossary.md): platform terms, and how the "virtual company" vocabulary maps onto them.

## Design

Detailed designs, one per area. Each section is marked *built* or *planned*.

| Document | Covers |
|---|---|
| [Event log](design/event-log.md) | Event format, hashing, payload types, blobs, schema evolution, backends, archival, streaming |
| [Execution](design/execution.md) | Run lifecycle, scheduling, the worker loop, leases, cancellation, subagents, recovery, failure modes |
| [Tools and machines](design/tools-and-machines.md) | Tool model, built-in and MCP tools, the daemon and its protocol, sandboxes, workspaces, secrets |
| [Runtimes and providers](design/runtimes-and-providers.md) | The native agent loop, harness adapters (ACP), model providers, capacity broker, context building |
| [Governance](design/governance.md) | Policies, approval boards, budgets, change management, the virtual-company template |
| [Fork and replay](design/fork-and-replay.md) | Rewind, fork, diff, replay and evals |
| [API](design/api.md) | REST and SSE, resources, idempotency, auth, versioning, SDKs, MCP server, webhooks |
| [Telegram](design/telegram.md) | The owner's phone: commands, approvals with one tap, notifications, security of the bot |
| [Intake](design/intake.md) | Work arriving as labelled GitHub issues: the label protocol, the task prompt, dedup and state, failure handling |
| [Security](design/security.md) | Threat model, controls, and what is not protected |
| [Deployment](design/deployment.md) | On-device, single server, cluster; config, observability, upgrades, backups |

## Decisions

[Architecture decision records](adr/README.md): why things are the way they are, and when to revisit.

## History

- [Porting from shiioo-legacy](legacy-port.md): what is kept, replaced or dropped from the previous codebase.
