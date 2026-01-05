# Phase 4: Capacity Broker Guide

This guide demonstrates how to use Shiioo's capacity broker for multi-source LLM capacity pooling, rate limit handling, priority queues, and cost tracking introduced in Phase 4.

## Overview

Phase 4 adds:
- **Multi-source capacity pooling** - Register multiple API keys/providers
- **Rate limit handling** - Automatic backoff and source selection
- **Priority queues** - Ensure high-priority workflows get capacity first
- **Cost tracking** - Track token usage and costs across all sources

## Capacity Source Management

### Register a Capacity Source

```bash
curl -X POST http://localhost:8080/api/capacity/sources \
  -H "Content-Type: application/json" \
  -d '{
    "id": "anthropic_primary",
    "name": "Anthropic Primary API Key",
    "provider": "anthropic",
    "api_key_hash": "sha256_hash_of_key",
    "model": "claude-opus-4",
    "rate_limits": {
      "requests_per_minute": 60,
      "tokens_per_minute": 100000,
      "tokens_per_day": 1000000
    },
    "cost_per_token": {
      "input_cost": 15.0,
      "output_cost": 75.0
    },
    "priority": 100,
    "enabled": true,
    "created_at": "2026-01-05T20:00:00Z",
    "updated_at": "2026-01-05T20:00:00Z"
  }'
```

### List All Capacity Sources

```bash
curl http://localhost:8080/api/capacity/sources
```

Response:
```json
{
  "sources": [
    {
      "id": "anthropic_primary",
      "name": "Anthropic Primary API Key",
      "provider": "anthropic",
      "model": "claude-opus-4",
      "rate_limits": {
        "requests_per_minute": 60,
        "tokens_per_minute": 100000,
        "tokens_per_day": 1000000
      },
      "cost_per_token": {
        "input_cost": 15.0,
        "output_cost": 75.0
      },
      "priority": 100,
      "enabled": true
    },
    {
      "id": "openai_backup",
      "name": "OpenAI Backup Key",
      "provider": "openai",
      "model": "gpt-4o",
      "rate_limits": {
        "requests_per_minute": 100,
        "tokens_per_minute": 150000,
        "tokens_per_day": null
      },
      "cost_per_token": {
        "input_cost": 5.0,
        "output_cost": 15.0
      },
      "priority": 50,
      "enabled": true
    }
  ]
}
```

### Get Specific Source

```bash
curl http://localhost:8080/api/capacity/sources/anthropic_primary
```

### Delete a Source

```bash
curl -X DELETE http://localhost:8080/api/capacity/sources/anthropic_primary
```

## Provider Types

Shiioo supports multiple LLM providers:

### Anthropic

```json
{
  "provider": "anthropic",
  "model": "claude-opus-4",
  "cost_per_token": {
    "input_cost": 15.0,
    "output_cost": 75.0
  }
}
```

### OpenAI

```json
{
  "provider": "openai",
  "model": "gpt-4o",
  "cost_per_token": {
    "input_cost": 5.0,
    "output_cost": 15.0
  }
}
```

### Azure OpenAI

```json
{
  "provider": "azure",
  "model": "gpt-4-turbo",
  "cost_per_token": {
    "input_cost": 10.0,
    "output_cost": 30.0
  }
}
```

### Custom Endpoint

```json
{
  "provider": {
    "custom": {
      "endpoint": "https://custom-llm.example.com/v1"
    }
  },
  "model": "custom-model",
  "cost_per_token": {
    "input_cost": 1.0,
    "output_cost": 2.0
  }
}
```

## Rate Limits and Priority

### How Source Selection Works

The capacity broker selects sources based on:
1. **Enabled status** - Only enabled sources are considered
2. **Priority** (0-255, higher = preferred) - Sources are sorted by priority
3. **Rate limits** - Sources that have exceeded their rate limits are skipped
4. **Backoff state** - Sources with active exponential backoff are skipped

Example with multiple sources:

```bash
# Register high-priority source
curl -X POST http://localhost:8080/api/capacity/sources \
  -H "Content-Type: application/json" \
  -d '{
    "id": "anthropic_enterprise",
    "name": "Anthropic Enterprise",
    "provider": "anthropic",
    "model": "claude-opus-4",
    "rate_limits": {
      "requests_per_minute": 1000,
      "tokens_per_minute": 1000000,
      "tokens_per_day": 10000000
    },
    "priority": 200,
    "enabled": true
  }'

# Register medium-priority source
curl -X POST http://localhost:8080/api/capacity/sources \
  -H "Content-Type: application/json" \
  -d '{
    "id": "anthropic_standard",
    "name": "Anthropic Standard",
    "provider": "anthropic",
    "model": "claude-sonnet-4",
    "rate_limits": {
      "requests_per_minute": 60,
      "tokens_per_minute": 100000,
      "tokens_per_day": 1000000
    },
    "priority": 100,
    "enabled": true
  }'

# Register low-priority fallback
curl -X POST http://localhost:8080/api/capacity/sources \
  -H "Content-Type: application/json" \
  -d '{
    "id": "openai_fallback",
    "name": "OpenAI Fallback",
    "provider": "openai",
    "model": "gpt-4o",
    "rate_limits": {
      "requests_per_minute": 100,
      "tokens_per_minute": 150000
    },
    "priority": 50,
    "enabled": true
  }'
```

### Rate Limit Enforcement

The broker tracks:
- **Requests per minute** - Rolling 60-second window
- **Tokens per minute** - Rolling 60-second window for input + output tokens
- **Tokens per day** - Daily limit with automatic reset at midnight UTC

When a source is rate-limited:
1. The broker applies exponential backoff (60s default, configurable via `retry_after`)
2. The source is skipped during backoff period
3. The broker automatically tries the next available source
4. If no sources are available, the request is queued with priority

## Cost Tracking

### View Usage Records

```bash
curl http://localhost:8080/api/capacity/usage
```

Response:
```json
{
  "usage": [
    {
      "id": "usage_123",
      "source_id": "anthropic_primary",
      "timestamp": "2026-01-05T20:30:00Z",
      "input_tokens": 1000,
      "output_tokens": 500,
      "total_tokens": 1500,
      "cost": 0.04875,
      "request_count": 1,
      "run_id": "550e8400-e29b-41d4-a716-446655440000",
      "step_id": "analyze"
    }
  ]
}
```

### Get Cost Summary

```bash
curl http://localhost:8080/api/capacity/cost
```

Response:
```json
{
  "total_cost": 12.45,
  "total_tokens": 250000,
  "total_requests": 150,
  "record_count": 150
}
```

## Priority Queue System

### How Priority Works

When all sources are rate-limited or unavailable, requests are queued with priority:

- **Priority 0-255** - Higher numbers = more urgent
- **Age** - Older requests are prioritized within the same priority level
- **FIFO within priority** - Requests with the same priority are processed in order

Example priorities:
- **200-255**: Critical production workflows
- **100-199**: Standard workflows
- **50-99**: Background/batch jobs
- **0-49**: Low-priority tasks

The priority is set when executing workflows. The capacity broker will process high-priority requests first when capacity becomes available.

## Cost Calculation

Costs are calculated per 1M tokens:

```
input_cost = (input_tokens * cost_per_token.input_cost) / 1,000,000
output_cost = (output_tokens * cost_per_token.output_cost) / 1,000,000
total_cost = input_cost + output_cost
```

Example:
- Input: 1,000 tokens @ $15/1M = $0.015
- Output: 500 tokens @ $75/1M = $0.0375
- **Total: $0.0525**

## Complete Example: Multi-Source Setup

