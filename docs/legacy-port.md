# Porting from shiioo-legacy

[shiioo-legacy](https://github.com/raskell-io/shiioo-legacy) (BUSL-1.1, about 44k lines) has strong domain logic on top of a single-node storage and execution core. Shiioo keeps the domain logic and replaces the core. Code is ported only after legacy is relicensed to Apache-2.0, or by its copyright holders.

## Replace (core does not meet the new guarantees)

| Legacy | Issue | Replacement |
|---|---|---|
| `storage/event_log.rs`: gzipped JSONL, buffered in memory | Events acted on can be lost on crash | `EventStore` with durable append before acting |
| `events.rs`: UUID + timestamp | No seq, epoch or hash link | `shiioo-core::Event` |
| `cluster.rs`: `ClusterManager`, `DistributedLock`, `LeaderElection` | In-process `HashMap`s, not shared across nodes | Store-level leases with fencing (Postgres) |
| `audit.rs`: hash chain in a `Vec` | Lost on restart | The chain lives in the event log itself, plus signed checkpoints |
| `storage/index.rs`: redb index | Single writer | Postgres. redb/SQLite stays as the embedded single-node backend |
| `llm_client.rs`: Anthropic only | — | Provider trait with several backends |

## Port (domain logic, mostly pure)

| Legacy module | Target |
|---|---|
| `agent/policy_engine.rs`, `policy.rs`, `agent/policies.rs` | `shiioo-policy`, re-expressed in Cedar where possible |
| `approval.rs` (approval boards, quorum) | `shiioo-governance` |
| `agent/budgets.rs` | `shiioo-governance`, enforced by the server |
| `config_change.rs` | `shiioo-governance` (change management for configuration an agent proposes) |
| `capacity.rs` (key and provider pooling, failover, cost) | `shiioo-providers` |
| `agent/skill_*`, `builtin_skills.rs` | `shiioo-skills` |
| `rbac.rs`, `tenant.rs`, `organization.rs` | `shiioo-auth` |
| `secrets.rs` | `shiioo-secrets` (envelope encryption) |
| `scheduler.rs` (cron) | scheduler role |
| `mcp` crate, `agent/mcp_manager.rs` | `shiioo-tools` |
| `agent/archetype*`, Chief of Staff, VISION/MISSION/STRATEGY flow | "Virtual company" template on top of the platform |

## Drop or defer

- GraphQL API: REST + SSE with OpenAPI is the single contract.
- `workflow/` DAG engine: re-evaluate once the agent loop exists. DAG steps can become a run type on the same log.
