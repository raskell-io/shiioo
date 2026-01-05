# Phase 6: Real-time Monitoring & Observability Guide

This guide demonstrates how to use Shiioo's real-time monitoring and observability features including metrics collection, performance analytics, WebSocket updates, and bottleneck detection introduced in Phase 6.

## Overview

Phase 6 adds:
- **Metrics Collection** - Prometheus-style counters, gauges, and histograms
- **Performance Analytics** - Workflow and step execution tracking with statistics
- **WebSocket Support** - Real-time updates for workflow status and system health
- **Bottleneck Detection** - Automatic identification of slow steps
- **Health Monitoring** - System-wide health status and success rates

## Metrics API

### Get All Metrics

Retrieve all system metrics including counters, gauges, and histograms:

```bash
curl http://localhost:8080/api/metrics
```

Response:
```json
{
  "counters": [
    {
      "name": "workflow_executions_total",
      "value": 1523,
      "labels": {
        "status": "success"
      },
      "last_updated": "2026-01-05T15:30:00Z"
    },
    {
      "name": "http_requests_total",
      "value": 45289,
      "labels": {
        "method": "GET",
        "path": "/api/runs"
      },
      "last_updated": "2026-01-05T15:30:15Z"
    }
  ],
  "gauges": [
    {
      "name": "active_workflows",
      "value": 12.0,
      "labels": {},
      "last_updated": "2026-01-05T15:30:10Z"
    },
    {
      "name": "memory_usage_mb",
      "value": 1024.5,
      "labels": {},
      "last_updated": "2026-01-05T15:30:12Z"
    }
  ],
  "histograms": [
    {
      "name": "workflow_duration_seconds",
      "buckets": [0.01, 0.1, 0.5, 1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0],
      "counts": [15, 120, 450, 800, 1200, 1450, 1500, 1520, 1523, 1523],
      "sum": 45678.5,
      "count": 1523,
      "labels": {},
      "last_updated": "2026-01-05T15:30:00Z"
    }
  ]
}
```

### Metric Types

**Counters** - Monotonically increasing values:
- `workflow_executions_total` - Total workflow executions
- `http_requests_total` - Total HTTP requests
- `errors_total` - Total errors

**Gauges** - Values that can go up or down:
- `active_workflows` - Currently running workflows
- `active_routines` - Enabled routines
- `pending_approvals` - Approvals awaiting votes
- `memory_usage_mb` - Memory consumption

**Histograms** - Distribution of values:
- `workflow_duration_seconds` - Workflow execution time
- `step_duration_seconds` - Individual step execution time
- `request_duration_seconds` - HTTP request latency

## Performance Analytics

### Get Workflow Analytics

Retrieve performance statistics for all workflows:

```bash
curl http://localhost:8080/api/analytics/workflows
```

Response:
```json
{
  "workflows": [
    {
      "workflow_id": "daily_report_generation",
      "execution_count": 156,
      "success_count": 153,
      "failure_count": 3,
      "total_duration_secs": 4680.5,
      "min_duration_secs": 25.2,
      "max_duration_secs": 35.8,
      "avg_duration_secs": 30.0,
      "last_execution": "2026-01-05T15:00:00Z"
    },
    {
      "workflow_id": "user_onboarding",
      "execution_count": 42,
      "success_count": 42,
      "failure_count": 0,
      "total_duration_secs": 210.0,
      "min_duration_secs": 4.5,
      "max_duration_secs": 6.2,
      "avg_duration_secs": 5.0,
      "last_execution": "2026-01-05T14:45:00Z"
    }
  ]
}
```

### Get Specific Workflow Analytics

```bash
curl http://localhost:8080/api/analytics/workflows/daily_report_generation
```

### Get Step Analytics

Retrieve performance statistics for all workflow steps:

```bash
curl http://localhost:8080/api/analytics/steps
```

