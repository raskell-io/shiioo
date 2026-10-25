# Deployment and operations

Shiioo ships as one binary, `shiioo`, that scales from a laptop to a cluster without changing the model: the same event log rules, the same roles, different storage and process layout.

Related: [architecture.md](../architecture.md) §4–5, [ADR-0003](../adr/0003-redb-default-postgres-for-clusters.md) (storage), [ADR-0006](../adr/0006-daemon-dials-out-over-websocket.md) (daemons), [security.md](security.md).

## 1. Modes

| | On-device (the office on one Mac) | Several machines | Cluster |
|---|---|---|---|
| Command | `shiioo up` (= `shiioo serve`), installed by `shiioo install` | `shiioo serve` + `shiioo node --server …` on the others | not a goal ([ADR-0010](../adr/0010-single-binary-server-and-nodes.md)) |
| Store | redb file | redb on the server | Postgres stays available behind a feature flag |
| Phone | Telegram + the dashboard over Tailscale | same | |
| Status | built | nodes planned | deferred |

### 1.1 On-device: `shiioo up` *(built)*

One process runs everything: the API and dashboard, the Telegram bot, the workers, and the local machine that runs harnesses and tools.

- **The office directory** is explicit: `shiioo --workspace ~/shiioo-office …`. Shiioo keeps its data in `.shiioo/` inside it:

  | File | What |
  |---|---|
  | `shiioo.redb` | the event log and run heads, the only source of truth |
  | `journal.jsonl` | the local machine's tool-call journal |
  | `worktrees/<run>/` | one git worktree per harness run on a repository |
  | `config.toml` | repositories, Telegram owner id, dashboard URL (written with comments on first start) |
  | `rules.toml` | what agents may do without asking (written with defaults on first start) |
  | `api-token` | bearer token for the API and dashboard, mode 0600 |
  | `telegram-token` | the bot token, written by `shiioo install`, mode 0600 (or use `SHIIOO_TELEGRAM_TOKEN`) |
  | `logs/serve.log` | the service's output |

  Back up the directory and you have backed up the office; `shiioo.redb` alone is enough for the record.
- **The API binds to loopback** (`127.0.0.1:7878`) and every call needs the token. Nothing listens publicly.
- **Reaching it from the phone: Tailscale.** Install Tailscale on the Mac and the phone (same account). On the Mac, once: `tailscale serve --bg 7878`. The dashboard is then `https://<machine>.<tailnet>.ts.net/`, with TLS from Tailscale, reachable only from your devices. Open it once with `/#token=<api-token>` (the fragment never leaves the browser; the page stores the token locally) and add it to the home screen. Put the URL under `[dashboard] url` in `config.toml` so Telegram messages link to runs.
- **Telegram** needs no inbound connectivity at all: the bot long-polls Telegram's servers.
- **As a service:** `shiioo install` writes a launchd agent (`~/Library/LaunchAgents/io.raskell.shiioo.plist`) that starts the office at login, restarts it if it stops, runs it with the current `PATH` (so `claude` and `codex` are found) and `--keep-awake`, which ties a `caffeinate -s -i` to the process so the Mac doesn't sleep while the office runs. The agent runs in the user's GUI session, so Claude Code's keychain login works. `shiioo install --print` shows the definition; `shiioo uninstall` removes it. On Linux the same command writes a `systemd --user` unit (not yet tested).
- **Shutting down:** runs in progress lose their worker; on the next start `shiioo` resumes them from the log exactly as after a crash (harness sessions are resumed with `--resume`).

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
