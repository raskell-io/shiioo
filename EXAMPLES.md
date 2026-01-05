# Shiioo Examples

## Example 1: Simple Sequential Workflow

This example demonstrates a basic 3-step workflow where each step depends on the previous one.

### Create the Job

```bash
curl -X POST http://localhost:8080/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Sequential Pipeline",
    "workflow": {
      "steps": [
        {
          "id": "analyze",
          "name": "Analyze Requirements",
          "role": "analyst",
          "action": {
            "type": "agent_task",
            "prompt": "Analyze project requirements"
          },
          "timeout_secs": 60
        },
        {
          "id": "design",
          "name": "Design Solution",
          "role": "architect",
          "action": {
            "type": "agent_task",
            "prompt": "Design solution based on requirements"
          },
          "timeout_secs": 90
        },
        {
          "id": "implement",
          "name": "Implement",
          "role": "developer",
          "action": {
            "type": "agent_task",
            "prompt": "Implement the design"
          },
          "timeout_secs": 120
        }
      ],
      "dependencies": {
        "design": ["analyze"],
        "implement": ["design"]
      }
    }
  }'
```

**Execution flow:**
```
analyze → design → implement
```

## Example 2: Parallel Workflow

This example shows parallel execution with a final aggregation step.

```bash
curl -X POST http://localhost:8080/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Parallel Analysis",
    "workflow": {
      "steps": [
        {
          "id": "analyze_performance",
          "name": "Performance Analysis",
          "role": "performance_engineer",
          "action": {
            "type": "agent_task",
            "prompt": "Analyze performance metrics"
          }
        },
        {
          "id": "analyze_security",
          "name": "Security Analysis",
          "role": "security_engineer",
          "action": {
            "type": "agent_task",
            "prompt": "Perform security audit"
          }
        },
        {
          "id": "analyze_ux",
          "name": "UX Analysis",
          "role": "ux_designer",
          "action": {
            "type": "agent_task",
            "prompt": "Evaluate user experience"
          }
        },
        {
          "id": "synthesize",
          "name": "Synthesize Findings",
          "role": "project_manager",
          "action": {
            "type": "agent_task",
            "prompt": "Combine all analyses into recommendations"
          }
        }
      ],
      "dependencies": {
        "synthesize": ["analyze_performance", "analyze_security", "analyze_ux"]
      }
    }
  }'
```

**Execution flow:**
```
analyze_performance ┐
analyze_security    ├─→ synthesize
analyze_ux          ┘
```

## Example 3: Workflow with Retry Policy

This example demonstrates automatic retry with exponential backoff.

```bash
curl -X POST http://localhost:8080/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Resilient Task",
    "workflow": {
      "steps": [
        {
          "id": "unstable_task",
          "name": "Task That Might Fail",
          "role": "worker",
          "action": {
            "type": "agent_task",
            "prompt": "Perform potentially unstable operation"
          },
          "timeout_secs": 30,
          "retry_policy": {
            "max_attempts": 5,
            "backoff_secs": 2
          }
        }
      ],
      "dependencies": {}
    }
  }'
```

**Retry behavior:**
- Attempt 1: Execute immediately
- Attempt 2: Wait 2s (backoff_secs × 2^0)
- Attempt 3: Wait 4s (backoff_secs × 2^1)
- Attempt 4: Wait 8s (backoff_secs × 2^2)
- Attempt 5: Wait 16s (backoff_secs × 2^3)

## Example 4: Workflow with Approval Gate

```bash
curl -X POST http://localhost:8080/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Deploy Pipeline",
    "workflow": {
      "steps": [
        {
          "id": "build",
          "name": "Build Application",
          "role": "ci_system",
          "action": {
            "type": "script",
            "command": "make",
            "args": ["build"]
          }
        },
        {
          "id": "test",
          "name": "Run Tests",
          "role": "ci_system",
          "action": {
            "type": "script",
            "command": "make",
            "args": ["test"]
          }
        },
        {
          "id": "approve",
          "name": "Manual Approval",
          "role": "manager",
          "action": {
            "type": "manual_approval",
            "approvers": ["ceo", "cto", "lead_engineer"]
          },
          "requires_approval": true
        },
        {
          "id": "deploy",
          "name": "Deploy to Production",
          "role": "ci_system",
          "action": {
            "type": "script",
            "command": "make",
            "args": ["deploy"]
          }
        }
      ],
      "dependencies": {
        "test": ["build"],
        "approve": ["test"],
        "deploy": ["approve"]
      }
    }
  }'
```

**Execution flow:**
```
build → test → [manual approval gate] → deploy
```

Note: In MVP mode, approvals are auto-granted. Phase 2 will add real approval workflows.

## Inspecting Results

### View Run Status

```bash
# Get run details
curl http://localhost:8080/api/runs/{run_id} | jq

# Sample output:
{
  "id": "d907163f-c436-4237-bb5e-4f6a53248845",
  "work_item_id": "ed6fd07a-61fe-4295-878f-f0ec6bf694a5",
  "status": "completed",
  "started_at": "2026-01-05T11:16:06.654382Z",
  "completed_at": "2026-01-05T11:16:06.678355Z",
  "steps": [
    {
      "id": "analyze",
      "status": "completed",
      "started_at": "2026-01-05T11:16:06.660382Z",
      "completed_at": "2026-01-05T11:16:06.669424Z",
      "attempt": 1,
      "error": null
    }
  ]
}
```

### View Event Log

```bash
# Get all events for a run
curl http://localhost:8080/api/runs/{run_id}/events | jq

# Event types you'll see:
# - run_started
# - step_scheduled
# - step_started
# - agent_message (to/from agent)
# - step_completed / step_failed
# - artifact_produced
# - run_completed / run_failed
```

### Inspect Storage

```bash
# View compressed event log
gunzip -c data/events/events/2026/01/05/{run_id}.jsonl.gz | jq

# Count events
gunzip -c data/events/events/2026/01/05/{run_id}.jsonl.gz | wc -l

# View a blob (prompt or response)
cat data/blobs/ab/abcd1234...

# List all blobs for a run
find data/blobs -type f -newer data/index.redb
```

## Advanced: Diamond Dependency

This demonstrates a complex DAG with a diamond-shaped dependency graph.

```bash
curl -X POST http://localhost:8080/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Diamond DAG",
    "workflow": {
      "steps": [
        {"id": "start", "name": "Initialize", "role": "init", "action": {"type": "agent_task", "prompt": "Initialize"}},
        {"id": "path_a", "name": "Path A", "role": "worker_a", "action": {"type": "agent_task", "prompt": "Process A"}},
        {"id": "path_b", "name": "Path B", "role": "worker_b", "action": {"type": "agent_task", "prompt": "Process B"}},
        {"id": "merge", "name": "Merge Results", "role": "merger", "action": {"type": "agent_task", "prompt": "Merge A and B"}}
      ],
      "dependencies": {
        "path_a": ["start"],
        "path_b": ["start"],
        "merge": ["path_a", "path_b"]
      }
    }
  }'
```

**Execution flow:**
```
        ┌─→ path_a ─┐
start ──┤           ├─→ merge
        └─→ path_b ─┘
```

The DAG executor will:
1. Execute `start`
2. Execute `path_a` and `path_b` in parallel (no dependencies between them)
3. Wait for both to complete
4. Execute `merge`

## Next Steps

- **Phase 2** will add real MCP tool integration (filesystem, git, web fetch, etc.)
- **Phase 3** will add role-based policies and approval workflows
- **Phase 4** will add capacity pooling across multiple LLM sources
- **Phase 5** will add cron-like recurring routines
