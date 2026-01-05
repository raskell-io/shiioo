# Shiioo (CO) - Virtual Company OS

**Agentic Enterprise Orchestrator** - A human-led operating system for running a "virtual enterprise" of LLM agents with roles, processes, jobs, governance, and transparent event-sourced persistence.

## Vision

Shiioo enables you to define **roles**, **processes**, **jobs**, **routines**, and **policies** while the platform executes workflows via **MCP tools** with **transparent, replayable context logs** backed by blob storage and an **org-wide capacity pool** to mitigate model rate limits.

## Core Principles

1. **Workflow-first, not chat-first** - Agents execute inside workflows. This is a durable workflow engine with agent steps.
2. **You remain in the driver's seat** - Approval gates, budgets, and policy enforcement are first-class.
3. **Event-sourced truth** - Every run emits an immutable event stream. All derived state is rebuildable.
4. **Transparent persistence** - Context is stored in blob storage as inspectable artifacts (JSONL, gzipped, content-addressed).
5. **Governed self-configuration** - Agents can propose changes, but changes follow enterprise change management with separation of duties.
6. **API-first and GitOps-friendly** - Everything is accessible via API; configuration can be stored/managed in Git.
7. **Single-binary bias** - One Rust server binary with embedded UI. HA comes later via multi-node.

## Architecture

### Components

- **API Server** (Axum/Tower) - RESTful API with OpenAPI
- **Web UI** - Embedded static assets (CEO Console - coming soon)
- **Scheduler** - Cron + queue for routines
- **Workflow Engine** - DAG execution with retries, timeouts, idempotency (Phase 1)
- **Policy Engine** - Authorization and governance (Phase 2)
- **MCP Tool Server** - Exposes enterprise tools to agent clients (Phase 2)
- **Context Store** - Immutable event log on blob storage + content-addressed blobs
- **Capacity Broker** - Routes LLM calls across a pool of capacity sources (Phase 4)

### Storage Model

**Blob Store Layout (S3 or filesystem):**
```
events/YYYY/MM/DD/<run_id>.jsonl.gz  # Append-only event stream
blobs/XX/<hash>                       # Content-addressed payloads (XX = first 2 chars)
```

**Index DB (redb):**
- Run metadata and status
- Fast queries without scanning events
- Rebuildable from event log if lost

## Current Status (Phase 5 Complete ✅)

### Implemented
- ✅ **Phase 0**: Core infrastructure
  - Rust workspace with 3 crates (core, server, mcp)
  - Core data types (Run, Step, Event, Role, Policy, WorkflowSpec)
  - Event types for event sourcing (30+ event types)
  - Blob storage abstraction (filesystem + S3-compatible)
  - Event log with JSONL + gzip compression
  - redb-based index store for fast queries
  - Axum API server with health check
  - Embedded UI placeholder
  - Transparent storage (all artifacts are inspectable)

- ✅ **Phase 1**: DAG workflow execution engine
  - **petgraph-based DAG executor** with topological sorting
  - **Dependency resolution** and parallel execution support
  - **Step executor** with full event logging
  - **Retry logic** with exponential backoff
  - **Timeout handling** per step
  - **Idempotency keys** (run_id, step_id, attempt)
  - **Event emission** for all state transitions
  - **Cancellation support** for running workflows
  - **Content-addressed artifact storage** for prompts/responses
  - **Full API integration** - jobs execute end-to-end

- ✅ **Phase 2**: MCP tools + policy enforcement
  - **MCP server** with JSON-RPC 2.0 over stdio
  - **Tool registry** with tiered security (Tier 0-2)
  - **5 MCP tools**: context_get, context_search, context_events, repo_read, web_fetch
  - **Policy engine** for governance and authorization
  - **Role-based access control** with tool allowlists
  - **Budget tracking** - daily token and cost limits per role
  - **Approval requirements** - tool-level and tier-level
  - **Policy rules** - deny paths, domain allowlists, approval rules
  - **Persistent storage** for roles and policies in redb
  - **Full API** for role and policy management

