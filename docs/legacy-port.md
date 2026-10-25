# Porting from shiioo-legacy

[shiioo-legacy](https://github.com/raskell-io/shiioo-legacy) (BUSL-1.1, about 44k lines) has strong domain logic on top of a single-node storage and execution core. Shiioo keeps the domain logic and replaces the core. Code is ported only after legacy is relicensed to Apache-2.0, or by its copyright holders.

Status column: *built* exists in `crates/`; *planned* is on the roadmap ([positioning.md](positioning.md#roadmap)); *deferred* was set aside with the multi-owner, cluster shape ([ADR-0008](adr/0008-personal-workforce-not-managed-cloud.md), [ADR-0010](adr/0010-single-binary-server-and-nodes.md)). The legacy module names are the ones to read when the planned pieces are built.

## Replace (core does not meet the new guarantees)

| Legacy | Issue | Replacement | Status |
|---|---|---|---|
| `storage/event_log.rs`: gzipped JSONL, buffered in memory | Events acted on can be lost on crash | `EventStore` with durable append before acting (`shiioo-core`, `shiioo-store`) | built |
| `events.rs`: UUID + timestamp | No seq, epoch or hash link | `shiioo-core::Event` | built |
| `cluster.rs`: `ClusterManager`, `DistributedLock`, `LeaderElection` | In-process `HashMap`s, not shared across nodes | Store-level leases with fencing, in the store itself (redb; Postgres behind a flag) | built |
| `audit.rs`: hash chain in a `Vec` | Lost on restart | The chain lives in the event log itself (built), plus signed checkpoints (planned) | built / planned |
| `storage/index.rs`: redb index | Single writer | redb as the one store of the office, one server process ([ADR-0003](adr/0003-redb-default-postgres-for-clusters.md), [ADR-0010](adr/0010-single-binary-server-and-nodes.md)); Postgres for clusters deferred | built |
| `llm_client.rs`: Anthropic only | — | Harnesses first ([ADR-0009](adr/0009-harness-first-runtimes.md)): Claude Code and Codex drivers in `shiioo-runtime`; `shiioo-providers` for OpenAI-compatible and local models | built |

## Port (domain logic, mostly pure)

| Legacy module | Target | Status |
|---|---|---|
| `agent/policy_engine.rs`, `policy.rs`, `agent/policies.rs` | `shiioo-rules` (`rules.toml`: allow / ask / deny per tool, command, path, repo); Cedar re-expression deferred ([ADR-0008](adr/0008-personal-workforce-not-managed-cloud.md)) | built / deferred |
| `approval.rs` (approval boards, quorum) | the owner's approvals in `shiioo-api` (one approver, phone or dashboard); boards with quorum deferred | built / deferred |
| `agent/budgets.rs` | `max_turns`, `max_minutes`, `max_budget_usd` and per-employee capacity limits (`shiioo-runtime`, `shiioo-api`); hierarchical budgets planned | partly built |
| `config_change.rs` | change management for configuration an agent proposes ([governance.md §5](design/governance.md#5-change-management-planned)) | planned |
| `capacity.rs` (key and provider pooling, failover, cost) | `shiioo-api::capacity` and the `[[employees]]` broker (subscription windows, not keys); key pools deferred | built / deferred |
| `agent/skill_*`, `builtin_skills.rs` | skills in agent definitions ([runtimes-and-providers.md](design/runtimes-and-providers.md)) | planned |
| `rbac.rs`, `tenant.rs`, `organization.rs` | multi-owner auth | deferred, ADR-0008 |
| `secrets.rs` | secrets injection with envelope encryption ([security.md §3.2](design/security.md#32-secrets-never-reach-the-model-planned)) | planned, phase 4 |
| `scheduler.rs` (cron) | routines ([positioning.md](positioning.md#roadmap), phase 5) | planned |
| `mcp` crate, `agent/mcp_manager.rs` | MCP servers as tools ([tools-and-machines.md](design/tools-and-machines.md)) | planned |
| `agent/archetype*`, Chief of Staff, VISION/MISSION/STRATEGY flow | the planner and the "virtual company" template ([governance.md §6](design/governance.md#6-the-virtual-company-template-planned)) | planned, phase 5 |

## Drop or defer

- GraphQL API: REST + SSE is the single contract ([ADR-0004](adr/0004-rest-and-sse-api.md)); OpenAPI generation and SDKs are deferred with it ([ADR-0010](adr/0010-single-binary-server-and-nodes.md)).
- `workflow/` DAG engine: re-evaluate with the planner (phase 5). DAG steps can become a run type on the same log.
- Multi-tenant pieces (`tenant.rs`, `organization.rs`, `rbac.rs`): deferred with the managed-cloud shape ([ADR-0008](adr/0008-personal-workforce-not-managed-cloud.md)).
