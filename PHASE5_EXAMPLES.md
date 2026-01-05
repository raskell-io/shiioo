# Phase 5: Automation & Governance Guide

This guide demonstrates how to use Shiioo's automation and governance features including cron-scheduled routines, approval boards with quorum voting, and config change management introduced in Phase 5.

## Overview

Phase 5 adds:
- **Cron Scheduler** - Automatically execute workflows on recurring schedules
- **Approval Boards** - Multi-person approval workflows with quorum rules
- **Config Change Management** - Propose, approve, and apply configuration changes
- **Governance Workflow** - Ensure critical changes require team consensus

## Routine Management

### Create a Recurring Routine

Create a routine that runs every 15 minutes:

```bash
curl -X POST http://localhost:8080/api/routines \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Hourly System Health Check",
    "description": "Check system health and notify on issues",
    "schedule": {
      "cron": "*/15 * * * *",
      "timezone": "UTC"
    },
    "workflow": {
      "steps": [
        {
          "id": "health_check",
          "name": "Run Health Check",
          "description": "Check all system components",
          "role": "devops_agent",
          "action": {
            "agent_task": {
              "prompt": "Check system health metrics and report any anomalies"
            }
          },
          "timeout_secs": 300,
          "requires_approval": false
        }
      ],
      "dependencies": {}
    },
    "enabled": true,
    "created_by": "admin@example.com"
  }'
```

Response:
```json
{
  "routine_id": "550e8400-e29b-41d4-a716-446655440000",
  "message": "Routine created successfully"
}
```

### Common Cron Schedules

- `*/15 * * * *` - Every 15 minutes
- `0 * * * *` - Every hour at minute 0
- `0 0 * * *` - Daily at midnight
- `0 9 * * 1` - Every Monday at 9:00 AM
- `0 0 1 * *` - First day of every month at midnight

### List All Routines

```bash
curl http://localhost:8080/api/routines
```

Response:
```json
{
  "routines": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "Hourly System Health Check",
      "description": "Check system health and notify on issues",
      "schedule": {
        "cron": "*/15 * * * *",
        "timezone": "UTC"
      },
      "enabled": true,
      "last_run": "2026-01-05T14:15:00Z",
      "next_run": "2026-01-05T14:30:00Z",
      "created_at": "2026-01-05T10:00:00Z",
      "created_by": "admin@example.com",
      "updated_at": "2026-01-05T10:00:00Z"
    }
  ]
}
```

### Enable/Disable a Routine

```bash
# Enable
curl -X POST http://localhost:8080/api/routines/550e8400-e29b-41d4-a716-446655440000/enable

# Disable
curl -X POST http://localhost:8080/api/routines/550e8400-e29b-41d4-a716-446655440000/disable
```

### Get Execution History

```bash
curl http://localhost:8080/api/routines/550e8400-e29b-41d4-a716-446655440000/executions
```

Response:
```json
{
  "executions": [
    {
      "id": "exec_123",
      "routine_id": "550e8400-e29b-41d4-a716-446655440000",
      "run_id": "run_456",
      "scheduled_at": "2026-01-05T14:15:00Z",
      "executed_at": "2026-01-05T14:15:02Z",
      "status": "Completed",
      "error": null
    }
  ]
}
```

### Delete a Routine

```bash
curl -X DELETE http://localhost:8080/api/routines/550e8400-e29b-41d4-a716-446655440000
```

## Approval Board Management

### Create an Approval Board

Create a board with majority quorum rule:

```bash
curl -X POST http://localhost:8080/api/approval-boards \
  -H "Content-Type: application/json" \
  -d '{
    "id": "engineering_leads",
    "name": "Engineering Leadership Board",
    "description": "Senior engineers who approve architectural changes",
    "approvers": [
      "alice@example.com",
      "bob@example.com",
      "carol@example.com"
    ],
    "quorum_rule": "Majority",
    "created_at": "2026-01-05T10:00:00Z",
    "updated_at": "2026-01-05T10:00:00Z"
  }'
```

### Quorum Rules

Shiioo supports four types of quorum rules:

1. **Unanimous** - All approvers must approve
```json
"quorum_rule": "Unanimous"
```

2. **Majority** - More than 50% must approve
```json
"quorum_rule": "Majority"
```

3. **MinCount** - At least N approvers must approve
```json
"quorum_rule": {
  "MinCount": {
    "min": 2
  }
}
```

4. **Percentage** - At least X% of approvers must approve
```json
"quorum_rule": {
  "Percentage": {
    "percent": 66
  }
}
```

### List All Approval Boards

```bash
curl http://localhost:8080/api/approval-boards
```

