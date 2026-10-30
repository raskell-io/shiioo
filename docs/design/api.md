# API

The public contract: REST resources, the event stream, idempotency, errors, auth, versioning, SDKs, the MCP server and outgoing webhooks.

Built today (`crates/shiioo-api`): runs, events and the SSE stream, resume, cancel, the permission and approval endpoints, the diff, capacity, the dashboard, bearer-token auth and problem+json errors. Everything an owner needs from the phone goes through these. The rest of this document is the contract they grow into; sections marked *(deferred)* belong to the multi-owner shape set aside in [ADR-0008](../adr/0008-personal-workforce-not-managed-cloud.md) and [ADR-0010](../adr/0010-single-binary-server-and-nodes.md).

Read [architecture.md](../architecture.md) first. Related: [ADR-0004](../adr/0004-rest-and-sse-api.md) (why REST + SSE), [execution.md](execution.md) (what commands do to a run), [governance.md](governance.md) (approvals), [security.md](security.md) (auth details), [ADR-0005](../adr/0005-cedar-policies-and-biscuit-credentials.md).

## 1. Principles *(built, except where marked)*

- **One contract.** REST + JSON under `/v1`, plus Server-Sent Events for streams. No GraphQL, no second RPC API for clients ([ADR-0004](../adr/0004-rest-and-sse-api.md)). The Telegram bot and the dashboard are clients of the same handlers.
- **Generated from code.** *(deferred, ADR-0010)* The OpenAPI 3.1 document generated from the handler types with `utoipa` and published at `/v1/openapi.json`, with CI failing on an unchecked change (section 7). Today the routes are documented here and in the crate's docs.
- **Commands write events.** Every mutating call on a run (`cancel`, approval decisions; `messages` *(planned)*) is turned into an event in the run's log by the server (`RunHead::append_external`, no lease needed), then the worker reacts at its next step. *(planned)* The response returns the event's `seq`, so clients can stream from exactly that point; today `cancel` returns `202` with the run.
- **Internals never leak.** Leases, epochs, `SeqConflict` and `Fenced` are worker concerns. A client never sees them; the API retries internally or returns a domain error.

## 2. Resources *(core built; the rest planned or deferred)*

One office means no `{org}` or `{project}` segment in the built routes: `/v1/runs`, not `/v1/projects/{project}/runs`. The rows below keep the longer paths where they come from the multi-office design *(deferred, ADR-0008)*; if they are ever built for one office, the segment is dropped the same way.

