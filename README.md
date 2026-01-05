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

## Current Status (Phase 1 Complete ✅)

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

### API Endpoints
  - `GET /api/health` - Health check
  - `GET /api/runs` - List all runs
  - `GET /api/runs/{run_id}` - Get run details
  - `GET /api/runs/{run_id}/events` - Get run events
  - `POST /api/jobs` - Create and execute a job

### Coming Next
- 🚧 **Phase 2**: MCP tools + policy enforcement
- 🚧 **Phase 3**: Org chart + roles + `.claude/` compiler
- 🚧 **Phase 4**: Capacity broker for rate limit resilience
- 🚧 **Phase 5**: Recurring routines + approval boards

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

### Phase 2 — MCP tools + policy
- MCP server over stdio
- Tool registry
- Policy engine for tool authorization
- Approval gates

### Phase 3 — Org + roles + GitOps
- Role specifications
- Process templates
- `.claude/` compiler (generate settings from org config)
- Governance workflows

### Phase 4 — Capacity broker
- Multi-source capacity pooling
- Rate limit handling with backoff
- Priority queues
- Cost tracking

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

MIT OR Apache-2.0

---

**Status**: Phase 1 complete. Workflows execute end-to-end with DAG dependencies, retry logic, and full event sourcing. Ready for Phase 2 (MCP tools & policy engine).
