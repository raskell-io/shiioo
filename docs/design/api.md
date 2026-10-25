# API

The public contract: REST resources, the event stream, idempotency, errors, auth, versioning, SDKs, the MCP server and outgoing webhooks.

Read [architecture.md](../architecture.md) first. Related: [ADR-0004](../adr/0004-rest-and-sse-api.md) (why REST + SSE), [execution.md](execution.md) (what commands do to a run), [governance.md](governance.md) (approvals), [security.md](security.md) (auth details), [ADR-0005](../adr/0005-cedar-policies-and-biscuit-credentials.md).

## 1. Principles *(planned)*

- **One contract.** REST + JSON under `/v1`, plus Server-Sent Events for streams. No GraphQL, no second RPC API for clients ([ADR-0004](../adr/0004-rest-and-sse-api.md)).
- **Generated from code.** The OpenAPI 3.1 document is generated from the handler types with `utoipa` and published at `/v1/openapi.json`. CI fails if it changes without a version-policy check (section 7).
- **Commands write events.** Every mutating call on a run (`messages`, `cancel`, approval decisions) is turned into an event in the run's log by the API role, then a worker reacts. The response returns the event's `seq`, so clients can stream from exactly that point.
- **Internals never leak.** Leases, epochs, `SeqConflict` and `Fenced` are worker concerns. A client never sees them; the API retries internally or returns a domain error.

## 2. Resources *(planned)*

| Resource | Endpoints |
|---|---|
| Organizations | `GET /v1/orgs`, `GET /v1/orgs/{org}` |
| Projects | `GET/POST /v1/orgs/{org}/projects`, `GET/PATCH/DELETE /v1/projects/{project}` |
| Agents | `GET/POST /v1/projects/{project}/agents`, `GET/PATCH /v1/agents/{agent}`, `GET /v1/agents/{agent}/versions` |
| Runs | `GET/POST /v1/projects/{project}/runs`, `GET /v1/runs/{run}` |
| Run events | `GET /v1/runs/{run}/events?from={seq}&limit=…`, `GET /v1/runs/{run}/events/stream` |
| Run commands | `POST /v1/runs/{run}/messages`, `POST /v1/runs/{run}/cancel` *(built)*, `POST /v1/runs/{run}/resume` *(built)*, `POST /v1/runs/{run}/fork` |
| Dashboard | `GET /` (and `/ui`, `/ui/app.js`, `/ui/app.css`) *(built)*: the mobile web dashboard, static, no auth on the page itself; it stores the token from `#token=…` locally and sends it on every API call |
| Capacity | `GET /v1/capacity` *(built)*: the latest rate-limit windows per provider (`five_hour`, `seven_day`: utilization, reset time), folded from `CapacityObserved` events; the broker and the phone read it |
| Diff | `GET /v1/runs/{run}/diff` *(built)*: `git diff HEAD...shiioo/<run>` of a repo run, text/plain |
| Employees | `POST /v1/runs` with `employee: "auto"` (the broker picks by capacity) or a configured name, plus `repo` (a label or path) *(built)*; the dashboard and Telegram use this |
| Permissions | `POST /v1/runs/{run}/permissions` *(built)*: a harness hook asks before a tool call; the rules answer, or the request waits for `POST /v1/approvals/{id}` *(built)* with `{"decision":"allow"\|"deny","note"}` |
| Run verification | `GET /v1/runs/{run}/verify` (re-checks the hash chain server-side, returns the head hash) |
| Approvals | `GET /v1/projects/{project}/approvals?state=pending`, `GET /v1/approvals/{id}`, `POST /v1/approvals/{id}/decisions` |
| Machines | `GET /v1/projects/{project}/machines`, `POST /v1/projects/{project}/machines/enrollments`, `DELETE /v1/machines/{machine}` |
| Secrets | `GET/POST /v1/projects/{project}/secrets` (names and metadata only; values are write-only), `PUT/DELETE /v1/secrets/{secret}` |
| Policies | `GET/POST /v1/projects/{project}/policies`, `GET/PUT /v1/policies/{policy}`, `POST /v1/policies/{policy}/test` |
| Budgets | `GET/PUT /v1/projects/{project}/budgets`, `GET /v1/projects/{project}/usage` |
| Triggers | `GET/POST /v1/projects/{project}/triggers`, `PATCH/DELETE /v1/triggers/{trigger}` |
| Webhooks (outgoing) | `GET/POST /v1/projects/{project}/webhooks`, `DELETE /v1/webhooks/{id}` |
| API keys | `GET/POST /v1/projects/{project}/keys`, `DELETE /v1/keys/{id}` |

IDs are prefixed, opaque strings (`run_01J…`, `agt_…`, `prj_…`) using ULIDs, so they sort by creation time and say what they are in logs.

### Starting a run

```http
POST /v1/projects/prj_01J9…/runs
Idempotency-Key: 4f1c2e7a-…
Content-Type: application/json

{
  "agent": "reviewer",
  "input": { "text": "Review PR #482 in raskell-io/shiioo" },
  "machine": { "labels": { "pool": "ci" } },
  "budget": { "max_usd": 1.5 }
}
```

