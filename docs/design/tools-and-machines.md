# Tools and machines

How agents act on the world: the tool model, where tools execute, how the daemon on each machine talks to the server, how workspaces are isolated and snapshotted, and how secrets reach a tool without reaching the model.

Read [architecture.md](../architecture.md) first. Related: [execution.md](execution.md) (the turn loop that calls tools), [governance.md](governance.md) (policy decisions before a call), [fork-and-replay.md](fork-and-replay.md) (what snapshots are for), [security.md](security.md) (threat model), [ADR-0006](../adr/0006-daemon-dials-out-over-websocket.md).

## 1. The tool model

### 1.1 Tool trait *(planned, `shiioo-tools`)*

```rust
pub trait Tool: Send + Sync {
    /// Stable, namespaced name the model sees: "shell", "fs.edit", "github.create_issue".
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON Schema for the input. Validated before the intent is recorded.
    fn input_schema(&self) -> &serde_json::Value;
    /// How the call behaves if it may have run but its result was never recorded.
    /// Can depend on the input (a read-only shell command vs `git push`).
    fn class(&self, input: &serde_json::Value) -> ToolClass;
    /// Where the call executes.
    fn placement(&self) -> Placement;
}

pub enum Placement {
    /// On the run's Machine, through its daemon (shell, files, local MCP servers).
    Machine,
    /// In the worker process (remote HTTP APIs, remote MCP servers, ask-human, spawn-subagent).
    Worker,
}
```

`ToolClass` is the one in `shiioo-core` *(built)*: `Pure`, `Idempotent { key }`, `SideEffecting`. It decides recovery after a crash (see [execution.md](execution.md)): pure and idempotent calls are re-run, side-effecting ones become `ToolOutcomeUnknown` unless the daemon journal can settle them (section 3.4).

Classification rules:

- A tool declares its class; the runtime never upgrades a call to a weaker class.
- When a tool can't tell, it says `SideEffecting`. Being wrong in that direction costs a human decision; being wrong the other way costs a duplicate `git push`.
- `Idempotent { key }` keys are derived from the input and the run (for example `issue-7:label:triaged`), not random, so a re-run hits the same key.

### 1.2 Call lifecycle *(planned)*

Each call is one round through these events (names from `shiioo-core::Payload`; new ones marked):

```
ModelResponseRecorded            the model asked for tool calls
PolicyDecision       (new)       allow / deny / needs approval, with policy version  → governance.md
ApprovalRequested … Decided (new) only if needed
ToolIntent                       call_id, tool, class, input (secrets as references, never values)
   … daemon or worker executes …
ToolResult                       output (redacted, size-limited), is_error
```

Intent is always recorded before execution, and the result before the next model call. The worker never starts a call whose intent isn't durable.

### 1.3 Built-in tools *(planned)*

| Tool | Placement | Class | Notes |
|---|---|---|---|
| `shell` | Machine | `SideEffecting` by default | Command, cwd, timeout, env refs. Policies can mark command patterns as `Pure` (for example `git status`, `ls`) |
| `fs.read` | Machine | `Pure` | Path, line range. Size-limited |
| `fs.write` | Machine | `Idempotent` (key = path + content digest) | Whole-file write |
| `fs.edit` | Machine | `Idempotent` (key = path + old/new digests) | Exact string replacement; fails if `old` doesn't match exactly once |
| `search` | Machine | `Pure` | ripgrep-style content search and glob file search |
| `web.fetch` | Worker | `Pure` | GET only, size-limited, readable-text extraction. Egress governed by policy |
| `ask_human` | Worker | `Pure` | Parks the run until a user message answers it; see [execution.md](execution.md) |
| `spawn_subagent` | Worker | `Idempotent` (key = call_id) | Starts a child run with an attenuated credential; returns when the child finishes or immediately with its run id |

### 1.4 MCP *(planned)*

Shiioo is an MCP client. Custom tools are MCP servers; Shiioo has no separate plugin format.

| MCP transport | Runs where | Typical use |
|---|---|---|
| stdio | On the Machine, launched by the daemon inside the run's sandbox | Local tools: databases, language servers, project scripts |
| Streamable HTTP | From the worker | Hosted tools: GitHub, Linear, Sentry, internal services |

- MCP tools are namespaced `mcp.<server>.<tool>`.
- MCP has no idempotency annotation we can trust, so MCP tools default to `SideEffecting`. MCP's `readOnlyHint` annotation, when the project's config trusts that server, maps to `Pure`. Project config can override per tool.
- OAuth for remote MCP servers is done once per project by a human; the token is stored as a Secret.
- Tool lists are fetched when a run starts and pinned in the run's first turn (as part of the context recorded in the log), so a server changing its tools mid-run can't change what a replay sees.

## 2. Machines *(planned)*

A **Machine** is any host or sandbox that runs `shiioo daemon`: a developer laptop, a CI runner, a VM, a container, a Firecracker microVM, or a sandbox from a third-party provider. Each run is bound to at most one machine; runs that only use worker-placed tools need none.

### 2.1 Enrollment

