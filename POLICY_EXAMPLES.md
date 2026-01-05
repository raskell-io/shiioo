# Policy Engine Examples

This document demonstrates how to use Shiioo's policy engine for governance and authorization.

## Overview

The policy engine provides:
- **Role-based access control** - Define which tools each role can access
- **Budget enforcement** - Set daily token and cost limits per role
- **Approval requirements** - Require approval for sensitive operations
- **Policy rules** - Fine-grained rules like path deny lists, domain allowlists

## Creating Roles

### Analyst Role

An analyst can read data and search context, with moderate budget limits:

```bash
curl -X POST http://localhost:8080/api/roles \
  -H "Content-Type: application/json" \
  -d '{
    "id": "analyst",
    "name": "Data Analyst",
    "description": "Can read and search data, limited write access",
    "prompt_template": "You are a data analyst. Analyze data and provide insights.",
    "allowed_tools": [
      "context_search",
      "context_get",
      "repo_read",
      "web_fetch"
    ],
    "budgets": {
      "daily_tokens": 100000,
      "daily_cost_cents": 1000
    },
    "requires_approval_for": []
  }'
```

### Engineer Role

An engineer has broader access but requires approval for write operations:

```bash
curl -X POST http://localhost:8080/api/roles \
  -H "Content-Type: application/json" \
  -d '{
    "id": "engineer",
    "name": "Software Engineer",
    "description": "Can read and write code, requires approval for sensitive ops",
    "prompt_template": "You are a software engineer. Write clean, maintainable code.",
    "allowed_tools": [
      "context_search",
      "context_get",
      "repo_read",
      "repo_write",
      "web_fetch"
    ],
    "budgets": {
      "daily_tokens": 500000,
      "daily_cost_cents": 5000
    },
    "requires_approval_for": [
      "repo_write",
      "tier2"
    ]
  }'
```

### Security Auditor Role

A security role with read-only access and high budget:

```bash
curl -X POST http://localhost:8080/api/roles \
  -H "Content-Type: application/json" \
  -d '{
    "id": "security_auditor",
    "name": "Security Auditor",
    "description": "Read-only access for security audits",
    "prompt_template": "You are a security auditor. Review code for vulnerabilities.",
    "allowed_tools": [
      "context_search",
      "context_get",
      "context_events",
      "repo_read"
    ],
    "budgets": {
      "daily_tokens": 1000000,
      "daily_cost_cents": 10000
    },
    "requires_approval_for": []
  }'
```

## Creating Policies

### No Secrets Policy

Deny access to secret files:

```bash
curl -X POST http://localhost:8080/api/policies \
  -H "Content-Type: application/json" \
  -d '{
    "id": "no_secrets",
    "name": "No Secrets Access",
    "description": "Prevent access to credential and secret files",
    "rules": [
      {
        "type": "deny_path",
        "patterns": [
          ".env",
          "credentials",
          "secrets",
          ".pem",
          ".key",
          "id_rsa",
          "id_ed25519",
          "password",
          "token"
        ]
      }
    ]
  }'
```

### Trusted Domains Policy

Allow web fetches only from trusted domains:

```bash
curl -X POST http://localhost:8080/api/policies \
  -H "Content-Type: application/json" \
  -d '{
    "id": "trusted_domains",
    "name": "Trusted Domains Only",
    "description": "Allow web fetches only from approved domains",
    "rules": [
      {
        "type": "allow_domain",
        "domains": [
          "github.com",
          "docs.rs",
          "doc.rust-lang.org",
          "crates.io",
          "wikipedia.org"
        ]
      }
    ]
  }'
```

### Critical Operations Policy

Require approval for database and production changes:

```bash
curl -X POST http://localhost:8080/api/policies \
  -H "Content-Type: application/json" \
  -d '{
    "id": "critical_ops",
    "name": "Critical Operations Approval",
    "description": "Require approval for dangerous operations",
    "rules": [
      {
        "type": "require_approval",
        "tool_ids": [
          "database_execute",
          "deploy_production",
          "delete_data"
        ]
      }
    ]
  }'
```

