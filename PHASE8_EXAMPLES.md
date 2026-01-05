# Phase 8: Advanced Features Examples

This document provides comprehensive examples of the advanced features implemented in Phase 8.

## Table of Contents

- [Overview](#overview)
- [Secret Management](#secret-management)
  - [Creating Secrets](#creating-secrets)
  - [Retrieving Secrets](#retrieving-secrets)
  - [Rotating Secrets](#rotating-secrets)
  - [Secret Versioning](#secret-versioning)
  - [Rotation Policies](#rotation-policies)
- [Advanced Workflow Patterns](#advanced-workflow-patterns)
  - [Parallel-For-Each](#parallel-for-each)
  - [Conditional Branches](#conditional-branches)
  - [Dynamic DAG Generation](#dynamic-dag-generation)
  - [Loop Constructs](#loop-constructs)
- [Workflow Versioning](#workflow-versioning)
- [API Reference](#api-reference)

## Overview

Phase 8 introduces **advanced features** to Shiioo:

- **Secret Management**: Encrypted storage, rotation policies, version history
- **Advanced Workflow Patterns**: Parallel-for-each, conditional branches, dynamic DAGs, loops
- **Workflow Versioning**: Track and manage workflow schema versions

## Secret Management

### Creating Secrets

#### Create an API key secret

```bash
curl -X POST http://localhost:3000/api/secrets \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Anthropic API Key",
    "description": "Production API key for Claude API",
    "secret_type": "ApiKey",
    "value": "sk-ant-api03-...",
    "rotation_policy": {
      "enabled": true,
      "rotation_interval_days": 90,
      "grace_period_days": 7,
      "notify_before_days": 7
    },
    "tags": {
      "environment": "production",
      "service": "claude-api"
    }
  }'
```

Response:
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Anthropic API Key",
  "description": "Production API key for Claude API",
  "secret_type": "ApiKey",
  "encrypted_value": "xM3fD...",
  "value_hash": "a1b2c3d4...",
  "version": 1,
  "rotation_policy": {
    "enabled": true,
    "rotation_interval_days": 90,
    "grace_period_days": 7,
    "notify_before_days": 7
  },
  "tags": {
    "environment": "production",
    "service": "claude-api"
  },
  "created_at": "2024-01-05T12:00:00Z",
  "updated_at": "2024-01-05T12:00:00Z",
  "last_rotated_at": null,
  "expires_at": null
}
```

#### Create a database password

```bash
curl -X POST http://localhost:3000/api/secrets \
  -H "Content-Type: application/json" \
  -d '{
    "name": "PostgreSQL Password",
    "description": "Production database password",
    "secret_type": "DatabasePassword",
    "value": "super-secret-password",
    "rotation_policy": {
      "enabled": true,
      "rotation_interval_days": 30,
      "grace_period_days": 3,
      "notify_before_days": 5
    }
  }'
```

### Retrieving Secrets

#### List all secrets (without values)

```bash
curl http://localhost:3000/api/secrets
```

Response:
```json
{
  "secrets": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "Anthropic API Key",
      "version": 1,
      "secret_type": "ApiKey",
      "created_at": "2024-01-05T12:00:00Z"
    },
    {
      "id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
      "name": "PostgreSQL Password",
      "version": 2,
      "secret_type": "DatabasePassword",
      "created_at": "2024-01-03T10:30:00Z"
    }
  ]
}
```

#### Get secret metadata (without value)

```bash
curl http://localhost:3000/api/secrets/550e8400-e29b-41d4-a716-446655440000
```

#### Get decrypted secret value

⚠️ **Security Note**: Use this endpoint carefully. Consider implementing additional authentication/authorization.

```bash
curl http://localhost:3000/api/secrets/550e8400-e29b-41d4-a716-446655440000/value
```

Response:
```json
{
  "value": "sk-ant-api03-..."
}
```

### Rotating Secrets

#### Manual rotation

```bash
curl -X POST http://localhost:3000/api/secrets/550e8400-e29b-41d4-a716-446655440000/rotate \
  -H "Content-Type: application/json" \
  -d '{
    "new_value": "sk-ant-api03-new-rotated-key-..."
  }'
```

Response:
```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "Anthropic API Key",
  "version": 2,
  "last_rotated_at": "2024-01-10T14:30:00Z",
  "encrypted_value": "yN4gE...",
  "value_hash": "b2c3d4e5..."
}
```

#### Check secrets needing rotation

```bash
curl http://localhost:3000/api/secrets/rotation/needed
```

Response:
```json
{
  "secrets": [
    {
      "id": "6ba7b810-9dad-11d1-80b4-00c04fd430c8",
      "name": "PostgreSQL Password",
      "version": 1,
      "last_rotated_at": "2023-11-05T10:00:00Z",
      "rotation_policy": {
        "enabled": true,
        "rotation_interval_days": 30
      }
    }
  ]
}
```

### Secret Versioning

#### Get version history

```bash
curl http://localhost:3000/api/secrets/550e8400-e29b-41d4-a716-446655440000/versions
```

Response:
```json
{
  "versions": [
    {
      "secret_id": "550e8400-e29b-41d4-a716-446655440000",
      "version": 1,
      "encrypted_value": "xM3fD...",
      "value_hash": "a1b2c3d4...",
      "created_at": "2024-01-05T12:00:00Z",
      "deprecated_at": "2024-01-10T14:30:00Z"
    },
    {
      "secret_id": "550e8400-e29b-41d4-a716-446655440000",
      "version": 2,
      "encrypted_value": "yN4gE...",
      "value_hash": "b2c3d4e5...",
      "created_at": "2024-01-10T14:30:00Z",
      "deprecated_at": null
    }
  ]
}
```

### Rotation Policies

Rotation policies control automatic secret rotation:

```rust
RotationPolicy {
    enabled: true,
    rotation_interval_days: 90,  // Rotate every 90 days
    grace_period_days: 7,         // Old secret valid for 7 days after rotation
    notify_before_days: 7,        // Notify 7 days before rotation due
}
```

#### Update secret metadata and policy

```bash
curl -X PUT http://localhost:3000/api/secrets/550e8400-e29b-41d4-a716-446655440000 \
  -H "Content-Type: application/json" \
  -d '{
    "rotation_policy": {
      "enabled": true,
      "rotation_interval_days": 60,
      "grace_period_days": 14,
      "notify_before_days": 14
    },
    "tags": {
      "environment": "production",
      "service": "claude-api",
      "compliance": "pci-dss"
    }
  }'
```

#### Delete a secret

```bash
curl -X DELETE http://localhost:3000/api/secrets/550e8400-e29b-41d4-a716-446655440000
```

## Advanced Workflow Patterns

### Parallel-For-Each

Execute the same workflow step for multiple items in parallel.

#### Code Example

```rust
use shiioo_core::workflow::{ParallelForEachBuilder, AdvancedPattern};
use shiioo_core::types::{StepSpec, StepAction, StepId, RoleId};

let items = vec![
    serde_json::json!({"user_id": "user1", "email": "user1@example.com"}),
    serde_json::json!({"user_id": "user2", "email": "user2@example.com"}),
    serde_json::json!({"user_id": "user3", "email": "user3@example.com"}),
];

let step_template = StepSpec {
    id: StepId("notify_user".to_string()),
    name: "Notify User".to_string(),
    description: Some("Send notification to user".to_string()),
    role: RoleId("notification_agent".to_string()),
    action: StepAction::AgentTask {
        prompt: "Send email notification to {{item}}".to_string(),
    },
    timeout_secs: Some(60),
    retry_policy: None,
    requires_approval: false,
};

let pattern = ParallelForEachBuilder::new()
    .items(items)
    .step_template(step_template)
    .max_parallelism(5)  // Run max 5 in parallel
    .build()?;
```

#### API Usage

Create a workflow with parallel-for-each pattern:

```json
{
  "name": "Bulk User Notifications",
  "steps": [
    {
      "id": "notify_users",
      "pattern": {
        "type": "ParallelForEach",
        "items": [
          {"user_id": "user1", "email": "user1@example.com"},
          {"user_id": "user2", "email": "user2@example.com"}
        ],
        "step_template": {
          "id": "notify",
          "name": "Notify User",
          "role": "notification_agent",
          "action": {
            "type": "agent_task",
            "prompt": "Send notification to {{item}}"
          }
        },
        "max_parallelism": 10
      }
    }
  ]
}
```

### Conditional Branches

Execute different steps based on runtime conditions.

#### Code Example

```rust
use shiioo_core::workflow::{AdvancedPattern, evaluate_condition};
use std::collections::HashMap;

let mut context = HashMap::new();
context.insert("status".to_string(), "approved".to_string());
context.insert("priority".to_string(), "high".to_string());

// Simple conditions
assert!(evaluate_condition("status == approved", &context)?);
assert!(evaluate_condition("priority != low", &context)?);

// Numeric conditions
context.insert("count".to_string(), "5".to_string());
assert!(evaluate_condition("count > 3", &context)?);
assert!(evaluate_condition("count <= 10", &context)?);

// Variable existence
assert!(evaluate_condition("status", &context)?);
assert!(!evaluate_condition("missing_field", &context)?);
```

#### Workflow Pattern

```json
{
  "type": "ConditionalBranch",
  "condition": "status == approved",
  "if_steps": [
    {
      "id": "deploy",
      "name": "Deploy to Production",
      "action": {"type": "agent_task", "prompt": "Deploy the application"}
    }
  ],
  "else_steps": [
    {
      "id": "notify_rejection",
      "name": "Notify Rejection",
      "action": {"type": "agent_task", "prompt": "Send rejection notification"}
    }
  ]
}
```

### Dynamic DAG Generation

Generate workflow steps dynamically at runtime based on execution results.

```json
{
  "type": "DynamicDAG",
  "generator_step": {
    "id": "generate_steps",
    "name": "Generate Workflow Steps",
    "role": "orchestrator",
    "action": {
      "type": "agent_task",
      "prompt": "Analyze requirements and generate workflow steps"
    }
  },
  "execute_generated": true
}
```

### Loop Constructs

Repeat steps until a condition is met.

```json
{
  "type": "Loop",
  "condition": "retry_count < max_retries",
  "max_iterations": 10,
  "loop_steps": [
    {
      "id": "attempt_operation",
      "name": "Attempt Operation",
      "action": {"type": "agent_task", "prompt": "Execute operation"}
    },
    {
      "id": "check_result",
      "name": "Check Result",
      "action": {"type": "agent_task", "prompt": "Verify operation succeeded"}
    }
  ]
}
```

## Workflow Versioning

Track and manage workflow schema versions over time.

### Code Example

```rust
use shiioo_core::workflow::{WorkflowVersionManager, WorkflowVersion};
use shiioo_core::types::WorkflowSpec;
use std::collections::HashMap;

let mut manager = WorkflowVersionManager::new();

// Register initial version
let v1_spec = WorkflowSpec {
    steps: vec![/* ... */],
    dependencies: HashMap::new(),
};

manager.register_version(
    "onboarding-workflow".to_string(),
    v1_spec,
    "admin@example.com".to_string(),
    "Initial onboarding workflow".to_string(),
);

// Register updated version
let v2_spec = WorkflowSpec {
    steps: vec![/* enhanced steps */],
    dependencies: HashMap::new(),
};

manager.register_version(
    "onboarding-workflow".to_string(),
    v2_spec,
    "admin@example.com".to_string(),
    "Added email verification step".to_string(),
);

// Get latest version
let latest = manager.get_latest_version("onboarding-workflow").unwrap();
println!("Latest version: {}", latest.version);

// Get specific version
let v1 = manager.get_version("onboarding-workflow", 1).unwrap();

// Deprecate old version
manager.deprecate_version("onboarding-workflow", 1)?;
```

### Version Information

```rust
pub struct WorkflowVersion {
    pub workflow_id: String,
    pub version: u32,
    pub spec: WorkflowSpec,
    pub created_at: DateTime<Utc>,
    pub created_by: String,
    pub changelog: String,
    pub is_deprecated: bool,
}
```

## API Reference

### Secret Management Endpoints

| Method | Endpoint | Description |
|--------|----------|-------------|
| `POST` | `/api/secrets` | Create a new secret |
| `GET` | `/api/secrets` | List all secrets (without values) |
| `GET` | `/api/secrets/{secret_id}` | Get secret metadata |
| `GET` | `/api/secrets/{secret_id}/value` | Get decrypted secret value |
| `PUT` | `/api/secrets/{secret_id}` | Update secret metadata |
| `DELETE` | `/api/secrets/{secret_id}` | Delete a secret |
| `POST` | `/api/secrets/{secret_id}/rotate` | Rotate a secret (create new version) |
| `GET` | `/api/secrets/{secret_id}/versions` | Get secret version history |
| `GET` | `/api/secrets/rotation/needed` | Get secrets needing rotation |

### Secret Types

```rust
pub enum SecretType {
    ApiKey,              // API key or access token
    DatabasePassword,    // Database password
    PrivateKey,          // RSA, Ed25519, etc.
    OAuthCredentials,    // OAuth client ID/secret
    Generic,             // Generic secret
}
```

## Security Best Practices

### Encryption

The current implementation uses XOR cipher for demonstration. **For production, use proper encryption:**

```rust
// TODO: Replace with AES-256-GCM or similar
use aes_gcm::{Aes256Gcm, Key, Nonce};
use aes_gcm::aead::{Aead, NewAead};

// Production-grade encryption
let key = Key::from_slice(encryption_key);
let cipher = Aes256Gcm::new(key);
let nonce = Nonce::from_slice(b"unique nonce");
let ciphertext = cipher.encrypt(nonce, plaintext.as_ref())?;
```

### Key Management

Store encryption keys securely:

1. **Environment Variables**: `SHIIOO_SECRET_ENCRYPTION_KEY`
2. **Key Management Service**: AWS KMS, Azure Key Vault, HashiCorp Vault
3. **Hardware Security Module (HSM)**: For highest security

### Access Control

Implement strict access controls for secret endpoints:

```rust
// Require admin role for secret management
#[axum::middleware(require_admin_role)]
async fn create_secret(...) { ... }

// Audit all secret access
tracing::warn!(
    secret_id = %secret_id.0,
    user = %user_id,
    "Secret value accessed"
);
```

### Rotation Automation

Automate secret rotation with background tasks:

```rust
// Check for secrets needing rotation every hour
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(3600));

    loop {
        interval.tick().await;

        let secrets = secret_manager.get_secrets_needing_rotation();
        for secret in secrets {
            // Notify administrators
            notify_rotation_needed(&secret).await;
        }
    }
});
```

## Advanced Patterns in Practice

### Dynamic Approval Workflow

Combine conditional branches with dynamic DAG:

```json
{
  "name": "PR Review Workflow",
  "steps": [
    {
      "id": "analyze_pr",
      "pattern": {
        "type": "DynamicDAG",
        "generator_step": {
          "id": "determine_reviewers",
          "action": {
            "type": "agent_task",
            "prompt": "Analyze PR and determine required reviewers based on files changed"
          }
        }
      }
    },
    {
      "id": "approval_gate",
      "pattern": {
        "type": "ConditionalBranch",
        "condition": "all_approvals_received",
        "if_steps": [{"id": "merge", "action": {"type": "agent_task", "prompt": "Merge PR"}}],
        "else_steps": [{"id": "request_changes", "action": {"type": "agent_task", "prompt": "Request changes"}}]
      }
    }
  ]
}
```

## Next Steps

- See [README.md](README.md) for overall system architecture
- See [PHASE7_EXAMPLES.md](PHASE7_EXAMPLES.md) for multi-tenancy and HA
- Review core types in `crates/core/src/types.rs`
- Explore advanced workflow patterns in `crates/core/src/workflow/advanced.rs`