- ✅ **Phase 3**: Organization & templates
  - **Organization management** - teams, people, org chart, reporting structure
  - **Organization validation** - cycle detection, referential integrity
  - **Process templates** - reusable workflow patterns with parameters
  - **Template instantiation** - parameter replacement and validation
  - **Claude config compiler** - generate `.claude/config.json` from org setup
  - **GitOps-friendly** - JSON/YAML org config, version-controlled
  - **Org queries** - management chain, team members, approval permissions
  - **Full API** for orgs, templates, and config compilation

- ✅ **Phase 4**: Capacity broker
  - **Multi-source capacity pooling** - support multiple API keys/providers
  - **Automatic source selection** - priority-based routing with fallback
  - **Rate limit handling** - per-minute, per-day limits with rolling windows
  - **Exponential backoff** - automatic retry with configurable backoff
  - **Priority queue system** - high-priority workflows get capacity first
  - **Cost tracking** - real-time token usage and cost monitoring
  - **Provider support** - Anthropic, OpenAI, Azure, custom endpoints
  - **Persistent storage** - capacity sources and usage history in redb
  - **Full API** for capacity management and cost reporting

- ✅ **Phase 5**: Automation & governance
  - **Cron scheduler** - recurring workflows with simplified cron expression parser
  - **Routine management** - enable/disable, execution history tracking
  - **Approval boards** - multi-person approval workflows with quorum rules
  - **Quorum rules** - Unanimous, Majority, MinCount, Percentage
  - **Config change management** - propose, approve, and apply configuration changes
  - **Voting system** - Approve, Reject, Abstain with automatic status resolution
  - **Governance workflow** - separation of duties for critical changes
  - **Audit trail** - all approvals and changes logged with timestamps
  - **Persistent storage** - routines, boards, approvals, and changes in redb
  - **Full API** for routine, approval, and config change management

### API Endpoints
**Workflow Management:**
  - `GET /api/health` - Health check
  - `GET /api/runs` - List all runs
  - `GET /api/runs/{run_id}` - Get run details
  - `GET /api/runs/{run_id}/events` - Get run events
  - `POST /api/jobs` - Create and execute a job

**Role Management:**
  - `GET /api/roles` - List all roles
  - `GET /api/roles/{role_id}` - Get role details
  - `POST /api/roles` - Create or update a role
  - `DELETE /api/roles/{role_id}` - Delete a role

**Policy Management:**
  - `GET /api/policies` - List all policies
  - `GET /api/policies/{policy_id}` - Get policy details
  - `POST /api/policies` - Create or update a policy
  - `DELETE /api/policies/{policy_id}` - Delete a policy

**Organization Management:**
  - `GET /api/organizations` - List all organizations
  - `GET /api/organizations/{org_id}` - Get organization details
  - `POST /api/organizations` - Create or update an organization
  - `DELETE /api/organizations/{org_id}` - Delete an organization

**Template Management:**
  - `GET /api/templates` - List all templates
  - `GET /api/templates/{template_id}` - Get template details
  - `POST /api/templates` - Create or update a template
  - `DELETE /api/templates/{template_id}` - Delete a template
  - `POST /api/templates/{template_id}/instantiate` - Instantiate template

**Claude Config Compiler:**
  - `GET /api/claude/compile/{role_id}` - Generate Claude config for role

**Capacity Management:**
  - `GET /api/capacity/sources` - List all capacity sources
  - `GET /api/capacity/sources/{source_id}` - Get capacity source details
  - `POST /api/capacity/sources` - Create or update a capacity source
  - `DELETE /api/capacity/sources/{source_id}` - Delete a capacity source
  - `GET /api/capacity/usage` - List capacity usage records
  - `GET /api/capacity/cost` - Get cost summary (total cost, tokens, requests)

**Routine Management (Phase 5):**
  - `GET /api/routines` - List all routines
  - `GET /api/routines/{routine_id}` - Get routine details
  - `POST /api/routines` - Create a routine
  - `DELETE /api/routines/{routine_id}` - Delete a routine
  - `POST /api/routines/{routine_id}/enable` - Enable a routine
  - `POST /api/routines/{routine_id}/disable` - Disable a routine
  - `GET /api/routines/{routine_id}/executions` - Get execution history