Response:
```json
{
  "steps": [
    {
      "step_id": "fetch_data",
      "execution_count": 156,
      "success_count": 156,
      "failure_count": 0,
      "retry_count": 0,
      "total_duration_secs": 1560.0,
      "min_duration_secs": 8.5,
      "max_duration_secs": 12.3,
      "avg_duration_secs": 10.0,
      "p50_duration_secs": 9.8,
      "p95_duration_secs": 11.5,
      "p99_duration_secs": 12.1,
      "durations": [8.5, 9.2, ...]
    },
    {
      "step_id": "generate_charts",
      "execution_count": 156,
      "success_count": 153,
      "failure_count": 3,
      "retry_count": 8,
      "total_duration_secs": 3120.5,
      "min_duration_secs": 15.0,
      "max_duration_secs": 25.0,
      "avg_duration_secs": 20.0,
      "p50_duration_secs": 19.5,
      "p95_duration_secs": 23.0,
      "p99_duration_secs": 24.5,
      "durations": [15.0, 16.2, ...]
    }
  ]
}
```

**Key Metrics:**
- `execution_count` - Total times this step executed
- `success_count` - Successful executions
- `failure_count` - Failed executions
- `retry_count` - Number of retries performed
- `avg_duration_secs` - Average execution time
- `p50_duration_secs` - Median (50th percentile)
- `p95_duration_secs` - 95th percentile
- `p99_duration_secs` - 99th percentile

## Execution Tracing

### Get Recent Execution Traces

Retrieve the last 50 workflow execution traces:

```bash
curl http://localhost:8080/api/analytics/traces
```

Response:
```json
{
  "traces": [
    {
      "run_id": "01JH8XYZABC123",
      "workflow_id": "daily_report_generation",
      "started_at": "2026-01-05T15:00:00Z",
      "completed_at": "2026-01-05T15:00:30Z",
      "duration_secs": 30.0,
      "status": "Completed",
      "steps": [
        {
          "step_id": "fetch_data",
          "started_at": "2026-01-05T15:00:00Z",
          "completed_at": "2026-01-05T15:00:10Z",
          "duration_secs": 10.0,
          "status": "Completed",
          "attempt": 0,
          "error": null
        },
        {
          "step_id": "generate_charts",
          "started_at": "2026-01-05T15:00:10Z",
          "completed_at": "2026-01-05T15:00:30Z",
          "duration_secs": 20.0,
          "status": "Completed",
          "attempt": 0,
          "error": null
        }
      ],
      "bottleneck": {
        "step_id": "generate_charts",
        "duration_secs": 20.0,
        "percentage_of_total": 66.67
      }
    }
  ]
}
```

### Get Specific Execution Trace

```bash
curl http://localhost:8080/api/analytics/traces/01JH8XYZABC123
```

## Bottleneck Detection

### Analyze Workflow Bottlenecks

Identify the slowest steps in a workflow:

```bash
curl http://localhost:8080/api/analytics/bottlenecks/daily_report_generation
```

Response:
```json
{
  "workflow_id": "daily_report_generation",
  "total_executions": 156,
  "avg_duration_secs": 30.0,
  "bottlenecks": [
    {
      "step_id": "generate_charts",
      "avg_duration_secs": 20.0,
      "percentage_of_workflow": 66.67,
      "execution_count": 156
    },
    {
      "step_id": "fetch_data",
      "avg_duration_secs": 10.0,
      "percentage_of_workflow": 33.33,
      "execution_count": 156
    }
  ]
}
```

**Interpretation:**
- `generate_charts` takes 66.67% of total workflow time
- Optimizing this step would have the biggest impact on overall performance
- Bottlenecks are sorted by percentage_of_workflow (highest first)

## System Health

### Get Health Status

Comprehensive system health overview:

```bash
curl http://localhost:8080/api/health/status
```

Response:
```json
{
  "status": "healthy",
  "uptime_secs": 0,
  "active_routines": 8,
  "total_routines": 12,
  "pending_approvals": 3,
  "total_workflow_executions": 1523,
  "successful_executions": 1498,
  "failed_executions": 25,
  "success_rate": 98.36
}
```

**Health Indicators:**
- `status` - Overall health status (`healthy`, `degraded`, `critical`)
- `success_rate` - Percentage of successful workflow executions
- `active_routines` - Number of enabled scheduled routines
- `pending_approvals` - Approvals awaiting votes