| Resource | Endpoints |
|---|---|
| Health | `GET /v1/health` *(built)*: `{"status":"ok"}`, the one route without a token; the installer and scripts wait on it |
| Organizations | `GET /v1/orgs`, `GET /v1/orgs/{org}` *(deferred, ADR-0008)* |
| Projects | `GET/POST /v1/orgs/{org}/projects`, `GET/PATCH/DELETE /v1/projects/{project}` *(deferred, ADR-0008)* |
| Agents | `GET/POST /v1/projects/{project}/agents`, `GET/PATCH /v1/agents/{agent}`, `GET /v1/agents/{agent}/versions` *(planned; today the agent is inline in the run body or an `[[employees]]` entry)* |
| Runs | `POST /v1/runs` *(built)*, `GET /v1/runs` *(built; most recently updated first)*, `GET /v1/runs/{run}` *(built)* |
| Run events | `GET /v1/runs/{run}/events?from={seq}` *(built; `limit` planned)*, `GET /v1/runs/{run}/events/stream` *(built)* |
| Run commands | `POST /v1/runs/{run}/messages` *(planned)*, `POST /v1/runs/{run}/cancel` *(built)*, `POST /v1/runs/{run}/resume` *(built)*, `POST /v1/runs/{run}/fork` *(planned)* |
| Dashboard | `GET /` (and `/ui`, `/ui/app.js`, `/ui/app.css`) *(built)*: the mobile web dashboard, static, no auth on the page itself; it stores the token from `#token=…` locally and sends it on every API call |
| Capacity | `GET /v1/capacity` *(built)*: the latest rate-limit windows per provider (`five_hour`, `seven_day`: utilization, reset time), folded from `CapacityObserved` events; the broker and the phone read it |
| Diff | `GET /v1/runs/{run}/diff` *(built)*: `git diff HEAD...shiioo/<run>` of a repo run, text/plain |
| Employees | `POST /v1/runs` with `employee: "auto"` (the broker picks by capacity) or a configured name, plus `repo` (a label or path) *(built)*; the dashboard and Telegram use this |
| Permissions | `POST /v1/runs/{run}/permissions` *(built)*: a harness hook asks before a tool call; the rules answer, or the request waits for `POST /v1/approvals/{id}` *(built)* with `{"decision":"allow"\|"deny","note"}` |
| Run verification | `GET /v1/runs/{run}/verify` *(planned; `shiioo log` verifies the chain locally today)* |
| Approvals | `POST /v1/approvals/{id}` *(built, above)*; `GET /v1/approvals?state=pending`, `GET /v1/approvals/{id}` *(planned; the pending one is in the run's status today)* |
| Machines | `GET /v1/machines` *(built)*; `POST /v1/machines/enrollments` *(built)*: a one-time token; `POST /v1/machines/enroll` *(built, unauthenticated: the token is the credential)*; `DELETE /v1/machines/{name}` *(built)*; `POST /v1/machines/{name}/heartbeat`, `GET /v1/machines/{name}/work?wait=` *(built, for nodes)* |
| Node store | `POST /v1/runs/{run}/lease`, `POST /v1/runs/{run}/lease/renew`, `POST /v1/runs/{run}/events` *(built)*: the store trait over HTTP for node workers; store errors as `{"store": {"kind": …}}` ([ADR-0012](../adr/0012-nodes-are-workers-over-the-offices-api.md)) |
| Deliverables | `GET /v1/runs/{run}/artifacts` *(built)*: files the branch changed plus untracked files in the worktree, with kind and size; `GET /v1/runs/{run}/artifacts/{path}` *(built)*: the file |
| Mail | `POST /v1/mail/send` `{run?, to, cc?, subject, body}` *(built)*: with a run, judged as the tool `mail.send` by the rules (default: ask) and recorded as `MailSent`; without, the owner's own request |
| Secrets | `GET/POST /v1/secrets` (names and metadata only; values are write-only), `PUT/DELETE /v1/secrets/{secret}` *(planned)* |
| Rules | `GET/PUT /v1/rules`, `POST /v1/rules/test` *(planned; `rules.toml` is edited on the machine today)* |
| Budgets | `GET/PUT /v1/budgets`, `GET /v1/usage` *(planned)* |
| Report | `GET /v1/report` *(built)*: the morning report as text; what finished, failed or needs the owner in the last 24 hours, approvals waiting, branches awaiting review, capacity, next routines |
| Roles | `GET /v1/roles` *(built)*: the configured `[[roles]]`; `POST /v1/runs` with `role: "<name>"` runs as one. `PATCH`/`POST` *(planned; config.toml today)* |
| Routines | `GET /v1/routines` *(built)*: the configured `[[routines]]`, the report schedule and a summary with next firings; `POST /v1/routines/{name}/run` *(built)*: fire one now. Editing them over the API *(planned; config.toml today)*. These are the "triggers" of earlier drafts |
| Plans | `POST /v1/plans` `{repo, goal, role?}` *(built)*: starts the chief of staff; `GET /v1/plans`, `GET /v1/plans/{id}` *(built)*; `POST /v1/plans/{id}/approve` *(built)*: one labelled issue per item for the intake |
| Webhooks (outgoing) | `GET/POST /v1/webhooks`, `DELETE /v1/webhooks/{id}` *(planned)* |
| API keys | `GET/POST /v1/keys`, `DELETE /v1/keys/{id}` *(planned; one bearer token today)* |

Run ids are UUIDs today (`RunId`), shown by their first eight characters on the phone and in branch names (`shiioo/7f3a21c9`). *(planned)* Prefixed ULIDs (`run_01J…`, `agt_…`) that sort by creation time and say what they are in logs; the change is additive for clients that treat ids as opaque strings.

### Starting a run *(built)*

```http
POST /v1/runs
Idempotency-Key: 4f1c2e7a-…
Content-Type: application/json

{ "task": "Review PR #482", "employee": "auto", "repo": "shiioo" }
```

`employee` is `"auto"` (the broker picks the configured employee with the most capacity left, [runtimes-and-providers.md §5.4](runtimes-and-providers.md#54-capacity-broker-built-as-the-employee-broker-key-pools-deferred)) or an employee's name; `repo` is a label from `[repos]` or an absolute path. Without `employee`, the body names the runtime directly: `{ "task", "agent": { "model", "system", "tools", "max_turns" }, "runtime": { "kind": "native" } | { "kind": "harness", … } }`.

```http
HTTP/1.1 201 Created

{ "id": "7f3a21c9-…", "task": "Review PR #482", "next_seq": 2,
  "status": { "state": "open" }, "updated_at": 1761376000000 }
```

The same body is returned by `GET /v1/runs/{run}` and, one per run, by `GET /v1/runs`. `status` is one of `open`, `waiting_approval` (with the pending request), `completed` (with the final `output`), `failed`, `needs_attention`, `cancelled` (with a `reason`), folded from the log by `shiioo-runtime::status`. *(planned)* `Location` header, `machine` and `budget` fields, and a `stream` URL in the response.

### Sending a message to a waiting run *(planned)*

```http
POST /v1/runs/run_01J9…/messages
Idempotency-Key: 9b0d…

{ "text": "Also check the migration." }
```

```json
{ "seq": 41 }
```

### Forking *(planned)*

```http
POST /v1/runs/run_01J9…/fork
{ "at_seq": 42, "changes": { "model": "gpt-5", "instructions_append": "Be stricter." } }
```

`at_seq` must be a turn boundary. Returns the new run. Semantics in [fork-and-replay.md](fork-and-replay.md).

## 3. Streaming *(run and office streams built in `shiioo-api`; deltas planned, fan-out deferred)*

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

**Built today** (`crates/shiioo-api`): `GET /v1/runs/{id}/events/stream` sends each log event as `event: event` with `id: <seq>` and the event JSON as data, resumes after `Last-Event-ID` (or from `?from=`), and ends after the terminal event. The handler subscribes to change notifications before its first read, then alternates reading the log and waiting; the `Notifying` store wrapper wakes it on every write in the same process, and a 2-second re-read covers writes from other processes (Postgres `LISTEN/NOTIFY` for that is *(deferred, ADR-0010)*; in one server process the wrapper is enough). No `delta` frames yet: neither the native provider nor the harness drivers stream partial output. Also built: `POST /v1/runs` (with `Idempotency-Key`, kept in memory for now), `GET /v1/runs` (most recently updated first), `GET /v1/runs/{id}`, `GET /v1/runs/{id}/events?from=`, `POST /v1/runs/{id}/resume`, `POST /v1/runs/{id}/cancel`, the permission and approval endpoints, bearer-token auth and problem+json errors.

**The approval loop** *(built)*: the worker starts Claude Code with a `PreToolUse` hook (`shiioo hook pre-tool-use`, see [runtimes-and-providers.md](runtimes-and-providers.md)). Before every tool call the hook `POST`s the call to `/v1/runs/{run}/permissions`. The server evaluates the owner's rules ([governance.md](governance.md)) and records a `PolicyDecision`; `allow`/`deny` answer at once. `ask` records an `ApprovalRequested`, notifies the owner (Telegram), and holds the HTTP request open until `POST /v1/approvals/{id}` records an `ApprovalDecided`, or the request expires (just under the hook's six-hour timeout) and is denied as `by: timeout`. The hook prints Claude Code's `permissionDecision` and the call proceeds or is refused. If the server is unreachable the hook denies, so a dead office never means an unguarded agent.

### 3.2 Office stream *(built for runs; machines planned)*

`GET /v1/events/stream` tells a client about every run without polling. Rather than summary notifications (`run_started`, `approval_requested`, …), each frame is the run's whole view (`event: run`, the `RunView` JSON), so the stream is idempotent: a client keeps the latest view per run, and a repeat is harmless. Approvals are visible as `status.state = waiting_approval` with the summary and approval id; plans and routine runs are runs. A first pass over every run ends with an `event: ready` frame, so a client knows when its list is complete. The cursor is the view's `updated_at` (`id:`), and `Last-Event-ID` restricts the first pass to runs updated at or after it, so a reconnect may repeat a run but never misses one. Details and the wake-up mechanism: [`shiioo-api/docs/http.md`](../../crates/shiioo-api/docs/http.md). *(planned)* `event: machine` frames for nodes going online and offline; the desk ([desk.md](desk.md)) needs them for the Machines group.

### 3.3 Fan-out *(deferred, ADR-0010)*

In one server process new events are pushed in-process (`Notifying<S>`), which is the whole design for one office. The cluster form, gateways subscribing to Postgres `LISTEN/NOTIFY` for the run heads they stream with NATS as an optional replacement, is set aside with the cluster itself ([deployment.md](deployment.md)).

## 4. Idempotency *(simple form built)*

- Built: `POST /v1/runs` accepts an `Idempotency-Key` header; a repeat with the same key returns the first run with `200` instead of creating another. The map is in memory and does not survive a restart; the body is not compared.
- Planned: every creating or commanding `POST` accepts the header (any string up to 255 bytes; SDKs send a UUID by default). The server stores `(key, principal, endpoint) → (request digest, response)` for 24 hours in the store. A repeat with the same key and body returns the stored response; the same key with a different body returns `409` with type `idempotency-key-reused`.
- Planned: run commands also carry the key into the event (`UserMessage { idempotency_key, … }`), so a retry that races the first attempt can't append twice.

## 5. Pagination, filtering, errors *(errors built; pagination planned)*

- *(planned)* Lists use cursor pagination: `?limit=50&cursor=…`, response `{ "items": […], "next_cursor": "…" }`. Cursors are opaque and stable under inserts. Today `GET /v1/runs` returns every run, most recently updated first, as a plain array.
- Event reads use `?from={seq}` *(built)*, since `seq` is already a perfect cursor.
- Errors are RFC 9457 `application/problem+json` with stable `type` URIs *(built)*. Today's slugs are `unauthorized` (401), `not-found` (404), `bad-request` (400), `conflict` (409), `failed` and `internal` (500); the finer-grained ones below are the planned shape:

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
| `forbidden` | 403 | Authenticated but not allowed *(deferred with multi-owner auth, ADR-0008)* |
| `run-closed` | 409 | Command on a finished run |
| `idempotency-key-reused` | 409 | Same key, different body |
| `budget-exceeded` | 402 | Starting a run whose budget is already spent |
| `rate-limited` | 429 | With `Retry-After` |

## 6. Authentication *(one token built; the rest planned or deferred)*

Details and threat model in [security.md](security.md).

**Today:** one bearer token, generated on first start into `.shiioo/api-token` (mode 0600), required on every `/v1` route except `/v1/health`. The dashboard, the Telegram office, the hook (`SHIIOO_API_TOKEN` in the harness's environment) and `curl` all present the same token. The API listens on loopback; the phone reaches it over Tailscale ([deployment.md](deployment.md)).

| Principal | How it authenticates | Status |
|---|---|---|
| Humans (dashboard) | the office token, pasted once as `#token=…` and kept in the browser | built |
| Humans (dashboard, several owners) | OAuth2 / OIDC with the org's identity provider; session cookie, `SameSite=Strict` | deferred, ADR-0008 |
| Humans (CLI) | OAuth2 device authorization flow (`shiioo login`) | planned (the CLI reads the token file today) |
| Services | API keys: `Authorization: Bearer shk_…`, scoped to a role, optionally an expiry; shown once, stored as a hash | planned |
| Agents | Biscuit tokens minted per run ([ADR-0005](../adr/0005-cedar-policies-and-biscuit-credentials.md)), attenuated for subagents | deferred, ADR-0008 |
| Nodes | mTLS client certificate or key-bound token, only on the node endpoint ([tools-and-machines.md](tools-and-machines.md)) | planned with remote nodes |

*(deferred, ADR-0008)* Authorization for every request through the same Cedar evaluation as tool calls, so "who may approve deploys" and "which agent may call `shell`" are one policy language. With one owner there is nothing to authorize beyond the token.

**Rate limits** *(planned)* are per API key or user: a token bucket on requests and a separate concurrency limit on open streams. Responses carry `RateLimit-*` headers (IETF draft fields).

## 7. Versioning *(the rule applies today; the tooling is planned or deferred)*

- `/v1` only gets **additive** changes: new endpoints, new optional fields, new enum values, new event types. Clients must ignore unknown fields and event types; the dashboard and the bot do.
- A breaking change ships as `/v2` for the affected resources, with `/v1` kept for at least 12 months and announced via the `Deprecation` and `Sunset` headers.
- **Event payload schemas are part of the API.** *(planned)* JSON Schemas for every event type are published at `/v1/schemas/events/{type}.json` and versioned by the event's `schema` field (see [event-log.md](event-log.md)). Today the schema is the `Payload` enum in `shiioo-core`, serialized with serde's tagging.
- *(deferred, ADR-0010)* CI compares the generated OpenAPI with the last release and fails on breaking diffs inside `/v1`.

## 8. SDKs *(deferred, ADR-0010)*

One owner with a phone needs no SDK; the dashboard's `app.js` and the Telegram office are the two clients, and `curl` with the token covers scripts. If generation comes back:

- **TypeScript first**, then **Python**: generated from the OpenAPI document, plus hand-written helpers where generation is weak:
  - a resumable stream reader that handles `Last-Event-ID`, backoff and the event/delta split;
  - `run.wait()` that streams until a terminal event or an approval is needed;
  - automatic `Idempotency-Key` on commands, reused across retries.
- **Rust**: the server's own request and response types (`RunView`, the create body), published as `shiioo-api-types`, with a thin `reqwest` client. *(planned; the hook in the `shiioo` binary is the one Rust client today and uses `reqwest` directly.)*

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

- The office subscribes URLs to event kinds: `run.finished`, `run.failed`, `approval.requested`, `approval.decided`, `machine.offline`, `budget.threshold`.
- Deliveries are signed with HMAC-SHA256 over `timestamp.body` (`Shiioo-Signature: t=…,v1=…`), with a per-webhook secret, and rejected by receivers if older than 5 minutes.
- At-least-once with exponential backoff for 24 hours; each delivery carries a stable `id` for deduplication. Delivery attempts are visible in the API.
- Webhook payloads link back to the run and include the event `seq`, so the receiver can fetch the authoritative events.

## 11. Open questions

- Public domain for problem `type` URIs (`shiioo.dev` is a placeholder).
- Whether the office stream needs filtering per subscriber at all; with one owner, no. *(deferred with ADR-0008)*
