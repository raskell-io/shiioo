# Deployment and operations

Shiioo ships as one binary, `shiioo`, that scales from a laptop to a cluster without changing the model: the same event log rules, the same roles, different storage and process layout.

Related: [architecture.md](../architecture.md) §4–5, [ADR-0003](../adr/0003-redb-default-postgres-for-clusters.md) (storage), [ADR-0006](../adr/0006-daemon-dials-out-over-websocket.md) (daemons), [security.md](security.md).

## 1. Modes

| | On-device | Single server | Cluster |
|---|---|---|---|
| Command | `shiioo up` | `shiioo serve` | `shiioo serve --roles …` per process |
| Store | redb | redb or Postgres | Postgres (HA) |
| Blobs | local directory | local directory or object storage | object storage |
| Daemons | in-process, local machine | remote, dialing in | remote, dialing in |
| Auth | none: the local OS user is the owner | OIDC or local accounts, API keys | OIDC/SSO, API keys |
| TLS | localhost only | required | required, at the load balancer and for daemons |
| Status | planned | planned | planned |

### 1.1 On-device: `shiioo up` *(planned)*

One process runs every role plus a daemon for the local machine.

- **Data directory**: platform default, overridable with `SHIIOO_DATA_DIR`:
  - Linux: `$XDG_DATA_HOME/shiioo` (default `~/.local/share/shiioo`)
  - macOS: `~/Library/Application Support/shiioo`
  - Windows (later): `%APPDATA%\shiioo`
- Contents: `shiioo.redb` (event log, run heads, queue, config), `blobs/`, `snapshots/`, `secrets.age` (secrets encrypted to a locally generated `age` identity stored in the OS keychain).
- Dashboard and API on `http://127.0.0.1:7878`, bound to loopback only. No accounts; a random token in the data directory protects the API from other local users.
- The local machine is enrolled automatically at sandbox level `host` (or `process` on Linux if available).
- Shut down: runs in progress are parked at the next safe point; anything interrupted is recovered from the log on next start, exactly like a worker crash.

### 1.2 Single server: `shiioo serve` *(planned)*

- All roles in one process, redb for small teams or Postgres when you want standard database tooling (backups, replicas).
- Remote machines run `shiioo daemon --server https://shiioo.example.com --enroll <token>`.
- TLS terminated by Shiioo (rustls, ACME optional) or a reverse proxy.

### 1.3 Cluster *(planned)*

```
            load balancer (TLS)
        ┌────────┼──────────┬──────────────┐
      api ×N   gateway ×N  (sticky by connection)
        │        │
        │      scheduler ×2 (leader-elected via Postgres advisory lock)
        │        │
      worker ×N  │
        └────────┴───── Postgres (primary + replicas, PITR)
                        object storage (versioned bucket)
```

- **Roles**: `api`, `gateway`, `scheduler`, `worker`, selected with `--roles`. All are stateless; scale each independently.
- **Gateways** hold long-lived daemon and SSE connections. A daemon connects to any gateway; the gateway registers `machine → gateway` in Postgres so workers can route tool calls. If a gateway dies, daemons reconnect elsewhere and in-flight calls are resolved by the intent/result protocol.
- **Schedulers** fire timers and triggers. Two run for availability; one holds the leader lock. Correctness does not depend on the lock (runs are protected by leases), only efficiency.
- **Workers** pull runnable runs from the queue (`FOR UPDATE SKIP LOCKED`) and are woken by `LISTEN/NOTIFY`.
- **NATS** is optional later, if notification fan-out outgrows Postgres. It would carry wakeups only, never state.

## 2. Configuration *(planned)*

One TOML file, every key overridable by environment variables (`SHIIOO_` prefix, `__` for nesting).

```toml
[server]
listen = "0.0.0.0:7878"
public_url = "https://shiioo.example.com"
roles = ["api", "gateway", "scheduler", "worker"]

[store]
backend = "postgres"                  # or "redb"
url = "postgres://shiioo@db/shiioo"   # or path = "/var/lib/shiioo/shiioo.redb"

[blobs]
url = "s3://shiioo-prod-blobs"        # or "file:///var/lib/shiioo/blobs"

[auth]
oidc_issuer = "https://login.example.com"
oidc_client_id = "shiioo"

[secrets]
kms = "aws-kms:arn:aws:kms:…"         # or age_identity = "/etc/shiioo/age.key"

[worker]
concurrency = 64                      # runs one worker drives at once
lease_ttl = "30s"

[telemetry]
otlp_endpoint = "http://otel-collector:4317"
log_format = "json"

[gitops]
repo = "git@github.com:acme/shiioo-config.git"
branch = "main"
```