## WebSocket Real-Time Updates

### Connect to WebSocket

Establish a WebSocket connection for real-time updates:

```javascript
const ws = new WebSocket('ws://localhost:8080/api/ws');

ws.onopen = () => {
  console.log('Connected to Shiioo WebSocket');

  // Subscribe to health updates
  ws.send(JSON.stringify({
    action: 'SubscribeHealth'
  }));
};

ws.onmessage = (event) => {
  const message = JSON.parse(event.data);
  console.log('Received:', message);

  switch (message.type) {
    case 'Subscribed':
      console.log(`Subscribed: ${message.subscription_id}`);
      break;

    case 'WorkflowUpdate':
      console.log(`Workflow ${message.run_id}: ${message.status} (${message.progress}%)`);
      break;

    case 'StepUpdate':
      console.log(`Step ${message.step_id} in workflow ${message.run_id}: ${message.status}`);
      break;

    case 'HealthUpdate':
      console.log(`System health: ${message.status}`);
      console.log(`Active workflows: ${message.active_workflows}`);
      console.log(`Pending approvals: ${message.pending_approvals}`);
      break;

    case 'MetricsUpdate':
      console.log(`Metric ${message.name}: ${message.value}`);
      break;
  }
};

ws.onerror = (error) => {
  console.error('WebSocket error:', error);
};

ws.onclose = () => {
  console.log('WebSocket connection closed');
};
```

### WebSocket Subscription Types

**Subscribe to All Workflows:**
```json
{"action": "SubscribeAll"}
```

**Subscribe to Specific Workflow:**
```json
{"action": "SubscribeWorkflow", "run_id": "01JH8XYZABC123"}
```

**Subscribe to Metrics Updates:**
```json
{"action": "SubscribeMetrics"}
```

**Subscribe to Health Updates:**
```json
{"action": "SubscribeHealth"}
```

**Unsubscribe:**
```json
{"action": "Unsubscribe"}
```

### WebSocket Message Types

**WorkflowUpdate:**
```json
{
  "type": "WorkflowUpdate",
  "run_id": "01JH8XYZABC123",
  "status": "Running",
  "progress": 45.5,
  "message": "Processing step 2 of 4"
}
```

**StepUpdate:**
```json
{
  "type": "StepUpdate",
  "run_id": "01JH8XYZABC123",
  "step_id": "generate_charts",
  "status": "Completed",
  "message": "Charts generated successfully"
}
```

**HealthUpdate:**
```json
{
  "type": "HealthUpdate",
  "status": "healthy",
  "active_workflows": 12,
  "active_routines": 8,
  "pending_approvals": 3
}
```

**MetricsUpdate:**
```json
{
  "type": "MetricsUpdate",
  "metric_type": "counter",
  "name": "workflow_executions_total",
  "value": 1524.0,
  "labels": {"status": "success"}
}
```

## Complete Observability Example

### Scenario: Monitor a Long-Running Workflow

1. **Start workflow execution:**
```bash
curl -X POST http://localhost:8080/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Monthly Report Generation",
    "workflow": {
      "steps": [
        {"id": "fetch_data", "name": "Fetch Data", "role": "data_analyst", "action": {"agent_task": {"prompt": "Fetch monthly data"}}},
        {"id": "analyze_data", "name": "Analyze Data", "role": "data_analyst", "action": {"agent_task": {"prompt": "Analyze the data"}}},
        {"id": "generate_charts", "name": "Generate Charts", "role": "visualization_agent", "action": {"agent_task": {"prompt": "Create visualizations"}}},
        {"id": "create_report", "name": "Create Report", "role": "report_writer", "action": {"agent_task": {"prompt": "Write final report"}}}
      ],
      "dependencies": {
        "analyze_data": ["fetch_data"],
        "generate_charts": ["analyze_data"],
        "create_report": ["generate_charts"]
      }
    },
    "execute": true
  }'
```

2. **Connect to WebSocket for real-time updates:**
```javascript
const ws = new WebSocket('ws://localhost:8080/api/ws');
ws.onopen = () => {
  ws.send(JSON.stringify({action: 'SubscribeAll'}));
};
```