**Approval Board Management (Phase 5):**
  - `GET /api/approval-boards` - List all approval boards
  - `GET /api/approval-boards/{board_id}` - Get board details
  - `POST /api/approval-boards` - Create an approval board
  - `DELETE /api/approval-boards/{board_id}` - Delete an approval board

**Approval Management (Phase 5):**
  - `GET /api/approvals` - List all approvals
  - `GET /api/approvals/{approval_id}` - Get approval details
  - `POST /api/approvals/{approval_id}/vote` - Cast a vote

**Config Change Management (Phase 5):**
  - `GET /api/config-changes` - List all config changes
  - `GET /api/config-changes/{change_id}` - Get change details
  - `POST /api/config-changes` - Propose a config change
  - `POST /api/config-changes/{change_id}/apply` - Apply an approved change
  - `POST /api/config-changes/{change_id}/reject` - Reject a change

### Coming Next
- 🚧 **Phase 6**: Real-time monitoring & observability
  - Metrics and monitoring dashboard
  - Real-time workflow status updates via WebSocket
  - Performance analytics and bottleneck detection
  - Agent interaction tracing

- 🚧 **Phase 7**: Multi-tenancy & high availability
  - Multi-tenant isolation
  - Distributed execution across multiple nodes
  - Leader election and consensus
  - Cross-region replication

## Quick Start

### Build and Run

```bash
cd shiioo
cargo build --release

# Run the server
./target/release/shiioo

# Or with custom settings
./target/release/shiioo --data-dir ./my-data --port 3000
```

### Test the API

```bash
# Health check
curl http://localhost:8080/api/health

# List runs
curl http://localhost:8080/api/runs

# Create and execute a workflow
curl -X POST http://localhost:8080/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Test Workflow",
    "description": "A 3-step test workflow with dependencies",
    "workflow": {
      "steps": [
        {
          "id": "step1",
          "name": "Analyze",
          "role": "analyst",
          "action": {
            "type": "agent_task",
            "prompt": "Analyze the requirements"
          },
          "timeout_secs": 60,
          "requires_approval": false
        },
        {
          "id": "step2",
          "name": "Design",
          "role": "architect",
          "action": {
            "type": "agent_task",
            "prompt": "Design the solution"
          },
          "timeout_secs": 90,
          "retry_policy": {
            "max_attempts": 3,
            "backoff_secs": 2
          },
          "requires_approval": false
        },
        {
          "id": "step3",
          "name": "Approve",
          "role": "manager",
          "action": {
            "type": "manual_approval",
            "approvers": ["ceo", "cto"]
          },
          "requires_approval": true
        }
      ],
      "dependencies": {
        "step2": ["step1"],
        "step3": ["step2"]
      }
    },
    "execute": true
  }'

# Get run status (use run_id from previous response)
curl http://localhost:8080/api/runs/{run_id}

# Get run events (full audit trail)
curl http://localhost:8080/api/runs/{run_id}/events

# View the UI
open http://localhost:8080
```

### Storage Inspection

All data is stored in transparent, inspectable formats:

```bash
# View the event log for a run
ls data/events/2026/01/05/
zcat data/events/2026/01/05/<run-id>.jsonl.gz | jq

# View a blob by hash
cat data/blobs/ab/<full-hash>

# Inspect the index database
# (redb files can be read with the redb CLI or inspected programmatically)
```

## Configuration

Create `shiioo.toml` in the working directory:

```toml
[storage]
blob_dir = "blobs"
event_log_dir = "events"
index_file = "index.redb"
```

Or use environment variables and command-line flags to override.

## Development

### Project Structure