```http
HTTP/1.1 201 Created
Location: /v1/runs/run_01J9…

{ "id": "run_01J9…", "state": "queued", "agent_version": 7, "head_seq": 0,
  "stream": "/v1/runs/run_01J9…/events/stream" }
```

### Sending a message to a waiting run

```http
POST /v1/runs/run_01J9…/messages
Idempotency-Key: 9b0d…

{ "text": "Also check the migration." }
```

```json
{ "seq": 41 }
```

### Forking

```http
POST /v1/runs/run_01J9…/fork
{ "at_seq": 42, "changes": { "model": "gpt-5", "instructions_append": "Be stricter." } }
```

`at_seq` must be a turn boundary. Returns the new run. Semantics in [fork-and-replay.md](fork-and-replay.md).

## 3. Streaming *(run stream built in `shiioo-api`; deltas, project stream and fan-out planned)*

### 3.1 Run stream

`GET /v1/runs/{run}/events/stream` returns `text/event-stream`:

```
id: 41
event: event
data: {"seq":41,"type":"user_message","recorded_at":"…","payload":{…},"hash":"…"}

event: delta
data: {"kind":"text","turn":5,"text":"Looking at the migr"}

event: delta
data: {"kind":"tool_output","call_id":"c_12","stream":"stdout","chunk":"Compiling shiioo-core…\n"}

: keep-alive
```

- **`event: event`** frames are log events. Their SSE `id` is the event's `seq`. On reconnect the browser (or SDK) sends `Last-Event-ID: 41` and the server replays everything after 41 from the store, then switches to live. Clients never miss or duplicate a log event.
- **`event: delta`** frames are ephemeral: model tokens and tool output as they happen. They have no `id`, are not stored, and are not replayed on reconnect; the final `model_response_recorded` and `tool_result` events carry the complete content.
- `?from={seq}` works like `Last-Event-ID` for clients that can't set headers.
- A comment line every 15 s keeps proxies from closing idle connections.
- The stream ends with `event: end` after a terminal event.

**Built today** (`crates/shiioo-api`): `GET /v1/runs/{id}/events/stream` sends each log event as `event: event` with `id: <seq>` and the event JSON as data, resumes after `Last-Event-ID` (or from `?from=`), and ends after the terminal event. The handler subscribes to change notifications before its first read, then alternates reading the log and waiting; the `Notifying` store wrapper wakes it on every write in the same process, and a 2-second re-read covers writes from other processes until Postgres `LISTEN/NOTIFY` is wired in. No `delta` frames yet: the provider is not streaming. Also built: `POST /v1/runs` (with `Idempotency-Key`, kept in memory for now), `GET /v1/runs` (most recently updated first), `GET /v1/runs/{id}`, `GET /v1/runs/{id}/events?from=`, `POST /v1/runs/{id}/resume`, `POST /v1/runs/{id}/cancel`, the permission and approval endpoints, bearer-token auth and problem+json errors.

**The approval loop** *(built)*: the worker starts Claude Code with a `PreToolUse` hook (`shiioo hook pre-tool-use`, see [runtimes-and-providers.md](runtimes-and-providers.md)). Before every tool call the hook `POST`s the call to `/v1/runs/{run}/permissions`. The server evaluates the owner's rules ([governance.md](governance.md)) and records a `PolicyDecision`; `allow`/`deny` answer at once. `ask` records an `ApprovalRequested`, notifies the owner (Telegram), and holds the HTTP request open until `POST /v1/approvals/{id}` records an `ApprovalDecided`, or the request expires (just under the hook's six-hour timeout) and is denied as `by: timeout`. The hook prints Claude Code's `permissionDecision` and the call proceeds or is refused. If the server is unreachable the hook denies, so a dead office never means an unguarded agent.

### 3.2 Project stream

`GET /v1/projects/{project}/events/stream` carries summary notifications (`run_started`, `run_finished`, `approval_requested`, `machine_offline`) for dashboards. It is resumable by a project-level cursor, not by run `seq`.

### 3.3 Fan-out

Streams are served by the gateway role. On a single node, new events are pushed in-process. In a cluster, gateways subscribe to Postgres `LISTEN/NOTIFY` for run heads they're streaming and read the new events from the store; NATS is an optional replacement if that becomes the bottleneck (see [deployment.md](deployment.md)).

## 4. Idempotency *(planned)*

- Every `POST` that creates or commands accepts an `Idempotency-Key` header (any string up to 255 bytes; SDKs send a UUID by default).
- The server stores `(key, principal, endpoint) → (request digest, response)` for 24 hours. A repeat with the same key and body returns the stored response; the same key with a different body returns `409` with type `idempotency-key-reused`.
- Run commands also carry the key into the event (`UserMessage { idempotency_key, … }`), so a retry that races the first attempt can't append twice even across API instances.

## 5. Pagination, filtering, errors *(planned)*

