# Phase 3: Organization & Templates Guide

This guide demonstrates how to use Shiioo's organization management, process templates, and Claude configuration compiler introduced in Phase 3.

## Overview

Phase 3 adds:
- **Organization Management** - Define teams, people, roles, reporting structure
- **Process Templates** - Reusable workflow patterns with parameters
- **Claude Config Compiler** - Generate `.claude/config.json` from org setup
- **GitOps-friendly** - Store org config in version control

## Organization Management

### Create an Organization

```bash
curl -X POST http://localhost:8080/api/organizations \
  -H "Content-Type: application/json" \
  -d '{
    "id": "my_company",
    "name": "My Company",
    "description": "Example organization",
    "teams": [
      {
        "id": "executive",
        "name": "Executive Team",
        "description": "C-suite and leadership",
        "lead": "ceo",
        "members": ["ceo", "cto", "cfo"],
        "parent_team": null
      },
      {
        "id": "engineering",
        "name": "Engineering",
        "description": "Product development team",
        "lead": "cto",
        "members": ["cto", "eng1", "eng2"],
        "parent_team": "executive"
      },
      {
        "id": "data",
        "name": "Data Analytics",
        "description": "Data science and analytics",
        "lead": "data_lead",
        "members": ["data_lead", "analyst1", "analyst2"],
        "parent_team": "engineering"
      }
    ],
    "people": [
      {
        "id": "ceo",
        "name": "Alice CEO",
        "email": "alice@company.com",
        "role": "executive",
        "team": "executive",
        "reports_to": null,
        "can_approve": ["all", "budget", "technical", "policy"]
      },
      {
        "id": "cto",
        "name": "Bob CTO",
        "email": "bob@company.com",
        "role": "executive",
        "team": "engineering",
        "reports_to": "ceo",
        "can_approve": ["technical", "deployment"]
      },
      {
        "id": "eng1",
        "name": "Charlie Engineer",
        "email": "charlie@company.com",
        "role": "engineer",
        "team": "engineering",
        "reports_to": "cto",
        "can_approve": []
      },
      {
        "id": "analyst1",
        "name": "Dana Analyst",
        "email": "dana@company.com",
        "role": "analyst",
        "team": "data",
        "reports_to": "data_lead",
        "can_approve": []
      }
    ],
    "org_chart": {
      "root_team": "executive",
      "reporting_structure": {
        "cto": "ceo",
        "eng1": "cto",
        "data_lead": "cto",
        "analyst1": "data_lead"
      }
    },
    "created_at": "2026-01-05T20:00:00Z",
    "updated_at": "2026-01-05T20:00:00Z"
  }'
```

### Query Organization

```bash
# List all organizations
curl http://localhost:8080/api/organizations

# Get specific organization
curl http://localhost:8080/api/organizations/my_company
```

### Organization Validation

The system automatically validates:
- All team members exist as people
- Team leads exist
- Parent teams exist
- No cycles in reporting structure
- Root team exists

## Process Templates

### Create a Template

Templates are reusable workflow patterns with parameters:

```bash
curl -X POST http://localhost:8080/api/templates \
  -H "Content-Type: application/json" \
  -d '{
    "id": "code_review",
    "name": "Standard Code Review",
    "description": "Review code changes with automated checks",
    "category": "code_review",
    "parameters": [
      {
        "name": "file_path",
        "description": "Path to the file to review",
        "param_type": "string",
        "default_value": null,
        "required": true
      },
      {
        "name": "reviewer",
        "description": "Person who will review (PersonId)",
        "param_type": "person_id",
        "default_value": "cto",
        "required": false
      },
      {
        "name": "severity_threshold",
        "description": "Minimum severity to report (low, medium, high)",
        "param_type": "string",
        "default_value": "medium",
        "required": false
      }
    ],
    "workflow_template": {
      "steps": [
        {
          "id": "static_analysis",
          "name": "Run static analysis on {{file_path}}",
          "description": "Automated code analysis",
          "role": "engineer",
          "action": {
            "type": "agent_task",
            "prompt": "Run static analysis on {{file_path}} and report issues with severity >= {{severity_threshold}}"
          },
          "timeout_secs": 120,
          "retry_policy": {
            "max_attempts": 2,
            "backoff_secs": 5
          },
          "requires_approval": false
        },
        {
          "id": "security_scan",
          "name": "Security scan",
          "description": "Check for security vulnerabilities",
          "role": "security_auditor",
          "action": {
            "type": "agent_task",
            "prompt": "Scan {{file_path}} for security vulnerabilities (OWASP Top 10, CWE)"
          },
          "timeout_secs": 180,
          "requires_approval": false
        },
        {
          "id": "manual_review",
          "name": "Manual code review",
          "description": "Human review of changes",
          "role": "engineer",
          "action": {
            "type": "manual_approval",
            "approvers": ["{{reviewer}}"]
          },
          "requires_approval": true
        }
      ],
      "dependencies": {
        "manual_review": ["static_analysis", "security_scan"]
      }
    },
    "created_at": "2026-01-05T20:00:00Z",
    "created_by": "admin"
  }'
```