1. An admin creates an enrollment token in a project (`POST /v1/projects/{id}/machines/enrollments`). It is single-use, short-lived (default 1 hour) and scoped to that project.
2. On the machine: `shiioo daemon enroll --server https://… --token …`. The daemon generates a key pair locally; the private key never leaves the machine.
3. The server exchanges the token for a long-lived **machine credential**: an mTLS client certificate signed by the project's CA, or, where TLS termination makes client certs impractical, a Biscuit token bound to the daemon's public key (each connection proves possession with a signed challenge).
4. The credential carries machine id, project id and labels (`os=linux`, `gpu=true`, `pool=ci`). Revoking the machine revokes the credential.

### 2.2 Connection: daemon dials out

The daemon opens a WebSocket over TLS to a gateway (`wss://…/v1/daemon`) and keeps it open ([ADR-0006](../adr/0006-daemon-dials-out-over-websocket.md)). No inbound ports, works through NAT and HTTP proxies. QUIC is a possible later transport behind the same protocol.

Frames are protobuf messages from `shiioo-protocol` (prost). One frame per WebSocket binary message.

```protobuf
syntax = "proto3";
package shiioo.daemon.v1;

message Frame {
  oneof msg {
    Hello hello = 1;            // daemon → gateway, first frame
    Welcome welcome = 2;        // gateway → daemon
    Heartbeat heartbeat = 3;    // both ways
    ToolInvocation invoke = 4;  // gateway → daemon
    ToolOutputDelta delta = 5;  // daemon → gateway, ephemeral
    ToolCompletion done = 6;    // daemon → gateway
    JournalQuery query = 7;     // gateway → daemon
    JournalAnswer answer = 8;   // daemon → gateway
    SnapshotRequest snapshot = 9;
    SnapshotTaken snapshot_taken = 10;
    Cancel cancel = 11;
  }
}

message Hello {
  uint32 protocol_version = 1;          // major; minor changes are additive
  repeated string capabilities = 2;     // "snapshot.git", "isolation.landlock", "mcp.stdio", …
  string daemon_version = 3;
  repeated InFlight in_flight = 4;      // resume: calls this daemon is still running or finished unacknowledged
}

message ToolInvocation {
  string run_id = 1;
  string call_id = 2;                   // the idempotency key for delivery
  string tool = 3;
  bytes input_json = 4;
  repeated SecretRef secrets = 5;       // references; values fetched just-in-time (section 5)
  uint64 timeout_ms = 6;
  uint64 epoch = 7;                     // lease epoch of the requesting worker
}
```

- **Version negotiation.** `Hello` carries the protocol major version and capabilities. The gateway answers with the version and capability set it will use, or closes with a clear error telling the operator to upgrade. Unknown fields are ignored (proto3), so minor versions are additive.
- **Heartbeats** every 15 s both ways; a connection silent for 45 s is closed. The machine's status (`online`, `offline`) is derived from its session, not stored as truth.
- **Reconnect with resume.** After any drop the daemon reconnects with backoff and lists in `Hello.in_flight` every call it is still running or has finished without an acknowledged `ToolCompletion`. The gateway re-attaches those calls to whichever worker now holds the run, so a network blip never turns into an unknown outcome.
- **Stale workers.** `ToolInvocation.epoch` lets the daemon drop invocations from a worker that has been fenced: the gateway rejects any invocation whose epoch is below the run's current one, and the daemon records the highest epoch it has seen per run.

### 2.3 Gateway routing

- **Single node:** the gateway is in-process; a map from machine id to connection is enough.
- **Cluster:** a `machine_sessions` table records which gateway instance holds each machine's connection (`machine_id`, `gateway_id`, `session_id`, `connected_at`). A worker that needs a machine looks up the gateway and sends the invocation to it over the internal RPC. On reconnect, the new gateway overwrites the row; a stale gateway that tries to deliver finds its `session_id` no longer current and drops the frame.
- Tool output deltas flow daemon → gateway → SSE subscribers directly, without touching the event store.

## 3. Executing a call on the daemon *(planned)*

### 3.1 Journal

