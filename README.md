<div align="center">

# Shiioo

**Agentic Enterprise Orchestrator**

*Run a virtual enterprise of LLM agents with workflows, governance, and transparent event-sourced persistence.*

[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-BSL%201.1-blue.svg)](LICENSE)
[![Phase](https://img.shields.io/badge/phase-10%20complete-green.svg)](https://github.com/raskell-io/shiioo)

[Documentation](#quick-start) · [API Reference](#api-endpoints) · [Contributing](#community)

</div>

---

## Quick Start

```bash
# Build and run
cargo build --release
./target/release/shiioo-server

# Or with mise
mise run build
mise run serve

# Visit the dashboard
open http://localhost:8080
```

The server starts with an embedded web dashboard, GraphQL API, and real-time monitoring at `http://localhost:8080`.

---

## Features

| Feature | Description |
|---------|-------------|
| **DAG Workflows** | Define multi-step workflows with dependencies, retries, and timeouts |
| **Event Sourcing** | Immutable event log with complete audit trail and time-travel replay |
| **Role-Based Access** | Fine-grained RBAC with permissions, approval gates, and budgets |
| **MCP Tool Server** | Expose enterprise tools to agent clients with policy enforcement |
| **Capacity Pooling** | Multi-source LLM capacity with rate limits, failover, and cost tracking |
| **Cron Scheduler** | Recurring workflows with execution history and enable/disable controls |
| **Approval Boards** | Multi-person approvals with quorum rules (unanimous, majority, count, %) |
| **Multi-Tenancy** | Isolated storage, quotas, and resource limits per tenant |
| **Cluster Management** | Distributed locking, leader election, and node health tracking |
| **Secret Management** | Encrypted storage with rotation policies and version history |
| **GraphQL API** | Complete Query, Mutation, and Subscription support with Playground |
| **Real-Time Dashboard** | Live metrics, workflow visualization, and audit log monitoring |
| **Compliance Ready** | SOC2, GDPR, tamper-proof audit logs with chain integrity verification |

---

## Why Shiioo?

Running LLM agents in production means managing workflows, approvals, budgets, and compliance. Most solutions are chat-first tools with workflows bolted on. Shiioo inverts this: it's a **durable workflow engine** with agent steps, built for enterprise governance from day one.

- **You stay in control** — Approval gates, budgets, and policy enforcement are first-class features
- **Event-sourced truth** — Every run emits an immutable event stream; all state is rebuildable
- **Transparent persistence** — Context stored as inspectable artifacts (JSONL, gzipped, content-addressed)
- **Governed self-configuration** — Agents can propose changes, but changes follow enterprise change management
- **Single binary** — One Rust server with embedded UI; no microservice sprawl

Shiioo enables you to define **roles**, **processes**, **jobs**, **routines**, and **policies** while the platform executes workflows via **MCP tools** with **transparent, replayable context logs** backed by blob storage and an **org-wide capacity pool** to mitigate model rate limits.

---

## Design Principles

- **Workflow-first, not chat-first** — Agents execute inside workflows as steps in a DAG
- **Event-sourced by design** — Immutable logs enable complete audit trails and time-travel debugging
- **Explicit over implicit** — Approvals, budgets, and policies are declared, not inferred
- **Transparent storage** — All artifacts are inspectable (JSONL, content-addressed blobs)
- **API-first and GitOps-friendly** — Everything accessible via API; configuration managed in Git
- **Production-grade from day one** — Multi-tenancy, RBAC, compliance, secrets, HA built-in

---

## Architecture

Shiioo is a single Rust binary that runs:

- **API Server** — RESTful + GraphQL with WebSocket subscriptions
- **Workflow Engine** — DAG execution with retries, timeouts, idempotency
- **Policy Engine** — Authorization, governance, and approval workflows
- **MCP Tool Server** — Enterprise tools exposed to agent clients
- **Scheduler** — Cron + queue for recurring workflows
- **Capacity Broker** — Routes LLM calls across multiple API keys/providers
- **Web Dashboard** — Real-time monitoring with embedded static assets

### Storage Model

**Blob Store (S3 or filesystem):**
```
events/YYYY/MM/DD/<run_id>.jsonl.gz  # Append-only event stream
blobs/XX/<hash>                       # Content-addressed payloads
```

**Index DB (redb):**
- Run metadata and status
- Fast queries without scanning events
- Rebuildable from event log if lost

---

## API Endpoints

### GraphQL API
- `POST /api/graphql` — Queries and mutations
- `GET /api/graphql` — Interactive GraphQL Playground
- `WS /api/graphql/ws` — WebSocket subscriptions (graphql-transport-ws)
- `GET /` or `/dashboard` — Real-time web dashboard

### REST API
- **Workflows**: `/api/runs`, `/api/jobs`
- **Roles & Policies**: `/api/roles`, `/api/policies`
- **Organization**: `/api/organizations`, `/api/templates`
- **Capacity**: `/api/capacity/sources`, `/api/capacity/usage`, `/api/capacity/cost`
- **Automation**: `/api/routines`, `/api/approval-boards`, `/api/approvals`, `/api/config-changes`
- **Observability**: `/api/metrics`, `/api/analytics/*`, `/api/health/status`
- **Multi-Tenancy**: `/api/tenants`, `/api/cluster/*`
- **Secrets**: `/api/secrets`, `/api/secrets/{id}/rotate`
- **Security**: `/api/audit/*`, `/api/rbac/*`, `/api/compliance/report`, `/api/security/scan`

See GraphQL Playground at `http://localhost:8080/api/graphql` for interactive schema exploration.

---

## Configuration

Create `shiioo.toml` in the working directory:

```toml
[storage]
blob_dir = "blobs"
event_log_dir = "events"
index_file = "index.redb"
```

Or use environment variables:

```bash
SHIIOO_DATA_DIR=./data
SHIIOO_PORT=8080
RUST_LOG=info
./target/release/shiioo-server
```

---

## Development

### Project Structure

```
shiioo/
├── crates/
│   ├── core/          # Domain types, storage, workflow engine, policy engine
│   ├── server/        # API server, GraphQL, dashboard UI
│   └── mcp/           # MCP tool server (JSON-RPC over stdio)
├── .mise.toml         # Task and dependency management
└── Cargo.toml         # Workspace manifest
```

### Available Tasks

```bash
mise run build        # Build release binary
mise run test         # Run all tests
mise run check        # Check code without building
mise run fmt          # Format code
mise run clippy       # Run linter
mise run run          # Run server (release)
mise run run-dev      # Run with debug logging
mise run dev          # Full dev build with checks
mise run ci           # CI pipeline (fmt-check, clippy, test)
mise run pre-commit   # Pre-commit checks
```

See `mise tasks` for full list.

### Run Tests

```bash
cargo test
# or
mise run test
```

### Storage Inspection

All data is stored in transparent, inspectable formats:

```bash
# View event log for a run
ls data/events/2026/01/07/
zcat data/events/2026/01/07/<run-id>.jsonl.gz | jq

# View a blob by hash
cat data/blobs/ab/<full-hash>
```

---

## Current Status

**Phase 10 Complete** ✅

All core features are production-ready:

- ✅ Phase 0: Core infrastructure
- ✅ Phase 1: DAG workflow execution
- ✅ Phase 2: MCP tools + policy engine
- ✅ Phase 3: Organization management + templates
- ✅ Phase 4: Capacity broker
- ✅ Phase 5: Automation & governance (routines, approvals)
- ✅ Phase 6: Real-time monitoring & observability
- ✅ Phase 7: Multi-tenancy & high availability
- ✅ Phase 8: Advanced features (secrets, parallel-for-each, conditionals)
- ✅ Phase 9: Enhanced security & compliance (audit logs, RBAC)
- ✅ Phase 10: UI & Developer Experience (GraphQL, dashboard)

**Next:** Phase 11 — Rust SDK & Client Libraries

---

## Community

💬 [Discussions](https://github.com/raskell-io/shiioo/discussions) — Questions, ideas, and feedback
🐛 [Issues](https://github.com/raskell-io/shiioo/issues) — Bug reports and feature requests
🤝 [Contributing](CONTRIBUTING.md) — Contribution guidelines

Contributions are welcome! See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines.

---

## License

**Business Source License 1.1**

Shiioo is licensed under the [Business Source License 1.1](LICENSE). This allows you to:
- Use Shiioo for internal production workflows
- Deploy Shiioo to execute your own organization's workflows and jobs
- Build workflow platforms with abstraction layers on top of Shiioo

The license **does not** permit running a public managed service where third parties can register and execute their own workflows through Shiioo's APIs.

**Change Date:** 4 years after each release
**Change License:** Apache License, Version 2.0

After the Change Date, Shiioo automatically becomes Apache 2.0 licensed.

---

<div align="center">

**Built with Rust** 🦀

*Production-ready enterprise orchestration for LLM agents*

</div>