### GitOps

With `[gitops]` set, agent definitions, policies, boards, skills and triggers are read from a git repository. The server polls or receives a webhook, validates the new commit (Cedar validation, schema checks), and records the commit hash as the config version. Changes proposed by agents become pull requests ([governance.md](governance.md) §5). The dashboard shows config read-only in this mode.

## 3. Observability *(planned)*

### Tracing

`tracing` with an OpenTelemetry exporter. **The trace id is derived from the run id**, so one trace contains every turn of a run across workers, gateways and daemons. Spans: `turn`, `model.call`, `policy.eval`, `approval.wait`, `tool.dispatch`, `tool.exec` (on the daemon), `store.append`.

### Metrics (Prometheus, `/metrics`)

| Metric | Type | Labels |
|---|---|---|
| `shiioo_runs` | gauge | `state` (queued, running, parked, waiting_approval, completed, failed) |
| `shiioo_queue_depth` | gauge | |
| `shiioo_queue_wait_seconds` | histogram | |
| `shiioo_lease_takeovers_total` | counter | `reason` (expired, released) |
| `shiioo_fenced_writes_total` | counter | — should be near zero; spikes mean stalled workers |
| `shiioo_seq_conflicts_total` | counter | |
| `shiioo_turn_duration_seconds` | histogram | `agent` |
| `shiioo_model_tokens_total` | counter | `provider`, `model`, `direction` |
| `shiioo_model_cost_usd_total` | counter | `provider`, `model` |
| `shiioo_model_errors_total` | counter | `provider`, `kind` |
| `shiioo_tool_duration_seconds` | histogram | `tool`, `outcome` |
| `shiioo_tool_outcome_unknown_total` | counter | `tool` |
| `shiioo_approvals_pending` | gauge | `board` |
| `shiioo_daemons_connected` | gauge | `gateway` |
| `shiioo_store_append_seconds` | histogram | `backend` |

### Logs

Structured (JSON in servers, pretty in `shiioo up`), with `run_id`, `seq`, `epoch`, `agent`, `trace_id` fields. Logs are for operators; the event log is the record.

## 4. Upgrades *(planned)*

- **Rolling upgrades.** Roles are stateless, so processes are replaced one at a time. A worker that is shut down releases its leases at the next safe point; one that is killed is recovered from the log.
- **Database migrations are forward-only** and must be compatible with the previous release running against the new schema (expand, then contract in a later release). Migrations run at startup with a lock, or via `shiioo migrate`.
- **Event schema** changes add new payload versions; readers upcast old versions after verifying the chain. Events are never rewritten ([event-log.md](event-log.md)).
- **Daemon protocol** versions are negotiated at connect time. The server supports the current and previous protocol version, so machines can be upgraded after the server.
- **redb file format** upgrades follow redb's upgrade path; `shiioo up` takes a backup copy before upgrading the file.

## 5. Backup and restore *(planned)*

| Store | Backup | Restore check |
|---|---|---|
| Postgres | continuous archiving with point-in-time recovery (pgBackRest, WAL-G or managed PITR) | `shiioo verify` checks every run's chain and the latest anchored checkpoints |
| redb | `shiioo backup` takes a consistent copy inside a read transaction; or a filesystem snapshot when the process is stopped | same |
| Blobs | bucket versioning and lifecycle rules; local directory with any file backup | content addresses are verified on read |
| Secrets | KMS keys per the KMS's own backup; the `age` identity must be backed up separately | |

A restore to a point before the last external anchor will be detected by `shiioo verify` as a rollback. That is intended; the operator acknowledges it explicitly.

## 6. Distribution *(planned)*

- **Binary**: static builds for Linux (x86_64, aarch64, musl) and macOS (x86_64, aarch64), signed, with SBOM ([security.md](security.md) §3.10).
- **Install**: Homebrew tap, `cargo install shiioo`, a shell installer that verifies signatures, and a container image (`ghcr.io/raskell-io/shiioo`, distroless).
- **Self-hosting**: `docker compose up` with Postgres, the server and a MinIO bucket; a Helm chart later.
- **The dashboard** is a static TypeScript build embedded in the binary.

## 7. Sizing notes

These are starting points to validate with the simulator and benchmarks, not promises.

- A worker's cost is dominated by waiting on models and tools; one worker process can drive hundreds of concurrent runs.
- Parked and waiting runs cost storage only.
- redb serializes writes across runs; expect it to be sufficient for one machine's worth of agents. Move to Postgres when you need more than one worker process.
- The events table grows with every step. Partition by time and archive completed runs to object storage (Parquet plus the chain hashes) after a retention period ([event-log.md](event-log.md)).
