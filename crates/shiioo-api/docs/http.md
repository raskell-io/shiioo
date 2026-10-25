# HTTP API

One bearer token, JSON bodies, problem+json errors, SSE for the live log.
Everything below is what `src/lib.rs` serves today; the broader API design
(projects, fan-out, webhooks, SDKs) is in the top-level
[API design](../../../docs/design/api.md) and marked planned there.

## Authentication

Every `/v1/*` route except `/v1/health` requires
`Authorization: Bearer <token>`. The token is compared in constant time. A
missing or wrong token gets `401` with a problem document. The dashboard
files (`/`, `/ui`, `/ui/app.js`, `/ui/app.css`) are public; the dashboard
itself carries the token in the URL fragment, see
[dashboard.md](dashboard.md).

Run ids in paths must be full UUIDs. A malformed id is `400`.

## Errors

```
HTTP/1.1 404 Not Found
Content-Type: application/problem+json

{"type":"https://shiioo.dev/problems/not-found","title":"Not Found","status":404,"detail":"…"}
```

| `ApiError` | Status | Slug | Detail shown? |
| --- | --- | --- | --- |
| `Unauthorized` | 401 | `unauthorized` | yes |
| `NotFound` | 404 | `not-found` | yes |
| `BadRequest` | 400 | `bad-request` | yes |
| `Conflict` | 409 | `conflict` | yes |
| `Failed` | 500 | `failed` | yes (for example git's stderr) |
| `Internal` | 500 | `internal` | no; logged on the server |

## Runs

### `POST /v1/runs` → `201 Created`, `RunView`

```json
{
  "task": "Fix the flaky test in ci.yml",
  "employee": "auto",
  "repo": "shiioo"
}
```

- `task` (required).
- `employee`: `"auto"` lets the capacity broker choose among the configured
  employees; a name picks that employee regardless of capacity. Setting it
  overrides `runtime` and `agent`. Unknown name → `400 no employee named …`;
  nobody has capacity → `409 every employee is out of capacity (…)`.
- `repo`: with `employee`, a configured repository label or an absolute
  path. Harness employees get a worktree and branch in it.
- `agent`: an `AgentSpec` (`model`, `system`, `tools`, `max_turns`); default
  is the server's default agent.
- `runtime`: `{"kind":"native"}` (default) or `{"kind":"harness", "harness":
  "claude-code" | "codex", …}` with the `HarnessSpec` fields described in
  [shiioo-runtime](../../shiioo-runtime/README.md).

`Idempotency-Key` header: a repeated key returns the first run with `200`
instead of creating another. The mapping is in memory only; it does not
survive a server restart *(durable keys planned)*.

The run is created and a worker is started for it at once (`spawn_drive`).

### `GET /v1/runs` → `200`, `[RunView]`

All runs, most recently updated first.

### `GET /v1/runs/{id}` → `200`, `RunView`

```json
{
  "id": "6f1d…",
  "task": "…",
  "next_seq": 14,
  "status": {"state": "waiting_approval", "approval_id": "…", "summary": "Bash: git push"},
  "updated_at": 1761300000000
}
```

`status` is the runtime's `RunStatus`, tagged by `state`: `open`,
`completed {output}`, `failed {reason}`, `needs_attention {reason}`,
`waiting_approval {approval_id, summary}`, `cancelled {reason}`.
`updated_at` is the last event's time in ms since the epoch.

### `GET /v1/runs/{id}/events?from=<seq>` → `200`, `[Event]`

The log from `seq` (default 0). Each event is the core `Event` as JSON:
`seq`, `epoch`, `recorded_at`, `prev_hash`, `hash`, `payload` (tagged by
`type`).

### `GET /v1/runs/{id}/events/stream` → `text/event-stream`

```
event: event
id: 7
data: {"seq":7,"epoch":1,"payload":{"type":"tool_intent",…},…}

```

- Starts at `?from=` or, when the request carries `Last-Event-ID`, at that
  id plus one (the header wins). Reconnecting clients therefore never miss or
  duplicate an event.
- The handler subscribes to the store's change notifications before its first
  read, then alternates reading the log and waiting; a 2 s re-read is the
  backstop for writes the notification did not carry.
- Keep-alive comments are sent while idle (axum's `KeepAlive::default()`).
- The stream ends after a terminal event (`run_completed`, `run_failed`,
  `run_cancelled`).

### `POST /v1/runs/{id}/resume` → `202 Accepted`

Starts a worker for an open run (status `open` or `waiting_approval`). Any
other status → `409 only open runs can be resumed`.

### `POST /v1/runs/{id}/cancel` → `202 Accepted`

Optional body `{"reason": "…"}`. Appends `CancelRequested { by: "owner",
reason }`; the worker notices at its next step (next harness line or next
native turn) and appends `RunCancelled`. A run that already closed → `409
the run has already finished`.

### `GET /v1/runs/{id}/diff` → `200`, `text/plain`

`git -C <repo> diff --stat -p HEAD...shiioo/<run>` for a harness run with a
repository. Output above 200 KiB is cut with `… truncated`. A run without a
repository → `404 this run has no repository`; a git failure → `500 failed`
with git's stderr as detail.

## Permissions and approvals

### `POST /v1/runs/{id}/permissions` → `200`, `PermissionOutcome`

Called by `shiioo hook pre-tool-use` from inside the harness, with the hook's
own token. Body:

```json
{"tool_name": "Bash", "tool_input": {"command": "git push"},
 "tool_use_id": "toolu_…", "session_id": "…"}
```

Response `{"decision": "allow" | "deny", "reason": "…", "approval_id": "…"
| null}`. The request blocks while the owner decides; the flow and the
timeout are in [approvals-and-capacity.md](approvals-and-capacity.md).

### `POST /v1/runs/{id}/hook/pre-tool-use` → `200`

The same decision as `permissions`, in the harness hook's own dialect: the
body is what Claude Code or Codex writes to the hook's stdin (`tool_name`,
`tool_input`, `tool_use_id`, `session_id`, …), the answer is what the hook
must print:

```json
{"hookSpecificOutput":{"hookEventName":"PreToolUse",
  "permissionDecision":"allow"|"deny","permissionDecisionReason":"…"}}
```

Used by sandboxed runs, whose hook is a `curl` call because the `shiioo`
binary is not inside the container. Waits like `permissions` does.

### `POST /v1/approvals/{id}` → `200`

```json
{"decision": "allow", "note": "fine, but squash first"}
```

`decision` is `allow` or `deny`; anything else → `400`. Response `{"run":
"<run id>", "approval_id": "…", "decision": "allow"}`. An id that matches no
open approval → `404`.

## Capacity

### `GET /v1/capacity` → `200`, `[ProviderCapacity]`

```json
[{"provider": "claude-code", "observed_at": 1761300000000,
  "windows": {"five_hour": {"utilization": 0.42, "resets_at": 1761310000, "status": "allowed"},
              "seven_day": {"utilization": 0.18, "resets_at": 1761800000, "status": "allowed"}}}]
```

Sorted by provider. Empty until a harness has reported a rate-limit event.

## Health and static files

- `GET /v1/health` → `{"status":"ok"}`.
- `GET /` and `GET /ui` → `index.html` (`text/html`); `GET /ui/app.js`
  (`text/javascript`); `GET /ui/app.css` (`text/css`).

## Event descriptions

`describe(&Event)` renders one line per payload type (`run created: <task>`,
`tool intent <tool> …`, `approval requested …`, and so on). The CLI's `log`,
the dashboard and the Telegram bot all use it, so the three views of a run
read the same.

See also: [API design](../../../docs/design/api.md),
[ADR-0004 REST and SSE](../../../docs/adr/0004-rest-and-sse-api.md).
