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
  "updated_at": 1761300000000,
  "created_at": 1761299000000,
  "runtime": "claude-code",
  "repo": "sango",
  "branch": "shiioo/6f1d2a3b"
}
```

`status` is the runtime's `RunStatus`, tagged by `state`: `open`,
`completed {output}`, `failed {reason}`, `needs_attention {reason}`,
`waiting_approval {approval_id, summary}`, `parked {waiting_for}`,
`cancelled {reason}`. `updated_at` and `created_at` are the last and first
event's times in ms since the epoch. `runtime` is `claude-code`, `codex`
or the native model; `repo` (a harness run's repository label, else its
directory name) and `branch` (a follow-up's is the one it continues) are
left out for runs without a repository. Also optional: `worker`, `machine`,
and `origin` for a follow-up.

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

### `GET /v1/events/stream` → `text/event-stream`

The office-wide stream, for the dashboard's run list and any client that
shows every run.

```
event: run
id: 1793350706007
data: {"id":"3bc1…","task":"…","next_seq":2,"status":{"state":"open"},"updated_at":1793350706007,…}

event: ready
data: {}

```

- First a pass over every run (oldest change first), closed by one `ready`
  frame; then a `run` frame whenever a run is created or its log grows.
  Each `run` frame carries the run's whole `RunView`, so a client keeps the
  latest per `id`; plans, routines and intake runs are runs and appear here.
- The frame id is the view's `updated_at`. With `Last-Event-ID` the first
  pass only sends runs updated at or after it: a reconnect can repeat a run,
  never miss one. A non-numeric id is a `400`.
- Writes are announced by `Notifying::subscribe_all` (a broadcast of run
  ids); bursts are gathered for 100 ms so a streaming harness turn sends a
  run once, not once per event. A subscriber that lags rescans every run,
  and a 10 s rescan covers writes from other processes.
- Keep-alive comments while idle; the stream does not end on its own.

### `POST /v1/runs/{id}/resume` → `202 Accepted`

Starts a worker for an open run (status `open` or `waiting_approval`). Any
other status → `409 only open runs can be resumed`.

### `POST /v1/runs/{id}/cancel` → `202 Accepted`

Optional body `{"reason": "…"}`. Appends `CancelRequested { by: "owner",
reason }`; the worker notices at its next step (next harness line or next
native turn) and appends `RunCancelled`. A run that already closed → `409
the run has already finished`.

### `POST /v1/runs/{id}/messages` → `202 Accepted`, `{"seq": n}`

The owner writes to a run: `{"text": "…"}` is appended as `UserMessage {
author: "owner" }` without a lease (`ApiState::send_message`), and `seq` is
where it landed.

- A parked run becomes runnable and is driven (or, if it is addressed to a
  machine, that node is woken); a run being worked on reads the message at
  its next turn, and answers it before completing or parking (see
  `shiioo-runtime/docs/worker-loop.md`). An open run that nobody drives is
  picked up.
- With `Idempotency-Key`, a repeat for the same run returns the first
  message's `seq` with `200` and appends nothing. Sends are serialized
  while the key is checked and the message appended, so a retry racing the
  first attempt waits for it. The keys are kept in memory.
- Claude Code and Codex read the message when their process finishes:
  instead of completing, the worker resumes the session with the message
  as the prompt (`shiioo-runtime/docs/worker-loop.md`, section 4). The
  "Now" option that interrupts the process is desk T14.
- `400` for an empty text; `404` for an unknown run; `409` for a finished
  run.
- When a drive ends, the office re-reads the run: a message that arrived
  while the drive was ending (and so did not start another) wakes it then.

### `POST /v1/runs/{id}/follow-up` → `201 Created`, the new run's `RunView`

Continues a finished Claude Code or Codex run (`ApiState::follow_up`): a
new run whose spec is the original's with `task` set to `{"text": "…"}`
and `origin: { run, seq, workspace, session }`. It works in the worktree
and branch of `workspace` (the first run of the chain, so a follow-up of a
follow-up stays on one branch) and its harness resumes `session` (the last
one the chain recorded) with the text as the prompt. The original stays
closed; the new run's view carries `origin` (the run it continues).

- With `Idempotency-Key`, a repeat for the same run returns the run the
  first one started with `200`. The keys are kept in memory.
- `400` for an empty text; `404` for an unknown run; `409` while the run is
  open (send it a message instead), for a native run (it has no session to
  resume), and while another run still works on the same branch.
- `diff` and the artifact routes of a follow-up read the shared branch and
  worktree.

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

## Deliverables

### `GET /v1/runs/{id}/artifacts` → `200`, `[Artifact]`

The files the run produced, for a run the office drove on a repository:
the branch's changes against the repository's HEAD (`added`, `modified`,
`deleted`) plus `untracked` files in the worktree, each with `size`, `kind`
(`image`, `audio`, `video`, `pdf`, `archive`, `text`, `other`) and
`content_type`. A run addressed to a machine answers `409`: its files are on
the node.

### `GET /v1/runs/{id}/artifacts/{path}` → `200`, the file

From the worktree while it exists, else from the branch. `Content-Type` by
extension, `Content-Disposition: attachment`. Paths with `..` are refused;
files over 50 MB answer `409`.

## Mail

### `POST /v1/mail/send` → `200`, `MailOutcome`

```json
{ "run": "<run id>" | null, "to": ["a@example.com"], "cc": [], "subject": "…", "body": "…" }
```

With `run`, the request is judged like a tool call named `mail.send` (input
`{to, cc, subject, description}`): the rules answer, or the owner is asked
and the request waits; a deny is `403` with the reason. Allowed, the office
sends through its account and appends `MailSent { to, subject, message_id }`
to the run. Without `run` the owner's request is sent as is. `409` when the
office has no mail account.

## Machines

### `POST /v1/machines/enrollments` → `201`

`{"labels": {"gpu": "true"}}` (optional) → `{"token": "…", "expires_in_secs": 3600}`.
A single-use token for `shiioo node enroll`; only its hash is kept.

### `POST /v1/machines/enroll` → `201` (no bearer token)

`{"token", "name", "labels", "capabilities", "version"}` →
`{"name", "machine_token"}`. The enrollment token is the credential. `400`
for an unknown or spent token or a bad name.

### `GET /v1/machines` → `200`

`{"machines": [{name, labels, capabilities, version, online, last_seen,
in_flight, created_at}], "text": "<the /machines text>"}`.

### `DELETE /v1/machines/{name}` → `204`

Forgets the machine; its token stops working.

### `POST /v1/machines/{name}/heartbeat` → `200`

`{"version", "capabilities", "in_flight": ["<run id>"]}`, from the node.

### `GET /v1/machines/{name}/work?wait=<secs>` → `200`

`{"runs": ["<run id>"]}`: open runs addressed to the machine by name or by
one of its labels. Waits up to `wait` seconds (at most 50) for one to appear.

### Node store: `POST /v1/runs/{id}/lease`, `POST /v1/runs/{id}/lease/renew`, `POST /v1/runs/{id}/events`

The store trait for node workers: `{"worker", "ttl_ms"}` → `Lease`;
`{"lease", "ttl_ms"}` → `Lease`; `{"lease", "expected_next", "payloads"}` →
`[Event]`. A store error answers `409` (or `404` for an unknown run) with
`{"type": "https://shiioo.dev/problems/store", "store": {"kind": "fenced" |
"lease_expired" | "lease_held" | "seq_conflict" | "run_closed" | …}}`, which
`StoreError::from_json` turns back into the error. A terminal event in an
append ends the run for the office: plans are read out, the owner is told.

## The company: report, roles, routines, plans

### `GET /v1/report` → `200`, `text/plain`

The morning report as the phone gets it: runs finished, failed and needing
the owner in the last 24 hours, approvals waiting, runs still going,
`shiioo/*` branches with unmerged commits per repository, capacity, the
next routines.

### `GET /v1/roles` → `200`, `[Role]`

The configured `[[roles]]`.

### `GET /v1/routines` → `200`

`{"routines": [Routine], "report": "<schedule>" | null, "summary": "<text>"}`:
the configured `[[routines]]`, the report schedule, and the one-line-per-
routine text with next firings that `/routines` shows.

### `POST /v1/routines/{name}/run` → `202 Accepted`

Fires a routine now. `{"run": "<run id>", "message": "⏰ routine … started run …"}`.
`404` for an unknown name.

### `POST /v1/plans` → `201 Created`

```json
{ "repo": "shiioo", "goal": "Ship v0.3", "role": null }
```

Starts the chief of staff: a harness run on `repo` (a configured label)
with read-only tools and the planning instructions. `{"plan": "<id>", "run":
"<id>"}`: the plan is referred to by the run's id or a 4+ character prefix.
When the run completes, the office parses its answer into the plan and
notifies the owner (`Notifications::message`), instead of the run's usual
ending message.

### `GET /v1/plans` → `200`, `[Plan]`; `GET /v1/plans/{id}` → `200`, `Plan`

```json
{ "run_id": "…", "repo": "shiioo", "goal": "Ship v0.3", "created_at": 1761400000000,
  "notes": "…", "items": [{ "title": "…", "body": "…" }],
  "status": { "status": "planning" | "proposed" | "approved" | "failed", "issues": ["<url>"], "reason": "…" } }
```

### `POST /v1/plans/{id}/approve` → `200`

Creates one issue per item on the plan's repository, labelled for the
intake (`ApiOptions.intake_label`), in order; `{"message": "<text for the
owner>", "plan": Plan}`. `409` if the plan is not `proposed` or the office
has no issue creator; `500` with the count of issues created if one fails
midway (the plan is then `approved` with the issues that exist).

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
- `GET /` and `GET /ui` → `index.html` (`text/html`); `GET /ui/{path}` →
  one of the files in `UI_FILES` (`text/javascript` for `.js`, `text/css`
  for `.css`), `404` for anything else.

## Event descriptions

`describe(&Event)` renders one line per payload type (`run created: <task>`,
`tool intent <tool> …`, `approval requested …`, and so on). The CLI's `log`,
the dashboard and the Telegram bot all use it, so the three views of a run
read the same.

See also: [API design](../../../docs/design/api.md),
[ADR-0004 REST and SSE](../../../docs/adr/0004-rest-and-sse-api.md).