- Lists use cursor pagination: `?limit=50&cursor=…`, response `{ "items": […], "next_cursor": "…" }`. Cursors are opaque and stable under inserts.
- Event reads use `?from={seq}` instead, since `seq` is already a perfect cursor.
- Errors are RFC 9457 `application/problem+json` with stable `type` URIs:

```json
{
  "type": "https://shiioo.dev/problems/run-closed",
  "title": "Run has finished",
  "status": 409,
  "detail": "Run run_01J9… completed at seq 88; it no longer accepts messages.",
  "instance": "/v1/runs/run_01J9…/messages"
}
```

| `type` | Status | When |
|---|---|---|
| `validation-failed` | 422 | Body doesn't match the schema; `errors` lists JSON pointers |
| `not-found` | 404 | Also used when the caller may not know the resource exists |
| `forbidden` | 403 | Authenticated but not allowed (includes the Cedar decision id) |
| `run-closed` | 409 | Command on a finished run |
| `idempotency-key-reused` | 409 | Same key, different body |
| `budget-exceeded` | 402 | Starting a run whose budget is already spent |
| `rate-limited` | 429 | With `Retry-After` |

## 6. Authentication *(planned)*

Details and threat model in [security.md](security.md).

| Principal | How it authenticates | Notes |
|---|---|---|
| Humans (dashboard) | OAuth2 / OIDC with the org's identity provider; session cookie, `SameSite=Strict` | SSO per organization |
| Humans (CLI) | OAuth2 device authorization flow (`shiioo login`) | Short-lived access token, refresh token in the OS keychain |
| Services | API keys: `Authorization: Bearer shk_…`, scoped to a project and a role, optionally an expiry | Shown once; stored as a hash |
| Agents | Biscuit tokens minted per run ([ADR-0005](../adr/0005-cedar-policies-and-biscuit-credentials.md)) | Attenuated for subagents; can only call the endpoints their run needs |
| Machines | mTLS client certificate or key-bound Biscuit | Only on the daemon endpoint; see [tools-and-machines.md](tools-and-machines.md) |

Authorization for every request goes through the same Cedar evaluation as tool calls, so "who may approve deploys" and "which agent may call `shell`" are one policy language.

**Rate limits** are per API key or user, per project: a token bucket on requests and a separate concurrency limit on open streams. Responses carry `RateLimit-*` headers (IETF draft fields).

## 7. Versioning *(planned)*

- `/v1` only gets **additive** changes: new endpoints, new optional fields, new enum values, new event types. Clients must ignore unknown fields and event types; the SDKs do.
- A breaking change ships as `/v2` for the affected resources, with `/v1` kept for at least 12 months and announced via the `Deprecation` and `Sunset` headers.
- **Event payload schemas are part of the API.** JSON Schemas for every event type are published at `/v1/schemas/events/{type}.json` and versioned by the event's `schema` field (see [event-log.md](event-log.md)).
- CI compares the generated OpenAPI with the last release and fails on breaking diffs inside `/v1`.

## 8. SDKs *(planned)*

- **TypeScript first**, then **Python**: generated from the OpenAPI document, plus hand-written helpers where generation is weak:
  - a resumable stream reader that handles `Last-Event-ID`, backoff and the event/delta split;
  - `run.wait()` that streams until a terminal event or an approval is needed;
  - automatic `Idempotency-Key` on commands, reused across retries.
- **Rust**: the server's own request and response types, published as `shiioo-api-types`, with a thin `reqwest` client.

```ts
const run = await shiioo.runs.create(projectId, { agent: "reviewer", input: { text } });
for await (const ev of shiioo.runs.stream(run.id)) {
  if (ev.type === "approval_requested") notify(ev);
  if (ev.terminal) break;
}
```

## 9. MCP server *(planned)*

Shiioo exposes itself as an MCP server at `/mcp` (streamable HTTP, OAuth), so Claude Code, Codex or any MCP client can drive it:

| MCP tool | Does |
|---|---|
| `start_run` | Start a run of an agent with input; returns the run id |
| `get_run` | State, last events, final output |
| `send_message` | Message to a waiting run |
| `list_approvals` / `decide_approval` | See and answer pending approvals (as the authenticated human) |
| `fork_run` | Fork at a turn with changes |

The MCP tools are thin wrappers over the same REST handlers and the same authorization.

## 10. Outgoing webhooks *(planned)*

- Projects subscribe URLs to event kinds: `run.finished`, `run.failed`, `approval.requested`, `approval.decided`, `machine.offline`, `budget.threshold`.
- Deliveries are signed with HMAC-SHA256 over `timestamp.body` (`Shiioo-Signature: t=…,v1=…`), with a per-webhook secret, and rejected by receivers if older than 5 minutes.
- At-least-once with exponential backoff for 24 hours; each delivery carries a stable `id` for deduplication. Delivery attempts are visible in the API.
- Webhook payloads link back to the run and include the event `seq`, so the receiver can fetch the authoritative events.

## 11. Open questions

- Public domain for problem `type` URIs (`shiioo.dev` is a placeholder).
- Whether the project stream should be filtered by Cedar per subscriber or only coarse-grained by role.