```
shiioo/
├── Cargo.toml                 # Workspace manifest
├── crates/
│   ├── core/                  # Core types, storage, events
│   │   ├── src/
│   │   │   ├── types.rs       # Domain types (Run, Step, Role, etc.)
│   │   │   ├── events.rs      # Event sourcing types
│   │   │   ├── storage/       # Blob, event log, index stores
│   │   │   ├── workflow.rs    # Workflow executor (stub)
│   │   │   └── policy.rs      # Policy engine (stub)
│   │   └── Cargo.toml
│   ├── server/                # API server + UI
│   │   ├── src/
│   │   │   ├── main.rs        # Entry point
│   │   │   ├── config.rs      # Configuration and app state
│   │   │   ├── api/           # API routes and handlers
│   │   │   └── ui.rs          # Embedded UI serving
│   │   └── Cargo.toml
│   └── mcp/                   # MCP server (stub for Phase 2)
│       ├── src/
│       │   ├── protocol.rs    # JSON-RPC types
│       │   ├── server.rs      # MCP server (stub)
│       │   └── tools.rs       # Tool definitions (stub)
│       └── Cargo.toml
└── README.md
```

### Run Tests

```bash
cargo test
```

### Run with Logging

```bash
RUST_LOG=debug cargo run
```

## Roadmap

### ✅ Phase 0 — Repo scaffolding (Complete)
- Rust workspace
- Core types and storage abstractions
- API skeleton
- Embedded UI placeholder

### ✅ Phase 1 — Durable workflow core (Complete)
- DAG execution engine (petgraph) ✅
- Retry and timeout logic ✅
- Idempotency keys ✅
- Run execution from job specs ✅
- Dependency resolution ✅
- Cancellation support ✅
- Full event logging ✅

### ✅ Phase 2 — MCP tools + policy (Complete)
- MCP server over stdio ✅
- Tool registry ✅
- Policy engine for tool authorization ✅
- Approval gates ✅
- Role-based access control ✅
- Budget tracking ✅

### ✅ Phase 3 — Org + roles + GitOps (Complete)
- Organization management ✅
- Team and person hierarchy ✅
- Process templates ✅
- `.claude/` compiler (generate settings from org config) ✅
- Template instantiation ✅
- GitOps-friendly config ✅

### ✅ Phase 4 — Capacity broker (Complete)
- Multi-source capacity pooling ✅
- Rate limit handling with backoff ✅
- Priority queues ✅
- Cost tracking ✅

### Phase 5 — Routines + boards
- Cron scheduler
- Approval boards with quorum
- Config change management workflow

## Design Highlights

### Event Sourcing

All state changes are captured as events and written to immutable logs. Benefits:
- Complete audit trail
- Time-travel debugging
- Reproducible runs
- Rebuildable derived state

### Content-Addressed Storage

Blobs (prompts, responses, patches, artifacts) are stored by SHA-256 hash:
- Automatic deduplication
- Tamper detection
- Efficient caching
- S3-compatible for scale

### Separation of Concerns

- **Core crate**: Pure business logic, no I/O or framework dependencies
- **Server crate**: HTTP API, configuration, wiring
- **MCP crate**: Tool protocol, isolated from workflow logic

## License

Business Source License 1.1

Shiioo is licensed under the [Business Source License 1.1](LICENSE). This allows you to:
- Use Shiioo for internal production workflows
- Deploy Shiioo to execute your own organization's workflows and jobs
- Build workflow platforms with abstraction layers on top of Shiioo

The license **does not** permit running a public managed service where third parties can register and execute their own workflows through Shiioo's APIs.

**Change Date:** 4 years after each release
**Change License:** Apache License, Version 2.0

After the Change Date, Shiioo automatically becomes Apache 2.0 licensed.

---

**Status**: Phase 4 complete. Production-ready enterprise orchestration with:
- DAG workflow execution with dependencies, retry, event sourcing
- Role-based access control and policy enforcement
- Organization management with teams, people, reporting structure
- Process templates for reusable workflows
- Claude config compiler for GitOps-friendly agent deployment
- Multi-source capacity pooling with rate limit handling
- Priority queues and automatic failover
- Real-time cost tracking and usage monitoring

See documentation:
- [POLICY_EXAMPLES.md](POLICY_EXAMPLES.md) - Policy engine and governance
- [PHASE3_EXAMPLES.md](PHASE3_EXAMPLES.md) - Org management, templates, Claude compiler
- [PHASE4_EXAMPLES.md](PHASE4_EXAMPLES.md) - Capacity broker, rate limits, cost tracking
