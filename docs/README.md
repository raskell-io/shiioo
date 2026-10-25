# Shiioo documentation

Shiioo is a **Virtual Company OS**: a staff of AI agents that work on your own machines around the clock, which you direct and supervise from your phone.

## Start here

- [What Shiioo is](positioning.md): the product definition, principles, topology, and the roadmap. Read this first.
- [Architecture overview](architecture.md): the one rule, the guarantees, components, storage, crate map.
- [Glossary](glossary.md): platform terms, and how the "virtual company" vocabulary maps onto them.
- [Contributing](contributing.md): building, testing, conventions, how to add things.
- [Backlog](backlog.md): known rough edges to fix or consciously keep.

## Design

Detailed designs, one per area. Each section is marked *(built)*, *(planned)* or *(deferred, ADR-n)*. Where a design doc and the code disagree, the code wins and the doc is wrong: please fix it.

| Document | Covers |
|---|---|
| [Event log](design/event-log.md) | Event format, hashing over stored bytes, the payload catalogue, external input, blobs, schema evolution, backends, archival, streaming |
| [Execution](design/execution.md) | Run lifecycle, scheduling, the worker loop, leases, cancellation, subagents, recovery, failure modes, simulation |
| [Tools and machines](design/tools-and-machines.md) | Tool model, built-in and MCP tools, nodes and their protocol, isolation levels, worktrees and snapshots, secrets |
| [Runtimes and providers](design/runtimes-and-providers.md) | The native loop, the Claude Code and Codex harness drivers, ACP (planned), model providers and the router, context building |
| [Governance](design/governance.md) | The owner's rules, approvals, budgets, change management, the virtual-company template |
| [Fork and replay](design/fork-and-replay.md) | Rewind, fork, diff, replay and evals |
| [API](design/api.md) | REST and SSE, employees, permissions and approvals, capacity, diff, the dashboard, errors, auth |
| [Telegram](design/telegram.md) | The owner's phone: commands, approvals with one tap, notifications, security of the bot |
| [Intake](design/intake.md) | Work arriving as labelled GitHub issues: the label protocol, the task prompt, dedup and state, failure handling |
| [Security](design/security.md) | Threat model, controls, today's posture (isolation levels, egress, secrets and redaction), and what is not protected |
| [Deployment](design/deployment.md) | The office on one Mac (`install`, keep-awake, Tailscale), several machines, config, observability, upgrades, backups |

## Crates

Each crate carries its own `README.md` (purpose, API, how to test) and a `docs/` directory for maintainers.

| Crate | Docs |
|---|---|
| `shiioo-core` | [README](../crates/shiioo-core/README.md), [docs](../crates/shiioo-core/docs) |
| `shiioo-store` | [README](../crates/shiioo-store/README.md), [docs](../crates/shiioo-store/docs) |
| `shiioo-runtime` | [README](../crates/shiioo-runtime/README.md), [docs](../crates/shiioo-runtime/docs) |
| `shiioo-secrets` | [README](../crates/shiioo-secrets/README.md) |
| `shiioo-rules` | [README](../crates/shiioo-rules/README.md), [docs](../crates/shiioo-rules/docs) |
| `shiioo-providers` | [README](../crates/shiioo-providers/README.md), [docs](../crates/shiioo-providers/docs) |
| `shiioo-api` | [README](../crates/shiioo-api/README.md), [docs](../crates/shiioo-api/docs) |
| `shiioo-telegram` | [README](../crates/shiioo-telegram/README.md), [docs](../crates/shiioo-telegram/docs) |
| `shiioo-intake` | [README](../crates/shiioo-intake/README.md), [docs](../crates/shiioo-intake/docs) |
| `shiioo-sim` | [README](../crates/shiioo-sim/README.md), [docs](../crates/shiioo-sim/docs) |
| `shiioo` (binary) | [README](../crates/shiioo/README.md), [docs](../crates/shiioo/docs) |

## Decisions

[Architecture decision records](adr/README.md): why things are the way they are, and when to revisit.

## History

- [Porting from shiioo-legacy](legacy-port.md): what is kept, replaced or dropped from the previous codebase.
