# Phase 10: UI & Developer Experience

Phase 10 adds a comprehensive GraphQL API, real-time web dashboard, and developer-friendly tools for interacting with the Shiioo Virtual Company OS.

## Features

### 1. GraphQL API

A complete GraphQL API with Query, Mutation, and Subscription support for programmatic access to all Shiioo features.

#### Query Examples

```graphql
# Get workflow by ID
query GetWorkflow {
  workflow(id: "workflow-123") {
    id
    name
    description
    createdAt
    steps {
      id
      name
      role
      actionType
    }
  }
}

# List recent workflow runs
query ListRuns {
  runs(limit: 10) {
    id
    workflowId
    status
    startedAt
    completedAt
    error
  }
}

# Get system metrics
query GetMetrics {
  metricsSummary {
    totalRuns
    successfulRuns
    failedRuns
    avgDurationMs
  }
}

# Get system health
query GetHealth {
  systemHealth {
    overallStatus
    totalRuns
    successfulRuns
    failedRuns
    successRate
  }
}

# Get audit log with filtering
query GetAuditLog {
  auditEntries(limit: 20, category: "WorkflowExecution") {
    id
    timestamp
    category
    severity
    userId
    tenantId
  }
}

# Get tenants
query GetTenants {
  tenants {
    id
    name
    status
    createdAt
  }
}

# Get cluster nodes
query GetClusterNodes {
  clusterNodes {
    id
    address
    status
    lastHeartbeat
  }
}
```

#### Mutation Examples

```graphql
# Create a new workflow run
mutation CreateRun {
  createRun(input: {
    workflowId: "workflow-123"
    inputs: { key: "value" }
  }) {
    id
    workflowId
    status
    startedAt
  }
}

# Register a new tenant
mutation RegisterTenant {
  registerTenant(input: {
    name: "Acme Corp"
    maxWorkflows: 100
    maxRoutines: 50
    maxStorageMb: 1024
    maxApiCallsPerHour: 10000
  }) {
    id
    name
    status
    createdAt
  }
}

# Suspend a tenant
mutation SuspendTenant {
  suspendTenant(id: "tenant-123") {
    id
    name
    status
  }
}
```

#### Subscription Examples

```graphql
# Subscribe to real-time run updates
subscription RunUpdates {
  runUpdates {
    id
    workflowId
    status
    startedAt
    completedAt
  }
}

# Subscribe to audit events
subscription AuditEvents {
  auditEvents {
    id
    timestamp
    category
    severity
    userId
  }
}

# Subscribe to metrics updates
subscription MetricsUpdates {
  metricsUpdates {
    totalRuns
    successfulRuns
    failedRuns
    avgDurationMs
  }
}
```

### 2. GraphQL Playground

Interactive GraphQL playground available at `/api/graphql` for exploring the API, testing queries, and viewing the schema documentation.

**Access the playground:**

```bash
# Start the server
cargo run --bin shiioo

# Open in browser
open http://localhost:8080/api/graphql
```

### 3. Real-Time Web Dashboard

A modern, real-time web dashboard for monitoring and managing Shiioo workflows.

**Features:**
- Real-time metrics via GraphQL subscriptions
- System health monitoring
- Recent workflow runs view
- Audit log with filtering
- Auto-reconnecting WebSocket
- Dark theme UI

**Access the dashboard:**

```bash
# Start the server
cargo run --bin shiioo

# Open in browser (root path or /dashboard)
open http://localhost:8080/
# or
open http://localhost:8080/dashboard
```

**Dashboard Sections:**

1. **Metrics Overview**
   - Total workflow runs
   - Success rate percentage
   - Average execution duration
   - Active tenant count

2. **System Health**
   - Overall status (Healthy/Degraded/Unhealthy)
   - Success rate calculation
   - Run statistics

3. **Recent Workflow Runs**
   - Live table of recent executions
   - Status indicators
   - Duration tracking

4. **Audit Log**
   - Filterable by category
   - Severity indicators
   - User tracking
   - Timestamp display

### 4. API Endpoints

