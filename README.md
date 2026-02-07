<div align="center">

# Shiioo

**Virtual Company OS**

*Run a virtual company of AI employees with business processes, governance, and transparent event-sourced persistence.*

[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)
[![License](https://img.shields.io/badge/license-BSL%201.1-blue.svg)](LICENSE)
[![Phase](https://img.shields.io/badge/CLI%20phase-4%20(tool%20use)-green.svg)](https://github.com/raskell-io/shiioo)

[Documentation](#quick-start) · [SDK](#rust-sdk) · [API Reference](#api-endpoints) · [Contributing](#community)

</div>

---

## Quick Start

```bash
# Build
cargo build --release

# Start the CLI — talk to your Chief of Staff
export ANTHROPIC_API_KEY=sk-ant-...
./target/release/shiioo

# Or start the API server
export SHIIOO_ENCRYPTION_KEY=$(openssl rand -base64 32 | head -c 32)
./target/release/shiioo serve

# Visit the dashboard
open http://localhost:8080
```

### CLI (Chief of Staff)

The `shiioo` command launches an interactive REPL where you talk to your Chief of Staff — an AI assistant that manages your virtual company. It can look up employees, hire new ones, delegate tasks, and report on company status using real tools.

```
CEO > list my employees
Chief of Staff
  [calling list_employees]
  [list_employees done] Found 3 employee(s):
...

CEO > hire a frontend engineer named Alice on the engineering team
Chief of Staff
  [calling hire_employee] name=Alice, description=Frontend Engineer, team=engineering
  [hire_employee done] Hired Alice (id: abc-123) on team 'engineering'.
...
```

Slash commands: `/help`, `/status`, `/employees`, `/cost`, `/model <name>`, `/clear`, `/quit`

### API Server

The server starts with an embedded web dashboard, GraphQL API, and real-time monitoring at `http://localhost:8080`.

---

## Features

| Feature | Description |
|---------|-------------|
| **CLI REPL** | Talk to your Chief of Staff — an agentic AI assistant with tool use |
| **Tool Use** | 7 built-in tools: list/get/hire employees, delegate tasks, check budgets, company status |
| **LLM Streaming** | Token-by-token SSE streaming via Anthropic Messages API with multi-source capacity |
| **DAG Workflows** | Define multi-step workflows with dependencies, retries, and timeouts |
| **Employee Primitive** | Virtual employees with skills, policies, MCP bindings, and budgets |
| **Employee Runtime** | Execute employee tasks with tool calls, budget tracking, and policy enforcement |
| **Employee Orchestration** | Departments, delegation, supervision with restart policies |
| **Event Sourcing** | Immutable event log with complete audit trail and time-travel replay |
| **Role-Based Access** | Fine-grained RBAC with permissions, approval gates, and budgets |
| **MCP Tool Server** | Expose enterprise tools to agent clients with policy enforcement |
| **Capacity Pooling** | Multi-source LLM capacity with rate limits, failover, and cost tracking |
| **Cron Scheduler** | Recurring workflows with execution history and enable/disable controls |
| **Executive Review Boards** | Multi-person approvals with quorum rules (unanimous, majority, count, %) |
| **Multi-Tenancy** | Isolated storage, quotas, and resource limits per tenant |
| **Cluster Management** | Distributed locking, leader election, and node health tracking |
| **Secret Management** | Encrypted storage with rotation policies and version history |
| **GraphQL API** | Complete Query, Mutation, and Subscription support with Playground |
| **Real-Time Dashboard** | Live metrics, workflow visualization, and audit log monitoring |
| **Compliance Ready** | SOC2, GDPR, tamper-proof audit logs with chain integrity verification |
| **Rust SDK** | Type-safe client library with async support and WebSocket subscriptions |

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

Shiioo is a single Rust binary with two modes:

- **CLI REPL** (`shiioo`) — Interactive Chief of Staff with tool-use agentic loop
- **API Server** (`shiioo serve`) — RESTful + GraphQL with WebSocket subscriptions

Both share the same core engine:

- **Workflow Engine** — DAG execution with retries, timeouts, idempotency
- **Policy Engine** — Authorization, governance, and approval workflows
- **LLM Client** — Anthropic Messages API with SSE streaming and tool use
- **Capacity Broker** — Routes LLM calls across multiple API keys/providers
- **MCP Tool Server** — Enterprise tools exposed to agent clients
- **Scheduler** — Cron + queue for recurring workflows
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
- **Employees**: `/api/employees` (alias for `/api/agents`), `/api/job-titles` (alias for `/api/archetypes`)
- **Observability**: `/api/metrics`, `/api/analytics/*`, `/api/health/status`
- **Multi-Tenancy**: `/api/tenants`, `/api/cluster/*`
- **Secrets**: `/api/secrets`, `/api/secrets/{id}/rotate`
- **Security**: `/api/audit/*`, `/api/rbac/*`, `/api/compliance/report`, `/api/security/scan`

See GraphQL Playground at `http://localhost:8080/api/graphql` for interactive schema exploration.

---

## Rust SDK

Add the SDK to your `Cargo.toml`:

```toml
[dependencies]
shiioo-sdk = { git = "https://github.com/raskell-io/shiioo" }
tokio = { version = "1", features = ["full"] }
```

### Basic Usage

```rust
use shiioo_sdk::{ShiiooClient, ShiiooResult};

#[tokio::main]
async fn main() -> ShiiooResult<()> {
    // Build client
    let client = ShiiooClient::builder()
        .base_url("http://localhost:8080")
        .api_key("sk-your-api-key")
        .build()?;

    // Check health
    let health = client.health().check().await?;
    println!("Server status: {}", health.status);

    // List workflow runs
    let runs = client.runs().list().await?;
    println!("Found {} runs", runs.len());

    // Create and execute a job
    let response = client.jobs().create(CreateJobRequest {
        name: "Code Review".to_string(),
        workflow: my_workflow,
        execute: Some(true),
        ..Default::default()
    }).await?;

    println!("Created job: {}", response.job_id);
    Ok(())
}
```

### WebSocket Subscriptions

```rust
use shiioo_sdk::{ShiiooClient, stream::SubscriptionEvent};

let client = ShiiooClient::builder()
    .base_url("http://localhost:8080")
    .build()?;

// Subscribe to real-time updates
let mut sub = client.subscribe().await?;
sub.subscribe_all().await?;

while let Some(event) = sub.next_event().await {
    match event? {
        SubscriptionEvent::WorkflowUpdate { run_id, status, .. } => {
            println!("Workflow {} is now {}", run_id, status);
        }
        SubscriptionEvent::StepUpdate { step_id, status, .. } => {
            println!("Step {} completed with status {}", step_id, status);
        }
        _ => {}
    }
}
```

### Available APIs

| API | Methods |
|-----|---------|
| `client.health()` | `check()`, `status()` |
| `client.runs()` | `list()`, `get()`, `events()` |
| `client.jobs()` | `create()` |
| `client.roles()` | `list()`, `get()`, `create()`, `delete()` |
| `client.policies()` | `list()`, `get()`, `create()`, `delete()` |
| `client.organizations()` | `list()`, `get()`, `create()`, `delete()` |
| `client.templates()` | `list()`, `get()`, `create()`, `delete()`, `instantiate()` |
| `client.capacity()` | `sources()`, `usage()`, `cost()` |
| `client.routines()` | `list()`, `get()`, `create()`, `enable()`, `disable()` |
| `client.approvals()` | `list()`, `get()`, `vote()` |
| `client.secrets()` | `list()`, `get()`, `create()`, `rotate()`, `versions()` |
| `client.tenants()` | `list()`, `get()`, `register()`, `suspend()`, `activate()` |
| `client.cluster()` | `nodes()`, `leader()`, `health()` |
| `client.audit()` | `entries()`, `statistics()`, `verify_chain()` |
| `client.rbac()` | `roles()`, `assign_role()`, `check_permission()` |
| `client.compliance()` | `generate_report()` |
| `client.security()` | `scan()` |

---

## Configuration

Create `shiioo.toml` in the working directory:

```toml
[storage]
blob_dir = "blobs"
event_log_dir = "events"
index_file = "index.redb"
```

### Environment Variables

| Variable | Required | Description |
|----------|----------|-------------|
| `ANTHROPIC_API_KEY` | **Yes** (CLI) | Anthropic API key for the Chief of Staff. Without it, the CLI uses mock responses |
| `SHIIOO_ENCRYPTION_KEY` | **Yes** (server) | 32-byte key for encrypting secrets (AES-256) |
| `SHIIOO_DATA_DIR` | No | Data directory (default: `./data`) |
| `SHIIOO_PORT` | No | Server port (default: `8080`) |
| `SHIIOO_RATE_LIMIT_PER_SECOND` | No | API rate limit per second (default: `10`) |
| `SHIIOO_RATE_LIMIT_BURST` | No | Maximum burst size for rate limiting (default: `50`) |
| `SHIIOO_LOG_JSON` | No | Enable JSON log format for production (default: `false`) |
| `RUST_LOG` | No | Log level (default: `info`) |

Generate an encryption key:

```bash
# Generate a 32-byte key
openssl rand -base64 32 | head -c 32

# Or use a password-derived key
echo -n "your-secure-password-here!!!" | head -c 32
```

Run with required configuration:

```bash
export SHIIOO_ENCRYPTION_KEY=$(openssl rand -base64 32 | head -c 32)
export SHIIOO_DATA_DIR=./data
./target/release/shiioo-server
```

---

## Development

### Project Structure

```
shiioo/
├── crates/
│   ├── core/          # Domain types, storage, workflow engine, policy engine, LLM client
│   ├── server/        # API server, GraphQL, dashboard UI, CLI REPL
│   │   └── src/cli/   # REPL, conversation, renderer, tool definitions
│   ├── mcp/           # MCP tool server (JSON-RPC over stdio)
│   └── sdk/           # Rust SDK client library
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

**CLI Phase 4 Complete** ✅

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
- ✅ Phase 11: Rust SDK & Client Libraries
- ✅ Phase 12: Agent Primitive with archetypes, storage, and migration
- ✅ Phase 13-14: Agent runtime and orchestration
- ✅ Phase 15: Testing infrastructure (integration + property-based tests)
- ✅ Phase 16: Documentation and API refinement

**CLI Phases:**
- ✅ CLI Phase 1: REPL skeleton with slash commands
- ✅ CLI Phase 2: Real LLM integration via Anthropic API
- ✅ CLI Phase 3: Token-by-token streaming
- ✅ CLI Phase 4: Tool use — 7 tools with agentic loop (up to 10 rounds)

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
