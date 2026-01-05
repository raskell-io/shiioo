# Phase 7: Multi-tenancy & High Availability Examples

This document provides comprehensive examples of the multi-tenancy and high availability features implemented in Phase 7.

## Table of Contents

- [Overview](#overview)
- [Multi-Tenancy](#multi-tenancy)
  - [Tenant Management](#tenant-management)
  - [Resource Quotas](#resource-quotas)
  - [Tenant-Scoped Storage](#tenant-scoped-storage)
- [High Availability](#high-availability)
  - [Cluster Management](#cluster-management)
  - [Distributed Locking](#distributed-locking)
  - [Leader Election](#leader-election)
- [API Reference](#api-reference)

## Overview

Phase 7 introduces **multi-tenancy** and **high availability (HA)** capabilities to Shiioo:

- **Multi-tenancy**: Isolate data and resources per tenant with configurable quotas
- **Distributed locking**: Coordinate exclusive access across cluster nodes
- **Leader election**: Automatic leader selection for cluster coordination
- **Node discovery & health**: Track cluster node status with heartbeat monitoring

## Multi-Tenancy

### Tenant Management

#### Register a new tenant

```bash
curl -X POST http://localhost:3000/api/tenants \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Acme Corporation",
    "description": "Enterprise customer with custom workflows",
    "quota": {
      "max_concurrent_workflows": 50,
      "max_workflows_per_day": 5000,
      "max_routines": 100,
      "max_storage_bytes": 53687091200,
      "max_api_requests_per_minute": 5000
    },
    "settings": {
      "data_retention_days": 365,
      "enable_audit_logging": true,
      "metadata": {
        "tier": "enterprise",
        "region": "us-east"
      }
    }
  }'
```

Response:
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Acme Corporation",
  "description": "Enterprise customer with custom workflows",
  "status": "Active",
  "quota": {
    "max_concurrent_workflows": 50,
    "max_workflows_per_day": 5000,
    "max_routines": 100,
    "max_storage_bytes": 53687091200,
    "max_api_requests_per_minute": 5000
  },
  "settings": {
    "data_retention_days": 365,
    "enable_audit_logging": true,
    "metadata": {
      "tier": "enterprise",
      "region": "us-east"
    }
  },
  "created_at": "2024-01-05T12:00:00Z",
  "updated_at": "2024-01-05T12:00:00Z"
}
```

#### List all tenants

```bash
curl http://localhost:3000/api/tenants
```

Response:
```json
{
  "tenants": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "Acme Corporation",
      "status": "Active",
      "created_at": "2024-01-05T12:00:00Z"
    },
    {
      "id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
      "name": "Beta Industries",
      "status": "Suspended",
      "created_at": "2024-01-03T10:30:00Z"
    }
  ]
}
```

#### Get tenant details

```bash
curl http://localhost:3000/api/tenants/550e8400-e29b-41d4-a716-446655440000
```

#### Update tenant

```bash
curl -X PUT http://localhost:3000/api/tenants/550e8400-e29b-41d4-a716-446655440000 \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Acme Corp (Updated)",
    "quota": {
      "max_concurrent_workflows": 100,
      "max_workflows_per_day": 10000,
      "max_routines": 200,
      "max_storage_bytes": 107374182400,
      "max_api_requests_per_minute": 10000
    }
  }'
```

#### Suspend tenant

```bash
curl -X POST http://localhost:3000/api/tenants/550e8400-e29b-41d4-a716-446655440000/suspend
```

Response:
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Acme Corporation",
  "status": "Suspended",
  "updated_at": "2024-01-05T14:30:00Z"
}
```

#### Activate tenant

```bash
curl -X POST http://localhost:3000/api/tenants/550e8400-e29b-41d4-a716-446655440000/activate
```

#### Delete tenant

```bash
curl -X DELETE http://localhost:3000/api/tenants/550e8400-e29b-41d4-a716-446655440000
```

Response:
```json
{
  "message": "Tenant 550e8400-e29b-41d4-a716-446655440000 deleted successfully"
}
```

### Resource Quotas

Each tenant has configurable resource limits:

| Quota | Description | Default |
|-------|-------------|---------|
| `max_concurrent_workflows` | Maximum workflows running simultaneously | 10 |
| `max_workflows_per_day` | Daily workflow execution limit | 1000 |
| `max_routines` | Maximum number of scheduled routines | 50 |
| `max_storage_bytes` | Storage quota in bytes | 10 GB |
| `max_api_requests_per_minute` | API rate limit | 1000 |

#### Check quota compliance

Quotas are enforced automatically when tenants:
- Execute workflows
- Create routines
- Store data
- Make API requests

The system will return `429 Too Many Requests` or `507 Insufficient Storage` when quotas are exceeded.

### Tenant-Scoped Storage

All tenant data is isolated in separate storage partitions:

```
data/
├── tenants/
│   ├── 550e8400-e29b-41d4-a716-446655440000/
│   │   ├── blobs/
│   │   ├── events.jsonl
│   │   └── index.redb
│   └── 6ba7b810-9dad-11d1-80b4-00c04fd430c8/
│       ├── blobs/
│       ├── events.jsonl
│       └── index.redb
```

#### Get tenant storage statistics

```bash
curl http://localhost:3000/api/tenants/550e8400-e29b-41d4-a716-446655440000/storage-stats
```

Response:
```json
{
  "total_bytes": 1073741824,
  "file_count": 1523
}
```

## High Availability

### Cluster Management

Shiioo supports multi-node clusters for high availability and horizontal scaling.

#### Register a cluster node

```bash
curl -X POST http://localhost:3000/api/cluster/nodes \
  -H "Content-Type: application/json" \
  -d '{
    "address": "http://node-1.example.com:3000",
    "region": "us-east-1",
    "metadata": {
      "instance_type": "m5.xlarge",
      "availability_zone": "us-east-1a"
    }
  }'
```

Response:
```json
{
  "id": "node-550e8400-e29b-41d4-a716-446655440000",
  "address": "http://node-1.example.com:3000",
  "region": "us-east-1",
  "status": "Healthy",
  "role": "Follower",
  "last_heartbeat": "2024-01-05T12:00:00Z",
  "started_at": "2024-01-05T12:00:00Z",
  "metadata": {
    "instance_type": "m5.xlarge",
    "availability_zone": "us-east-1a"
  }
}
```

#### List cluster nodes

```bash
curl http://localhost:3000/api/cluster/nodes
```

Response:
```json
{
  "nodes": [
    {
      "id": "node-550e8400-e29b-41d4-a716-446655440000",
      "address": "http://node-1.example.com:3000",
      "region": "us-east-1",
      "status": "Healthy",
      "role": "Leader",
      "last_heartbeat": "2024-01-05T12:05:00Z"
    },
    {
      "id": "node-6ba7b810-9dad-11d1-80b4-00c04fd430c8",
      "address": "http://node-2.example.com:3000",
      "region": "us-east-1",
      "status": "Healthy",
      "role": "Follower",
      "last_heartbeat": "2024-01-05T12:05:02Z"
    },
    {
      "id": "node-7c9e6679-7425-40de-944b-e07fc1f90ae7",
      "address": "http://node-3.example.com:3000",
      "region": "us-west-2",
      "status": "Degraded",
      "role": "Follower",
      "last_heartbeat": "2024-01-05T11:59:30Z"
    }
  ]
}
```

#### Send heartbeat

Nodes should send periodic heartbeats to maintain healthy status:

```bash
curl -X POST http://localhost:3000/api/cluster/nodes/node-550e8400-e29b-41d4-a716-446655440000/heartbeat
```

Response:
```json
{
  "message": "Heartbeat acknowledged"
}
```

**Heartbeat interval**: 30 seconds (configurable)
**Unhealthy threshold**: No heartbeat for >30 seconds

#### Get cluster health

```bash
curl http://localhost:3000/api/cluster/health
```

Response:
```json
{
  "total_nodes": 3,
  "healthy_nodes": 2,
  "has_leader": true,
  "leader_id": "node-550e8400-e29b-41d4-a716-446655440000"
}
```

#### Remove cluster node

```bash
curl -X DELETE http://localhost:3000/api/cluster/nodes/node-7c9e6679-7425-40de-944b-e07fc1f90ae7
```

### Distributed Locking

The `DistributedLock` module provides cluster-wide coordination for exclusive resource access.

#### Code Example: Acquiring a lock

```rust
use shiioo_core::cluster::{DistributedLock, NodeId};

let lock = DistributedLock::new(60); // 60 second TTL
let node_id = NodeId::new("node-1");

// Try to acquire lock
if lock.acquire("workflow-scheduler", node_id.clone())? {
    println!("Lock acquired!");

    // Perform critical section work
    schedule_workflows()?;

    // Release lock when done
    lock.release("workflow-scheduler", &node_id)?;
} else {
    println!("Lock held by another node");
}
```

#### Features

- **TTL-based expiration**: Locks automatically expire after configured duration
- **Automatic cleanup**: Expired locks are periodically cleaned up
- **Holder tracking**: Know which node holds each lock
- **Custom TTL**: Per-lock TTL configuration

### Leader Election

The `LeaderElection` module coordinates automatic leader selection across cluster nodes.

#### Code Example: Leader election

```rust
use shiioo_core::cluster::{ClusterManager, DistributedLock, LeaderElection, NodeId};
use std::sync::Arc;

let cluster_manager = Arc::new(ClusterManager::new(NodeId::new("node-1"), 30));
let lock = Arc::new(DistributedLock::new(30));
let election = LeaderElection::new(cluster_manager.clone(), lock, 15); // 15 sec lease

// Try to become leader
if election.try_become_leader(&node_id)? {
    println!("This node is now the leader!");

    // Start leader-specific tasks
    start_routine_scheduler();
    start_approval_processor();
}

// Renew leadership periodically
loop {
    tokio::time::sleep(Duration::from_secs(10)).await;

    if !election.renew_leadership(&node_id)? {
        println!("Lost leadership!");
        stop_leader_tasks();
        break;
    }
}
```

#### Get current leader

```bash
curl http://localhost:3000/api/cluster/leader
```

Response:
```json
{
  "leader": {
    "id": "node-550e8400-e29b-41d4-a716-446655440000",
    "address": "http://node-1.example.com:3000",
    "region": "us-east-1",
    "status": "Healthy",
    "role": "Leader",
    "last_heartbeat": "2024-01-05T12:05:00Z"
  }
}
```

#### Features

- **Automatic failover**: When leader goes down, another node is elected
- **Lease-based**: Leaders must renew leases to maintain leadership
- **Graceful step-down**: Leaders can voluntarily step down
- **Split-brain prevention**: Uses distributed locks to prevent multiple leaders

## API Reference

### Multi-Tenancy Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/api/tenants` | Register a new tenant |
| `GET` | `/api/tenants` | List all tenants |
| `GET` | `/api/tenants/{tenant_id}` | Get tenant details |
| `PUT` | `/api/tenants/{tenant_id}` | Update tenant |
| `DELETE` | `/api/tenants/{tenant_id}` | Delete tenant |
| `POST` | `/api/tenants/{tenant_id}/suspend` | Suspend tenant |
| `POST` | `/api/tenants/{tenant_id}/activate` | Activate tenant |
| `GET` | `/api/tenants/{tenant_id}/storage-stats` | Get storage statistics |

### Cluster Management Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/api/cluster/nodes` | Register a cluster node |
| `GET` | `/api/cluster/nodes` | List all cluster nodes |
| `GET` | `/api/cluster/nodes/{node_id}` | Get node details |
| `DELETE` | `/api/cluster/nodes/{node_id}` | Remove node from cluster |
| `POST` | `/api/cluster/nodes/{node_id}/heartbeat` | Send node heartbeat |
| `GET` | `/api/cluster/leader` | Get current leader node |
| `GET` | `/api/cluster/health` | Get cluster health status |

## Architecture Patterns

### Multi-tenant Workflow Execution

```rust
use shiioo_core::tenant::TenantContext;
use shiioo_core::storage::TenantStorage;

async fn execute_workflow_for_tenant(
    tenant_id: TenantId,
    workflow: WorkflowSpec,
) -> Result<Run> {
    // Create tenant context
    let context = TenantContext::new(tenant_id.clone());

    // Get tenant-scoped storage
    let event_log = tenant_storage.event_log(&tenant_id)?;
    let blob_store = tenant_storage.blob_store(&tenant_id)?;
    let index_store = tenant_storage.index_store(&tenant_id)?;

    // Execute workflow with isolated storage
    let executor = WorkflowExecutor::new(event_log, blob_store, index_store);
    let run = executor.execute(work_item_id, workflow).await?;

    Ok(run)
}
```

### Distributed Routine Scheduling

```rust
// Only the leader node runs routine scheduler
async fn run_scheduler_if_leader(
    election: Arc<LeaderElection>,
    node_id: NodeId,
    scheduler: Arc<RoutineScheduler>,
) {
    loop {
        if election.renew_leadership(&node_id)? {
            // Run scheduled routines
            scheduler.execute_due_routines().await?;
        } else {
            // Wait until we become leader
            tokio::time::sleep(Duration::from_secs(5)).await;
        }

        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}
```

### Cross-region Tenant Placement

```rust
// Route tenants to specific regions
async fn create_tenant_in_region(
    name: String,
    region: String,
) -> Result<Tenant> {
    let tenant = Tenant {
        id: TenantId::generate(),
        name,
        description: format!("Tenant in {}", region),
        settings: TenantSettings {
            metadata: HashMap::from([
                ("region".to_string(), region.clone()),
            ]),
            ..Default::default()
        },
        ..Default::default()
    };

    // Initialize storage in region-specific path
    let storage_path = format!("data/{}/{}", region, tenant.id.0);
    tenant_storage.initialize_tenant_at(&tenant.id, storage_path)?;

    Ok(tenant)
}
```

## Next Steps

- See [README.md](README.md) for overall system architecture
- See [PHASE5_EXAMPLES.md](PHASE5_EXAMPLES.md) for automation and governance
- See [PHASE6_EXAMPLES.md](PHASE6_EXAMPLES.md) for observability features
- Review core types in `crates/core/src/types.rs`
