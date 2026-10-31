# shiioo-api

The HTTP face of a Shiioo office: an axum router that creates runs, streams
their event logs over SSE, answers the harness permission hook, takes the
owner's approval decisions, and serves the dashboard (the desk). It also holds the
two pieces of in-memory coordination the server needs: pending approvals
waiting for an answer and the capacity tracker that decides which employee
takes the next task.

Everything durable still lives in the event log. The API appends the owner's
events with `append_external` (no lease needed) and the workers absorb them;
see [docs/approvals-and-capacity.md](docs/approvals-and-capacity.md).

## Position in the crate graph

```
shiioo-core ─┐
shiioo-store ─┼──►  shiioo-api  ──►  shiioo (binary)
shiioo-runtime┤
shiioo-rules ─┘
```

- `shiioo-store` for `Notifying` (wakes SSE streams on every write) and
  `EventStore`.
- `shiioo-runtime` for `Worker`, `RunSpec`, `HarnessSpec`, `RunStatus`,
  `HookEndpoint`, `HOOK_TIMEOUT_SECS`.
- `shiioo-rules` for `RuleSet` and `PermissionRequest`, judged before the
  owner is asked.
- The binary builds `ApiState`, plugs in Telegram as `Notifications`, and
  serves the router. Nothing here knows about Telegram, launchd or config
  files.

## Module map

| Where | What |
| --- | --- |
| `src/lib.rs` | `router`, `ApiState`, `ApiOptions`, `Notifications`, `CapacityTracker`, `RunView`, permission/approval types, `ApiError`, the SSE `Feed`, `describe`. |
| `src/employees.rs` | `Employee`, `EmployeeKind`, `pick`, `default_employees`: the capacity broker. |
| `src/isolation.rs` | `IsolationPolicy`: the level a run gets (default, per repo, requested). |
| `src/roles.rs` | `Role`: a reusable agent configuration laid over an employee. |
| `src/schedule.rs` | `Schedule`: the owner's schedule words (`weekdays 07:00`, `every 6h`) and their next firing. |
| `src/routines.rs` | `Routine`, `RoutineState`, `due`: scheduled runs and what already fired. |
| `src/report.rs` | `ApiState::report`: the morning report, counted from the record and the repositories. |
| `src/plans.rs` | `Plan`, `Plans`, `IssueCreator`, the planner's task and plan parsing: the chief of staff. |
| `ui/` | The dashboard (the desk): `index.html`, `app.css`, native ES modules (`app.js`, `core.js`, `office.js`, `views/…`) and two fonts, embedded into the binary (`UI_FILES`, `UI_FONTS`) and served at `/` and `/ui/{path}`. See [docs/dashboard.md](docs/dashboard.md). |

## Public API

**Building a server**

- `ApiState::new(store, workers, token, default_agent)` or
  `ApiState::with_options(…, ApiOptions { rules, employees, capacity, repos,
  notifications })`. `ApiOptions::default()` has the default rule set, the
  default employees (`claude`, `codex`) and a fresh tracker.
- `router(Arc<ApiState<S>>) -> axum::Router`.
- `WorkerFactory<S>`: a closure the state calls to build a `Worker` for each
  drive, so the binary decides the machine, hooks and observer.
- `trait Notifications { approval_requested, approval_resolved, run_ended }`:
  implement it to be told what the owner should hear about.

**Doing things without HTTP** (the Telegram bot uses these)

- `start_run(spec)`, `assign(employee, repo)`, `cancel(run, by, reason)`,
  `decide(approval_id, allow, by, note, notify)`, `resolve_run(id_or_prefix)`
  (full UUID or a unique prefix of at least four characters), `run_view`,
  `list_runs`, `log_text(run, last)`, `diff(run)`, `capacity()`,
  `employees()`, `repos()`, `repo_path(label)`, `seed_capacity()`.
- `describe(&Event) -> String`: the one-line rendering of an event used by
  the CLI `log`, the dashboard and Telegram alike.

**Capacity**

- `CapacityTracker::observe(provider, windows, observed_at)` and
  `observe_event(&Event)` (feed it `CapacityObserved` events, newest wins),
  `snapshot()`, `provider(name)`, `text()`.

## Endpoints

Full request and response shapes are in [docs/http.md](docs/http.md).