#### GraphQL Endpoints

```bash
# Query and Mutation endpoint
POST /api/graphql
Content-Type: application/json

{
  "query": "query { systemHealth { overallStatus } }",
  "variables": {}
}

# GraphQL Playground (browser)
GET /api/graphql

# WebSocket Subscriptions
WS /api/graphql/ws
Protocol: graphql-transport-ws
```

#### REST Endpoints

All existing REST endpoints remain available for backward compatibility:

```bash
# Health check
GET /api/health

# Runs
GET /api/runs
GET /api/runs/{run_id}

# Analytics
GET /api/metrics
GET /api/analytics/workflows

# Audit
GET /api/audit/entries

# RBAC
GET /api/rbac/roles
POST /api/rbac/roles

# And many more...
```

## Usage Examples

### Using the GraphQL API with cURL

```bash
# Query system health
curl -X POST http://localhost:8080/api/graphql \
  -H "Content-Type: application/json" \
  -d '{
    "query": "query { systemHealth { overallStatus successRate } }"
  }'

# Create a workflow run
curl -X POST http://localhost:8080/api/graphql \
  -H "Content-Type: application/json" \
  -d '{
    "query": "mutation CreateRun($input: CreateRunInput!) { createRun(input: $input) { id status } }",
    "variables": {
      "input": {
        "workflowId": "workflow-123"
      }
    }
  }'

# Get recent runs
curl -X POST http://localhost:8080/api/graphql \
  -H "Content-Type: application/json" \
  -d '{
    "query": "query { runs(limit: 5) { id status startedAt } }"
  }'
```

### Using WebSocket Subscriptions (JavaScript)

```javascript
// Connect to GraphQL WebSocket
const ws = new WebSocket('ws://localhost:8080/api/graphql/ws', 'graphql-transport-ws');

ws.onopen = () => {
  // Initialize connection
  ws.send(JSON.stringify({ type: 'connection_init' }));

  // Subscribe to metrics updates
  ws.send(JSON.stringify({
    id: 'metrics-sub',
    type: 'subscribe',
    payload: {
      query: `
        subscription {
          metricsUpdates {
            totalRuns
            successfulRuns
            avgDurationMs
          }
        }
      `
    }
  }));
};

ws.onmessage = (event) => {
  const message = JSON.parse(event.data);

  if (message.type === 'connection_ack') {
    console.log('Subscription ready');
  }

  if (message.type === 'next') {
    const metrics = message.payload.data.metricsUpdates;
    console.log('Metrics update:', metrics);
  }
};
```

### Using the Dashboard

1. **Start the server:**
   ```bash
   cargo run --bin shiioo
   ```

2. **Open the dashboard:**
   ```bash
   open http://localhost:8080/
   ```

3. **View real-time metrics:**
   - Metrics update automatically via WebSocket subscriptions
   - No manual refresh needed
   - Auto-reconnects if connection drops

4. **Filter audit logs:**
   - Click category tabs to filter
   - View by Authentication, Authorization, WorkflowExecution, etc.

5. **Monitor workflow runs:**
   - See recent executions in real-time
   - Status updates appear automatically

## Architecture

### GraphQL Schema Structure

```
Query {
  workflow(id)
  workflows(limit)
  run(id)
  runs(limit)
  auditEntries(limit, category)
  tenants()
  clusterNodes()
  metricsSummary()
  systemHealth()
}

Mutation {
  createRun(input)
  registerTenant(input)
  suspendTenant(id)
}

Subscription {
  runUpdates
  auditEvents
  metricsUpdates
}
```

### Dashboard Architecture