### Instantiate a Template

```bash
curl -X POST http://localhost:8080/api/templates/code_review/instantiate \
  -H "Content-Type: application/json" \
  -d '{
    "template_id": "code_review",
    "parameters": {
      "file_path": "src/auth/login.rs",
      "reviewer": "cto",
      "severity_threshold": "high"
    },
    "created_at": "2026-01-05T20:15:00Z",
    "created_by": "eng1"
  }'
```

This returns a fully instantiated `WorkflowSpec` with all parameters replaced.

### Template Parameter Types

- `string` - Any text value
- `number` - Numeric value (validated)
- `boolean` - true/false (validated)
- `role_id` - Reference to a role
- `team_id` - Reference to a team
- `person_id` - Reference to a person

### More Template Examples

**Deployment Template:**

```bash
curl -X POST http://localhost:8080/api/templates \
  -H "Content-Type: application/json" \
  -d '{
    "id": "deploy_service",
    "name": "Deploy Service to Environment",
    "description": "Standard deployment workflow with checks",
    "category": "deployment",
    "parameters": [
      {
        "name": "service_name",
        "description": "Name of the service to deploy",
        "param_type": "string",
        "required": true
      },
      {
        "name": "environment",
        "description": "Target environment (staging, production)",
        "param_type": "string",
        "required": true
      },
      {
        "name": "approver",
        "description": "Who must approve production deploys",
        "param_type": "person_id",
        "default_value": "cto",
        "required": false
      }
    ],
    "workflow_template": {
      "steps": [
        {
          "id": "run_tests",
          "name": "Run test suite for {{service_name}}",
          "description": "Execute all tests",
          "role": "engineer",
          "action": {
            "type": "script",
            "command": "cargo",
            "args": ["test", "--package", "{{service_name}}"]
          },
          "timeout_secs": 300
        },
        {
          "id": "build",
          "name": "Build {{service_name}}",
          "description": "Compile release binary",
          "role": "engineer",
          "action": {
            "type": "script",
            "command": "cargo",
            "args": ["build", "--release", "--package", "{{service_name}}"]
          },
          "timeout_secs": 600
        },
        {
          "id": "deploy",
          "name": "Deploy to {{environment}}",
          "description": "Deploy service",
          "role": "engineer",
          "action": {
            "type": "script",
            "command": "./scripts/deploy.sh",
            "args": ["{{service_name}}", "{{environment}}"]
          },
          "timeout_secs": 300,
          "requires_approval": true
        }
      ],
      "dependencies": {
        "build": ["run_tests"],
        "deploy": ["build"]
      }
    }
  }'
```

## Claude Config Compiler

### Generate Claude Configuration for a Role

The compiler generates `.claude/config.json` based on your organization setup:

```bash
curl http://localhost:8080/api/claude/compile/engineer
```

Response:
```json
{
  "config": {
    "mcp_servers": {
      "shiioo": {
        "command": "shiioo-mcp",
        "args": [],
        "env": {
          "SHIIOO_DATA_DIR": "./data",
          "SHIIOO_ORG_ID": "my_company"
        }
      }
    },
    "tools": [
      {
        "name": "context_search",
        "enabled": true,
        "tier": 0,
        "requires_approval": false
      },
      {
        "name": "repo_read",
        "enabled": true,
        "tier": 0,
        "requires_approval": false
      },
      {
        "name": "repo_write",
        "enabled": true,
        "tier": 1,
        "requires_approval": true
      },
      {
        "name": "database_execute",
        "enabled": false,
        "tier": 2,
        "requires_approval": true
      }
    ],
    "settings": {
      "max_tokens": 10000,
      "temperature": 0.7,
      "model": "claude-opus-4-5"
    }
  },
  "readme": "# Claude Code Configuration for Software Engineer\n\n...",
  "message": "Claude configuration compiled successfully"
}
```

### Using the Generated Config

1. **Save the configuration:**
   ```bash
   curl http://localhost:8080/api/claude/compile/engineer | \
     jq '.config' > .claude/config.json
   ```