```bash
# 1. Register primary Anthropic source (highest priority)
curl -X POST http://localhost:8080/api/capacity/sources \
  -H "Content-Type: application/json" \
  -d '{
    "id": "anthropic_primary",
    "name": "Anthropic Primary",
    "provider": "anthropic",
    "api_key_hash": "$(echo -n 'sk-ant-...' | sha256sum | cut -d\" \" -f1)",
    "model": "claude-opus-4",
    "rate_limits": {
      "requests_per_minute": 60,
      "tokens_per_minute": 100000,
      "tokens_per_day": 1000000
    },
    "cost_per_token": {
      "input_cost": 15.0,
      "output_cost": 75.0
    },
    "priority": 100,
    "enabled": true,
    "created_at": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'",
    "updated_at": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'"
  }'

# 2. Register backup OpenAI source (medium priority)
curl -X POST http://localhost:8080/api/capacity/sources \
  -H "Content-Type: application/json" \
  -d '{
    "id": "openai_backup",
    "name": "OpenAI Backup",
    "provider": "openai",
    "api_key_hash": "$(echo -n 'sk-...' | sha256sum | cut -d\" \" -f1)",
    "model": "gpt-4o",
    "rate_limits": {
      "requests_per_minute": 100,
      "tokens_per_minute": 150000
    },
    "cost_per_token": {
      "input_cost": 5.0,
      "output_cost": 15.0
    },
    "priority": 50,
    "enabled": true,
    "created_at": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'",
    "updated_at": "'$(date -u +%Y-%m-%dT%H:%M:%SZ)'"
  }'

# 3. List all sources
curl http://localhost:8080/api/capacity/sources | jq

# 4. Run a workflow (will use Anthropic primary if available)
curl -X POST http://localhost:8080/api/jobs \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Analyze Code",
    "workflow": {
      "steps": [
        {
          "id": "analyze",
          "name": "Code Analysis",
          "description": "Analyze the code for issues",
          "role": "engineer",
          "action": {
            "type": "agent_task",
            "prompt": "Analyze the code in src/main.rs and report any issues"
          },
          "timeout_secs": 120
        }
      ],
      "dependencies": {}
    },
    "execute": true
  }'

# 5. Check usage and costs
curl http://localhost:8080/api/capacity/usage | jq
curl http://localhost:8080/api/capacity/cost | jq
```

## Best Practices

1. **Register multiple sources** - Ensure redundancy across providers
2. **Set appropriate priorities** - Higher priority for expensive/powerful models
3. **Configure rate limits accurately** - Match your API plan limits
4. **Monitor costs regularly** - Use the cost API to track spending
5. **Use daily limits** - Prevent runaway costs with `tokens_per_day`
6. **Hash API keys** - Never store plaintext keys, use SHA-256 hashes
7. **Enable/disable sources** - Temporarily disable sources without deleting
8. **Test failover** - Verify backup sources work before you need them

## Programmatic Usage (Rust)

The `CapacityBroker` can be used programmatically:

```rust
use shiioo_core::capacity::CapacityBroker;
use shiioo_core::types::*;

// Create broker
let broker = CapacityBroker::new();

// Register sources
let source = CapacitySource {
    id: CapacitySourceId::new("anthropic_1"),
    name: "Anthropic Primary".to_string(),
    provider: LlmProvider::Anthropic,
    api_key_hash: "sha256_hash".to_string(),
    model: "claude-opus-4".to_string(),
    rate_limits: RateLimits {
        requests_per_minute: 60,
        tokens_per_minute: 100_000,
        tokens_per_day: Some(1_000_000),
    },
    cost_per_token: CostPerToken {
        input_cost: 15.0,
        output_cost: 75.0,
    },
    priority: 100,
    enabled: true,
    created_at: Utc::now(),
    updated_at: Utc::now(),
};

broker.register_source(source)?;

// Execute request
let request = LlmRequest {
    prompt: "Analyze this code".to_string(),
    max_tokens: 1000,
    temperature: Some(0.7),
    model: None,
};

let response = broker.execute_request(
    request,
    RunId::new(),
    StepId::new("analyze"),
    RoleId::new("engineer"),
    100, // priority
).await?;

// Track costs
let total_cost = broker.get_total_cost(Utc::now() - Duration::days(30));
println!("Total cost (last 30 days): ${:.2}", total_cost);
```

## Next Steps

See:
- [README.md](README.md) for overall architecture
- [POLICY_EXAMPLES.md](POLICY_EXAMPLES.md) for policy management
- [PHASE3_EXAMPLES.md](PHASE3_EXAMPLES.md) for organization management