The daemon keeps a local, fsynced journal (a small redb file in the daemon's state directory), one entry per `call_id`:

| State | Written when | Contains |
|---|---|---|
| `received` | Before acknowledging the invocation | run id, call id, epoch, input digest |
| `started` | Immediately before the process or effect starts | start time, pid |
| `finished` | After the result is captured, before sending `ToolCompletion` | exit status, result digest, result (until acknowledged) |

Entries are kept for 7 days after acknowledgement, then compacted to `call_id → result digest`.

### 3.2 Deduplication

An invocation whose `call_id` is already in the journal is not executed again:

- `finished`: resend the stored completion.
- `started`, process still alive: re-attach output streaming.
- `received` but not `started`: safe to start; nothing has happened.

### 3.3 Cancellation and limits

- `Cancel { call_id }` sends SIGTERM to the process group, then SIGKILL after a grace period. A cancelled side-effecting call is reported as cancelled-after-start, which the runtime treats like an unknown outcome unless the tool says otherwise.
- Every call has a timeout (default 10 minutes, policy can raise it) and output limits: the recorded result keeps the first and last 32 KiB of each stream plus total byte counts; the full output goes to the blob store if the tool asks for it.

### 3.4 Reconciliation after a worker crash

When a new worker takes over a run with a dangling side-effecting intent, it asks the daemon before deciding (`JournalQuery { call_id }`):

| Journal says | Outcome recorded by the new worker |
|---|---|
| Not present | The call never reached the machine: re-send it (safe, nothing ran) |
| `received`, not `started` | Re-send; the daemon will start it once |
| `started`, still running | Re-attach and wait for the result |
| `finished` | Record its result as the `ToolResult` |
| Daemon unreachable, or journal lost | `ToolOutcomeUnknown` → reconcile hook or human |

This keeps `ToolOutcomeUnknown` for the cases where nobody can know, instead of every crash.

## 4. Isolation *(planned)*

Each machine declares the isolation levels it supports (`Hello.capabilities`); each agent's policy states the minimum it needs. A run is only scheduled on a machine that meets it.

| Level | Mechanism | Use |
|---|---|---|
| `host` | Runs as the daemon's user | Your own laptop, explicit opt-in per project; shown prominently in the UI |
| `process` | Linux: Landlock (filesystem), seccomp (syscalls), network namespaces for egress control. macOS: `sandbox-exec` Seatbelt profiles | Default for developer machines |
| `container` | OCI container per run (rootless podman or Docker) | CI runners, shared servers |
| `microvm` | Firecracker microVM per run, from a pool of warm snapshots | Untrusted code, multi-tenant cloud |
| `provider` | A third-party sandbox service | Burst capacity without running hosts |

Third-party sandboxes plug in through a provider trait; the provider starts a sandbox, the daemon inside it enrolls with a short-lived token minted for that run, and from then on it is an ordinary machine:

```rust
pub trait SandboxProvider: Send + Sync {
    async fn create(&self, spec: &SandboxSpec, enrollment: EnrollmentToken) -> Result<SandboxId>;
    async fn destroy(&self, id: &SandboxId) -> Result<()>;
    fn capabilities(&self) -> &[Capability];
}
```

Network egress is denied by default above `host`; policies allow destinations by host pattern.

## 5. Workspaces and snapshots *(planned)*

### 5.1 Workspaces

A workspace is a directory on the machine: one per run by default, or one shared per project for long-lived agents (with a lock so two runs never write it at once). A run's workspace is created from a source (git URL and ref, an archive, a snapshot of another run, or empty).

### 5.2 Snapshots

The daemon snapshots the workspace at turn boundaries (after all tool results of a turn are recorded) and on request. Each snapshot is recorded as an event:

```rust
SnapshotTaken { turn: u32, snapshot_id: String, digest: String, method: SnapshotMethod }
```

| Method | How | Trade-off |
|---|---|---|
| `git` (default) | Commit the working tree, including untracked files minus ignores, into a hidden ref `refs/shiioo/<run>/<turn>` without touching the user's index or branches | Cheap, portable, works anywhere git does; captures files only |
| `btrfs` / `zfs` | Read-only subvolume or dataset snapshot | Instant, whole directory including ignored files; needs the filesystem |
| `overlayfs` | Freeze the upper layer per turn | Good for containers |
| `firecracker` | MicroVM memory and disk snapshot | Captures running processes too; largest |

Snapshots are what make fork, rewind and diff possible: see [fork-and-replay.md](fork-and-replay.md). The digest is part of the event, so the hash chain covers the workspace state. Snapshots can be exported to the blob store so a fork can start on a different machine.

## 6. Secrets *(planned)*

- Secrets are stored encrypted on the server (envelope encryption; see [security.md](security.md)). The model never sees a secret value, and no event contains one.
- Tool inputs reference secrets by name (`{"$secret": "GITHUB_TOKEN"}`) or agents declare them as env bindings. The `ToolIntent` records only references.
- At execution, the daemon asks the server for the values just-in-time over the authenticated connection, scoped to that `call_id` and the run's credential. The server checks policy again at this point.
- The daemon injects values as environment variables or files with mode 0600 in a per-call temp directory, deleted after the call.
- Before a result leaves the daemon, every injected value (and common encodings of it: base64, URL-encoded) is replaced with `[secret:NAME]`. The worker repeats the redaction for worker-placed tools.
- Each resolution is an event (`SecretResolved { call_id, name }`, value never included), so the audit trail shows which call could see which secret.

## 7. Streaming tool output *(planned)*

- stdout/stderr chunks are sent as `ToolOutputDelta` frames and forwarded to SSE subscribers as ephemeral deltas (see [api.md](api.md)). They are not events and are not hashed; a client that reconnects sees the final `ToolResult` instead.
- Only the final, redacted, size-limited `ToolResult` is part of the log.

## 8. Open questions

- Windows machines: isolation story (AppContainer, Job objects) not designed yet.
- Whether `shell` should parse commands to infer `Pure` automatically, or only via policy allowlists. Starting with allowlists.
- Snapshot retention per project and its interaction with archival in [event-log.md](event-log.md).