2. **Save the README:**
   ```bash
   curl http://localhost:8080/api/claude/compile/engineer | \
     jq -r '.readme' > .claude/README.md
   ```

3. **Start Claude Code** - it will automatically use the configuration!

### GitOps Workflow

Store your org configuration in Git:

1. **Define organization in YAML/JSON:**
   ```yaml
   # org.yaml
   id: my_company
   name: My Company
   teams:
     - id: engineering
       name: Engineering
       # ...
   ```

2. **Apply to Shiioo:**
   ```bash
   curl -X POST http://localhost:8080/api/organizations \
     -H "Content-Type: application/json" \
     -d @org.json
   ```

3. **Generate Claude configs for all roles:**
   ```bash
   for role in engineer analyst security_auditor; do
     curl http://localhost:8080/api/claude/compile/$role | \
       jq '.config' > .claude/config-$role.json
   done
   ```

4. **Commit to version control:**
   ```bash
   git add org.json .claude/
   git commit -m "Update org structure and Claude configs"
   ```

## Complete Example: Set Up a New Org

```bash
# 1. Create roles
curl -X POST http://localhost:8080/api/roles -d @roles/engineer.json
curl -X POST http://localhost:8080/api/roles -d @roles/analyst.json

# 2. Create policies
curl -X POST http://localhost:8080/api/policies -d @policies/no_secrets.json

# 3. Create organization
curl -X POST http://localhost:8080/api/organizations -d @org.json

# 4. Create templates
curl -X POST http://localhost:8080/api/templates -d @templates/code_review.json
curl -X POST http://localhost:8080/api/templates -d @templates/deploy.json

# 5. Generate Claude configs for each role
for role in engineer analyst security_auditor; do
  curl http://localhost:8080/api/claude/compile/$role | \
    jq '.config' > .claude/config-$role.json
  curl http://localhost:8080/api/claude/compile/$role | \
    jq -r '.readme' > .claude/README-$role.md
done

# 6. Instantiate a template and run it
WORKFLOW=$(curl -X POST http://localhost:8080/api/templates/code_review/instantiate \
  -H "Content-Type: application/json" \
  -d '{
    "template_id": "code_review",
    "parameters": {"file_path": "src/main.rs", "reviewer": "cto"},
    "created_at": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'",
    "created_by": "eng1"
  }' | jq '.workflow')

curl -X POST http://localhost:8080/api/jobs \
  -H "Content-Type: application/json" \
  -d "{
    \"name\": \"Review src/main.rs\",
    \"workflow\": $WORKFLOW,
    \"execute\": true
  }"
```

## Advanced Features

### Org Chart Queries

The OrganizationManager provides programmatic queries:

- `get_person(person_id)` - Get person details
- `get_team(team_id)` - Get team details
- `get_direct_reports(person_id)` - Who reports to this person
- `get_all_team_members(team_id)` - All members including sub-teams
- `can_approve(person_id, approval_type)` - Check approval permissions
- `get_management_chain(person_id)` - Chain of command to root

These are available via the Rust API for custom integrations.

### Template Parameter Extraction

Extract parameters from a template string:

```rust
use shiioo_core::template::TemplateProcessor;

let text = "Deploy {{service}} to {{environment}}";
let params = TemplateProcessor::extract_parameters(text);
// Returns: ["environment", "service"]
```

### Custom Claude Config

You can customize the generated config:

```rust
use shiioo_core::claude_compiler::ClaudeCompiler;

let compiler = ClaudeCompiler::new(org, roles, policies);
let mut config = compiler.compile_for_role(&role_id)?;

// Add custom MCP servers
config.mcp_servers.insert("custom_tool".to_string(), McpServerConfig {
    command: "custom-mcp".to_string(),
    args: vec!["--port".to_string(), "3000".to_string()],
    env: HashMap::new(),
});

// Adjust settings
config.settings.temperature = Some(0.5);
```

## Best Practices

1. **Version control everything** - Store org, roles, policies, templates in Git
2. **Validate before applying** - The API validates org structure automatically
3. **Use templates for common workflows** - Don't repeat yourself
4. **Generate Claude configs per environment** - dev, staging, production
5. **Audit approval permissions** - Review `can_approve` lists regularly
6. **Start small** - Begin with a simple org structure, expand as needed
7. **Test templates** - Instantiate with test parameters before production use

## Next Steps

See:
- [README.md](README.md) for overall architecture
- [POLICY_EXAMPLES.md](POLICY_EXAMPLES.md) for policy management
- [EXAMPLES.md](EXAMPLES.md) for workflow examples