### Get Specific Board

```bash
curl http://localhost:8080/api/approval-boards/engineering_leads
```

### Delete an Approval Board

```bash
curl -X DELETE http://localhost:8080/api/approval-boards/engineering_leads
```

## Config Change Management

### Propose a Config Change

Propose a policy change that requires approval:

```bash
curl -X POST http://localhost:8080/api/config-changes \
  -H "Content-Type: application/json" \
  -d '{
    "change_type": "Policy",
    "description": "Increase maximum budget for engineering team",
    "before": "{\"max_budget\": 1000}",
    "after": "{\"max_budget\": 2000}",
    "proposed_by": "alice@example.com",
    "approval_board": "engineering_leads"
  }'
```

Response:
```json
{
  "change_id": "change_789",
  "approval_id": "approval_101",
  "message": "Config change proposed successfully"
}
```

### Change Types

- `Policy` - Policy configuration changes
- `Role` - Role definition changes
- `Organization` - Organizational structure changes
- `Routine` - Routine schedule or workflow changes
- `Capacity` - Capacity source configuration changes

### List All Config Changes

```bash
curl http://localhost:8080/api/config-changes
```

Response:
```json
{
  "changes": [
    {
      "id": "change_789",
      "change_type": "Policy",
      "description": "Increase maximum budget for engineering team",
      "proposed_by": "alice@example.com",
      "approval_id": "approval_101",
      "status": "PendingApproval",
      "before": "{\"max_budget\": 1000}",
      "after": "{\"max_budget\": 2000}",
      "applied_at": null,
      "created_at": "2026-01-05T10:00:00Z"
    }
  ]
}
```

### Change Status Values

- `Proposed` - Change proposed, no approval required
- `PendingApproval` - Waiting for approval board vote
- `Approved` - Approval board approved, ready to apply
- `Rejected` - Approval board rejected
- `Applied` - Change has been applied
- `Failed` - Application failed

## Approval Voting Workflow

### List Pending Approvals

```bash
curl http://localhost:8080/api/approvals
```

Response:
```json
{
  "approvals": [
    {
      "id": "approval_101",
      "board_id": "engineering_leads",
      "subject": {
        "ConfigChange": {
          "change_id": "change_789"
        }
      },
      "requested_by": "alice@example.com",
      "status": "Pending",
      "votes": [],
      "created_at": "2026-01-05T10:00:00Z",
      "resolved_at": null
    }
  ]
}
```

### Cast a Vote

```bash
curl -X POST http://localhost:8080/api/approvals/approval_101/vote \
  -H "Content-Type: application/json" \
  -d '{
    "voter_id": "bob@example.com",
    "decision": "Approve",
    "comment": "LGTM - budget increase is justified"
  }'
```

Vote decisions:
- `Approve` - Vote to approve
- `Reject` - Vote to reject
- `Abstain` - Abstain from voting

Response:
```json
{
  "message": "Vote cast successfully"
}
```

### Apply an Approved Change

Once a config change has been approved, apply it:

```bash
curl -X POST http://localhost:8080/api/config-changes/change_789/apply
```

Response:
```json
{
  "message": "Config change applied successfully"
}
```

### Reject a Config Change

Reject a change with a reason:

```bash
curl -X POST http://localhost:8080/api/config-changes/change_789/reject \
  -H "Content-Type: application/json" \
  -d '{
    "reason": "Budget increase not justified by current metrics"
  }'
```

## Complete Governance Workflow Example

### Scenario: Schedule a Weekly Budget Review

1. **Create an approval board for finance team:**
```bash
curl -X POST http://localhost:8080/api/approval-boards \
  -H "Content-Type: application/json" \
  -d '{
    "id": "finance_board",
    "name": "Finance Review Board",
    "description": "CFO and finance leads",
    "approvers": ["cfo@example.com", "finance_lead@example.com"],
    "quorum_rule": "Majority",
    "created_at": "2026-01-05T10:00:00Z",
    "updated_at": "2026-01-05T10:00:00Z"
  }'
```

2. **Create a weekly routine for budget review:**
```bash
curl -X POST http://localhost:8080/api/routines \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Weekly Budget Review",
    "description": "Review and approve budget allocations",
    "schedule": {
      "cron": "0 9 * * 1",
      "timezone": "UTC"
    },
    "workflow": {
      "steps": [
        {
          "id": "generate_report",
          "name": "Generate Budget Report",
          "role": "finance_analyst",
          "action": {
            "agent_task": {
              "prompt": "Generate weekly budget utilization report"
            }
          }
        },
        {
          "id": "propose_adjustments",
          "name": "Propose Budget Adjustments",
          "role": "finance_analyst",
          "action": {
            "agent_task": {
              "prompt": "Analyze report and propose budget adjustments"
            }
          }
        }
      ],
      "dependencies": {
        "propose_adjustments": ["generate_report"]
      }
    },
    "enabled": true,
    "created_by": "finance_lead@example.com"
  }'
```