## Policy Evaluation Flow

When a role attempts to use a tool, the policy engine checks:

1. **Tool Permission**: Is the tool in the role's `allowed_tools` list?
2. **Budget Limits**: Has the role exceeded daily token or cost budgets?
3. **Approval Requirements**: Does this tool require approval (via role or policy)?
4. **Policy Rules**: Do any global policies deny or restrict this operation?

### Decision Types

- **Allow**: Operation proceeds immediately
- **Deny**: Operation blocked with reason
- **RequiresApproval**: Operation queued for human approval

## Querying Roles and Policies

### List all roles

```bash
curl http://localhost:8080/api/roles
```

### Get a specific role

```bash
curl http://localhost:8080/api/roles/analyst
```

### List all policies

```bash
curl http://localhost:8080/api/policies
```

### Get a specific policy

```bash
curl http://localhost:8080/api/policies/no_secrets
```

## Budget Tracking

The policy engine automatically tracks:
- **Tokens used** per role per day
- **Cost in cents** per role per day
- **Automatic reset** at midnight UTC

Example workflow with budget enforcement:

```json
{
  "name": "Data Analysis Job",
  "workflow": {
    "steps": [
      {
        "id": "analyze",
        "name": "Analyze Dataset",
        "role": "analyst",  // Uses analyst role and budgets
        "action": {
          "type": "agent_task",
          "prompt": "Analyze the sales data and provide insights"
        }
      }
    ]
  }
}
```

If the analyst role has already used 95,000 of its 100,000 daily token budget, this task will be denied if it would exceed the limit.

## Approval Workflow

When a step requires approval:

```json
{
  "id": "deploy",
  "name": "Deploy to Production",
  "role": "engineer",
  "action": {
    "type": "agent_task",
    "prompt": "Deploy the new feature to production"
  },
  "requires_approval": true
}
```

The policy engine returns:
```json
{
  "decision": "RequiresApproval",
  "approvers": ["ceo", "cto"]
}
```

The step is queued until approved by one of the designated approvers.

## Deleting Roles and Policies

### Delete a role

```bash
curl -X DELETE http://localhost:8080/api/roles/analyst
```

### Delete a policy

```bash
curl -X DELETE http://localhost:8080/api/policies/no_secrets
```

## Integration with Workflows

Roles and policies are enforced automatically during workflow execution:

```bash
curl -X POST http://localhost:8080/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Code Review Workflow",
    "description": "Multi-step code review with different roles",
    "workflow": {
      "steps": [
        {
          "id": "read_code",
          "name": "Read Code",
          "role": "security_auditor",
          "action": {
            "type": "agent_task",
            "prompt": "Review src/main.rs for security issues"
          }
        },
        {
          "id": "propose_fix",
          "name": "Propose Fix",
          "role": "engineer",
          "action": {
            "type": "agent_task",
            "prompt": "Write a fix for the security issue"
          },
          "requires_approval": true
        }
      ],
      "dependencies": {
        "propose_fix": ["read_code"]
      }
    },
    "execute": true
  }'
```

In this workflow:
1. The security auditor reads the code (read-only, no approval needed)
2. The engineer proposes a fix (requires approval due to role settings)
3. Budget tracking happens automatically for both roles

## Best Practices

1. **Start restrictive**: Give roles minimal permissions, expand as needed
2. **Use tier-based approvals**: Require approval for all tier1+ tools for sensitive roles
3. **Set realistic budgets**: Monitor actual usage and adjust daily limits
4. **Combine policies**: Use multiple policies for defense in depth
5. **Audit regularly**: Review roles and policies quarterly
6. **Test in staging**: Validate policy changes in non-production first

## Next Steps

See [README.md](README.md) for more information about:
- Workflow DAG execution
- Event sourcing
- MCP tool integration
- Configuration management