3. **Monitor execution trace:**
```bash
# Get the run_id from step 1 response
RUN_ID="01JH8XYZABC123"

# Check real-time trace
curl http://localhost:8080/api/analytics/traces/$RUN_ID
```

4. **After completion, analyze bottlenecks:**
```bash
curl http://localhost:8080/api/analytics/bottlenecks/monthly_report_generation
```

5. **Review workflow statistics:**
```bash
curl http://localhost:8080/api/analytics/workflows/monthly_report_generation
```

6. **Check system health:**
```bash
curl http://localhost:8080/api/health/status
```

## Best Practices

### Performance Monitoring

- **Track percentiles, not just averages**: p95 and p99 reveal outliers that averages miss
- **Monitor success rates**: High execution counts with low success rates indicate systemic issues
- **Use execution traces for debugging**: Detailed step-by-step execution logs help identify failures
- **Set up bottleneck analysis dashboards**: Regular review of bottlenecks guides optimization efforts

### WebSocket Connections

- **Implement reconnection logic**: WebSocket connections can drop; auto-reconnect on failure
- **Use subscriptions wisely**: Subscribe only to what you need to minimize bandwidth
- **Handle message bursts**: Buffer and batch process high-frequency updates
- **Implement ping/pong**: Keep connections alive with regular heartbeats

### Metrics Collection

- **Use labels for dimensionality**: Add labels like `status`, `workflow_id`, `step_id` for filtering
- **Don't over-collect**: Too many metrics increase storage and query costs
- **Use appropriate metric types**:
  - Counters for cumulative counts (requests, executions)
  - Gauges for current state (active workflows, memory)
  - Histograms for distributions (duration, latency)

### Health Monitoring

- **Set up alerts**: Notify when success rate drops below threshold
- **Monitor trends**: Gradual degradation is easier to catch with trending
- **Check pending approvals**: High counts might indicate bottlenecks in governance
- **Review active routines**: Disabled routines might indicate issues

## Troubleshooting

### High Latency Detected

1. Check bottleneck analysis for the workflow
2. Review p95 and p99 percentiles for affected steps
3. Check execution traces for patterns (retries, errors)
4. Review system resource usage (memory, CPU)

### Low Success Rate

1. Filter analytics by failure status
2. Review execution traces for failed runs
3. Check error messages in step traces
4. Review retry counts - high retries indicate flaky steps

### WebSocket Connection Issues

1. Verify WebSocket endpoint is accessible: `ws://localhost:8080/api/ws`
2. Check browser console for connection errors
3. Ensure firewall allows WebSocket traffic
4. Verify server logs for WebSocket errors

### Missing Metrics

1. Metrics are in-memory - restarting the server clears them
2. Check that workflow execution is actually occurring
3. Verify analytics tracking is enabled
4. Review server logs for tracking errors

## API Reference Summary

### Metrics
- `GET /api/metrics` - Get all metrics

### Analytics
- `GET /api/analytics/workflows` - List workflow statistics
- `GET /api/analytics/workflows/{workflow_id}` - Get specific workflow stats
- `GET /api/analytics/steps` - List step statistics
- `GET /api/analytics/traces` - Get recent execution traces (last 50)
- `GET /api/analytics/traces/{run_id}` - Get specific execution trace
- `GET /api/analytics/bottlenecks/{workflow_id}` - Analyze workflow bottlenecks

### Health
- `GET /api/health/status` - Get system health status

### WebSocket
- `GET /api/ws` - WebSocket endpoint for real-time updates

## Integration Examples

### Prometheus Integration

Export metrics in Prometheus format (future enhancement):
```bash
curl http://localhost:8080/api/metrics/prometheus
```

### Grafana Dashboard

Create visualizations for:
- Workflow execution rates
- Success/failure rates over time
- Step duration percentiles (p50, p95, p99)
- Active workflows gauge
- Bottleneck heatmap

### Alert Rules

Example alerting thresholds:
- Success rate < 95% for 5 minutes
- p99 latency > 2x baseline
- Pending approvals > 20
- Active workflows > capacity limit