3. **When routine runs, it can propose a config change:**
```bash
curl -X POST http://localhost:8080/api/config-changes \
  -H "Content-Type: application/json" \
  -d '{
    "change_type": "Policy",
    "description": "Adjust Q1 engineering budget based on utilization",
    "before": "{\"engineering_budget\": 50000}",
    "after": "{\"engineering_budget\": 55000}",
    "proposed_by": "finance_analyst_agent",
    "approval_board": "finance_board"
  }'
```

4. **Finance board members vote:**
```bash
# CFO approves
curl -X POST http://localhost:8080/api/approvals/approval_XYZ/vote \
  -H "Content-Type: application/json" \
  -d '{
    "voter_id": "cfo@example.com",
    "decision": "Approve",
    "comment": "Budget adjustment approved"
  }'

# Finance lead approves (reaches majority quorum)
curl -X POST http://localhost:8080/api/approvals/approval_XYZ/vote \
  -H "Content-Type: application/json" \
  -d '{
    "voter_id": "finance_lead@example.com",
    "decision": "Approve",
    "comment": "Utilization data supports the increase"
  }'
```

5. **Apply the approved change:**
```bash
curl -X POST http://localhost:8080/api/config-changes/change_XYZ/apply
```

## Best Practices

### Routine Scheduling

- **Use appropriate intervals**: Don't schedule routines more frequently than necessary
- **Set timeouts**: Always specify `timeout_secs` for routine steps
- **Monitor execution history**: Regularly check execution logs for failures
- **Disable before deleting**: Disable routines before deleting to avoid mid-execution issues

### Approval Boards

- **Choose appropriate quorum rules**:
  - Use Unanimous for critical changes (e.g., production deployments)
  - Use Majority for regular changes
  - Use MinCount when you have a large board but want fast approvals
  - Use Percentage for flexible scaling as team size changes
- **Document approval board purpose**: Clear descriptions help approvers understand their role
- **Keep boards small**: 3-5 approvers is optimal for most boards

### Config Changes

- **Always include before/after**: Document current and proposed state
- **Write clear descriptions**: Help approvers understand the impact
- **Use appropriate change types**: Correct categorization aids in auditing
- **Apply promptly after approval**: Don't let approved changes sit unapplied

### Security Considerations

- **Require approvals for sensitive changes**: Policy, role, and capacity changes should require approval
- **Audit trails**: All approvals and config changes are logged with timestamps and actors
- **Revoke carefully**: Removing approvers from boards doesn't retroactively invalidate their votes
- **Rate limit checking**: Routines respect capacity limits and rate limiting

## Troubleshooting

### Routine Not Executing

1. Check if routine is enabled: `GET /api/routines/{id}`
2. Verify cron expression is valid
3. Check execution history for errors
4. Ensure the workflow executor has capacity

### Approval Stuck in Pending

1. List approvals to see vote status: `GET /api/approvals/{id}`
2. Check quorum rule - ensure enough approvers have voted
3. Verify all voters are valid members of the approval board

### Config Change Can't Be Applied

1. Check approval status: Change must be in `Approved` status
2. Verify the approval board voted and reached quorum
3. Check logs for application errors
4. Ensure the change hasn't already been applied

## API Reference Summary

### Routines
- `POST /api/routines` - Create routine
- `GET /api/routines` - List all routines
- `GET /api/routines/{id}` - Get specific routine
- `DELETE /api/routines/{id}` - Delete routine
- `POST /api/routines/{id}/enable` - Enable routine
- `POST /api/routines/{id}/disable` - Disable routine
- `GET /api/routines/{id}/executions` - Get execution history

### Approval Boards
- `POST /api/approval-boards` - Create board
- `GET /api/approval-boards` - List all boards
- `GET /api/approval-boards/{id}` - Get specific board
- `DELETE /api/approval-boards/{id}` - Delete board

### Approvals
- `GET /api/approvals` - List all approvals
- `GET /api/approvals/{id}` - Get specific approval
- `POST /api/approvals/{id}/vote` - Cast a vote

### Config Changes
- `POST /api/config-changes` - Propose change
- `GET /api/config-changes` - List all changes
- `GET /api/config-changes/{id}` - Get specific change
- `POST /api/config-changes/{id}/apply` - Apply approved change
- `POST /api/config-changes/{id}/reject` - Reject change