```
┌─────────────────────────────────────────┐
│          Web Dashboard (HTML/JS)        │
│  ┌─────────────────────────────────┐   │
│  │  GraphQL Query (initial load)   │   │
│  └─────────────────────────────────┘   │
│  ┌─────────────────────────────────┐   │
│  │  WebSocket Subscriptions        │   │
│  │  (real-time updates)            │   │
│  └─────────────────────────────────┘   │
└─────────────────────────────────────────┘
                   ↓
┌─────────────────────────────────────────┐
│         GraphQL API (Axum)              │
│  ┌─────────────────────────────────┐   │
│  │  HTTP POST /api/graphql         │   │
│  │  (queries & mutations)          │   │
│  └─────────────────────────────────┘   │
│  ┌─────────────────────────────────┐   │
│  │  WS /api/graphql/ws            │   │
│  │  (subscriptions)                │   │
│  └─────────────────────────────────┘   │
└─────────────────────────────────────────┘
                   ↓
┌─────────────────────────────────────────┐
│            AppState                      │
│  - index_store (runs)                   │
│  - audit_log                            │
│  - tenant_manager                       │
│  - cluster_manager                      │
│  - analytics                            │
└─────────────────────────────────────────┘
```

## Implementation Details

### GraphQL Schema (`crates/server/src/graphql/schema.rs`)

- Uses `async-graphql` v7.0
- Query resolvers fetch from AppState stores
- Mutation resolvers modify state and return results
- Subscriptions use `async_stream` for real-time updates
- All subscriptions emit updates every 5-10 seconds

### Static Assets (`crates/server/src/ui.rs`)

- Uses `rust_embed` to embed dashboard at compile time
- Dashboard served from `crates/server/static/dashboard.html`
- Root path (`/`) serves the dashboard
- Fallback for SPA routing support

### WebSocket Protocol

- Uses `graphql-transport-ws` protocol
- Messages: `connection_init`, `subscribe`, `next`, `complete`
- Auto-reconnect on connection loss
- Multiple concurrent subscriptions supported

## Benefits

1. **Developer Experience**
   - Interactive GraphQL playground for API exploration
   - Real-time updates via subscriptions
   - Self-documenting schema
   - Type-safe queries and mutations

2. **Monitoring & Observability**
   - Real-time dashboard for system status
   - Audit log filtering and search
   - Workflow execution tracking
   - System health metrics

3. **Integration Friendly**
   - GraphQL API for flexible data fetching
   - WebSocket subscriptions for push updates
   - REST endpoints still available
   - Easy to integrate with external tools

4. **Production Ready**
   - Embedded dashboard (no build step)
   - Auto-reconnecting WebSocket
   - Error handling and fallbacks
   - Dark theme for reduced eye strain

## Next Steps

With Phase 10 complete, consider:

1. **Phase 11: Rust SDK** - Client library for Rust applications
2. **Phase 12: CLI Tools** - Command-line interface for workflow management
3. **Phase 13: Plugins** - Plugin system for extending functionality
4. **Phase 14: Advanced Workflows** - Visual workflow builder, conditions, loops

## Testing

```bash
# Run all tests
cargo test --workspace

# Build and start server
cargo run --bin shiioo

# Access GraphQL playground
open http://localhost:8080/api/graphql

# Access dashboard
open http://localhost:8080/

# Test GraphQL query
curl -X POST http://localhost:8080/api/graphql \
  -H "Content-Type: application/json" \
  -d '{"query":"{ systemHealth { overallStatus } }"}'
```

## Dependencies Added

```toml
[dependencies]
# GraphQL support (Phase 10)
async-graphql = { version = "7.0", features = ["chrono", "uuid"] }
async-graphql-axum = "7.0"
async-stream = "0.3"
```

## Files Modified/Created

**Created:**
- `crates/server/src/graphql/mod.rs` - GraphQL handlers and playground
- `crates/server/src/graphql/schema.rs` - GraphQL schema definition
- `crates/server/static/dashboard.html` - Real-time web dashboard

**Modified:**
- `crates/server/Cargo.toml` - Added GraphQL dependencies
- `crates/server/src/api/mod.rs` - Added GraphQL routes
- `crates/server/src/ui.rs` - Added dashboard serving
- `crates/core/src/audit.rs` - Added Clone trait
- `crates/core/src/rbac.rs` - Added Clone trait
- `crates/server/src/api/handlers.rs` - Fixed moved value issues

---

**Phase 10 Status:** ✅ Complete

All features implemented, tested, and documented.