| Method and path | Auth | Purpose |
| --- | --- | --- |
| `GET /v1/health` | no | `{"status":"ok"}` |
| `GET /`, `GET /ui`, `GET /ui/{path}` | no | dashboard files |
| `POST /v1/runs` | yes | create a run (`Idempotency-Key` honoured) |
| `GET /v1/runs` | yes | all runs, most recently updated first |
| `GET /v1/runs/{id}` | yes | one `RunView` |
| `GET /v1/runs/{id}/events?from=` | yes | the event log from a `seq` |
| `GET /v1/runs/{id}/events/stream` | yes | SSE, resumes from `Last-Event-ID` |
| `GET /v1/events/stream` | yes | SSE of every run's `RunView` as it changes |
| `POST /v1/runs/{id}/messages` | yes | the owner writes to a run (`Idempotency-Key` honoured) |
| `POST /v1/runs/{id}/follow-up` | yes | continue a finished harness run in a new run |
| `POST /v1/runs/{id}/resume` | yes | drive an open run again |
| `POST /v1/runs/{id}/cancel` | yes | ask the run to stop at its next step |
| `POST /v1/runs/{id}/permissions` | yes | the harness hook asks "may I?" |
| `GET /v1/runs/{id}/diff` | yes | `git diff` of the run's branch |
| `POST /v1/approvals/{id}` | yes | the owner answers allow or deny |
| `GET /v1/capacity` | yes | subscription windows per provider |
| `POST /v1/runs/{id}/hook/pre-tool-use` | yes | the hook in its own dialect, for sandboxes |
| `GET /v1/report` | yes | the morning report |
| `GET /v1/roles`, `GET /v1/routines`, `POST /v1/routines/{name}/run` | yes | roles, routines, fire one |
| `POST /v1/plans`, `GET /v1/plans`, `GET /v1/plans/{id}`, `POST /v1/plans/{id}/approve` | yes | the chief of staff |

Auth is one bearer token compared in constant time; `{id}` is a full run
UUID (prefix resolution is a library feature, not an HTTP one).

## Testing

```sh
cargo test -p shiioo-api
```

- `tests/api.rs`: a run streams to completion, a client resumes from
  `Last-Event-ID`, missing or wrong tokens get problem+json 401s, a stream
  opened before the events exist receives them live.
- `tests/permissions.rs`: rules answer at once, `ask` waits for the owner's
  decision through `/v1/approvals/{id}`, cancel stops the run at its next
  step.
- `tests/dashboard.rs`: dashboard files need no token, `UI_FILES` and
  `UI_FONTS` match `ui/` and every module import resolves, diff of a run
  without a repository is 404, diff shows the branch's changes after a fake
  harness commits on it.
- `tests/events.rs`: the office stream sends a snapshot, then live
  changes, and resumes from `Last-Event-ID`; it needs the token and a
  numeric id.
- `tests/messages.rs`: a message wakes a parked run once even when retried;
  empty, finished and unknown runs are refused.
- `tests/follow_up.rs`: a follow-up resumes the session and names the
  original, retries return the same run, chains stay on one branch, and
  follow-ups are refused for open, native and busy runs.
- `tests/company.rs`: roles, plans (proposed from a scripted planner, approved
  into issues through a fake creator), routines fired on demand and the report,
  over HTTP.
- `src/employees.rs` unit tests cover the broker's thresholds and TOML
  shape.

Tests use the memory store and a fake harness (a shell script run through
`/bin/sh`); none needs network access.

## Further reading

- [docs/http.md](docs/http.md): every endpoint, body, status code, the
  problem+json errors and the SSE frame format.
- [docs/approvals-and-capacity.md](docs/approvals-and-capacity.md): the
  permission flow from hook to Telegram and back, timeouts, the capacity
  tracker and the employee broker.
- [docs/dashboard.md](docs/dashboard.md): the embedded web UI, the desk.
- Top-level: [API design](../../docs/design/api.md),
  [governance design](../../docs/design/governance.md),
  [security design](../../docs/design/security.md),
  [ADR-0004 REST and SSE](../../docs/adr/0004-rest-and-sse-api.md),
  [architecture](../../docs/architecture.md).
- Run specs and statuses come from
  [shiioo-runtime](../shiioo-runtime/README.md).
