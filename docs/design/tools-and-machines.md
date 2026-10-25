# Tools and machines

How agents act on the world: the tool model, where tools execute, how the node on each machine talks to the server, how workspaces are isolated and snapshotted, and how secrets reach a tool without reaching the model. (Earlier drafts called the node a "daemon"; [ADR-0010](../adr/0010-single-binary-server-and-nodes.md) names the roles server and node.)

Built today (`crates/shiioo-runtime`): the tool trait and three built-in tools, the in-process `LocalMachine` with its journal and epoch fence, one git worktree per run, the `container` isolation level with egress allow lists, and secrets as named references with redaction (`shiioo-secrets`, `shiioo-runtime::secrets`, `::redact`). Remote nodes, MCP, snapshots and secret injection at the tool boundary of the native loop are *(planned)*; `openshell` is experimental.

Read [architecture.md](../architecture.md) first. Related: [execution.md](execution.md) (the turn loop that calls tools), [governance.md](governance.md) (policy decisions before a call), [fork-and-replay.md](fork-and-replay.md) (what snapshots are for), [security.md](security.md) (threat model), [ADR-0006](../adr/0006-daemon-dials-out-over-websocket.md).

## 1. The tool model

### 1.1 Tool trait *(built, `shiioo-runtime::tools`)*

```rust
pub trait Tool: Send + Sync {
    /// Name, description and JSON Schema as offered to the model.
    fn spec(&self) -> ToolSpec;
    /// How the call behaves if it may have run but its result was never recorded.
    /// Decided per call: a read-only shell command vs `git push`.
    fn class(&self, input: &serde_json::Value) -> ToolClass;
    /// Runs the call in the run's workspace.
    fn execute<'a>(&'a self, input: &'a serde_json::Value, workspace: &'a Path) -> BoxFuture<'a, ToolOutput>;
}
```

A `Toolbox` maps names to tools; the agent's `tools` list says which of them it may use. *(planned)* A `Placement` (Machine or Worker) once tools exist that need no machine (remote HTTP APIs, remote MCP servers, ask-human, spawn-subagent); today every tool runs on the machine.

`ToolClass` is the one in `shiioo-core` *(built)*: `Pure`, `Idempotent { key }`, `SideEffecting`. It decides recovery after a crash (see [execution.md](execution.md)): pure and idempotent calls are re-run, side-effecting ones become `ToolOutcomeUnknown` unless the machine's journal can settle them (section 3.4). Harness tool calls get a class too, by tool name (`Read`/`Grep` pure, `Edit`/`Write` idempotent, `Bash` side-effecting), so the log reads the same for every runtime.

Classification rules:

- A tool declares its class; the runtime never upgrades a call to a weaker class.
- When a tool can't tell, it says `SideEffecting`. Being wrong in that direction costs a human decision; being wrong the other way costs a duplicate `git push`.
- `Idempotent { key }` keys are derived from the input and the run (for example `issue-7:label:triaged`), not random, so a re-run hits the same key.

### 1.2 Call lifecycle *(built)*

Each call is one round through these events (names from `shiioo-core::Payload`):

```
ModelResponseRecorded            the model asked for tool calls
PolicyDecision                   allow / deny / ask, with the matching rule         → governance.md
ApprovalRequested … Decided      only if the rules said ask
ToolIntent                       call_id, tool, class, input
   … the machine executes …
ToolResult                       output (size-limited), is_error
```

Intent is always recorded before execution, and the result before the next model call. The worker never starts a call whose intent isn't durable. Today `PolicyDecision` and approvals exist for harness runs (through their hook); the native loop records intents and results but doesn't consult the rules yet. *(planned)* Secrets as references in inputs, redaction of outputs.

### 1.3 Built-in tools *(three built; the rest planned)*

| Tool | Status | Class | Notes |
|---|---|---|---|
| `shell` | built | `SideEffecting` | `sh -c` in the workspace, 120 s timeout; output truncated to the first and last 32 KiB. The rules decide which command patterns need no approval ([governance.md](governance.md)) |
| `read_file` | built | `Pure` | Path relative to the workspace, no `..`; size-limited |
| `write_file` | built | `Idempotent` (key = digest of path + content) | Whole-file write, creates parent directories |
| `fs.edit` | planned | `Idempotent` (key = path + old/new digests) | Exact string replacement; fails if `old` doesn't match exactly once |
| `search` | planned | `Pure` | ripgrep-style content search and glob file search |
| `web.fetch` | planned (worker-placed) | `Pure` | GET only, size-limited, readable-text extraction. Egress governed by the rules |
| `ask_human` | planned (worker-placed) | `Pure` | Parks the run until a user message answers it; see [execution.md](execution.md) |
| `spawn_subagent` | planned (worker-placed) | `Idempotent` (key = call_id) | Starts a child run with a subset of the parent's permissions; returns when the child finishes or immediately with its run id |

Harness runs bring their own tools (Claude Code's `Read`/`Edit`/`Bash`/…, Codex's commands and patches); Shiioo records them, it doesn't run them ([runtimes-and-providers.md](runtimes-and-providers.md)).

### 1.4 MCP *(planned)*

Shiioo is an MCP client. Custom tools are MCP servers; Shiioo has no separate plugin format.

| MCP transport | Runs where | Typical use |
|---|---|---|
| stdio | On the Machine, launched by the node inside the run's sandbox | Local tools: databases, language servers, project scripts |
| Streamable HTTP | From the worker | Hosted tools: GitHub, Linear, Sentry, internal services |

- MCP tools are namespaced `mcp.<server>.<tool>`.
- MCP has no idempotency annotation we can trust, so MCP tools default to `SideEffecting`. MCP's `readOnlyHint` annotation, when the owner's config trusts that server, maps to `Pure`. The config can override per tool.
- OAuth for remote MCP servers is done once by the owner; the token is stored as a Secret.
- Tool lists are fetched when a run starts and pinned in the run's first turn (as part of the context recorded in the log), so a server changing its tools mid-run can't change what a replay sees.

## 2. Machines *(local machine built; remote nodes planned)*

A **Machine** is where a run's tools and harness execute. Today it is the in-process `LocalMachine` of the office itself (`shiioo-runtime::machine`): the server and the machine are one process (`shiioo up`). *(planned)* A remote `shiioo node` on any other host (an inference box, a laptop, a CI runner, a sandbox) implements the same contract over the wire. Each run is bound to at most one machine.

### 2.1 Enrollment

1. The owner creates an enrollment token (`POST /v1/machines/enrollments`). It is single-use and short-lived (default 1 hour).
2. On the machine: `shiioo node enroll --server https://… --token …`. The node generates a key pair locally; the private key never leaves the machine.
3. The server exchanges the token for a long-lived **machine credential**: an mTLS client certificate signed by the office's CA, or, where TLS termination makes client certs impractical, a signed bearer token bound to the node's public key (each connection proves possession with a signed challenge). *(deferred, ADR-0008)* Biscuit tokens as in [ADR-0005](../adr/0005-cedar-policies-and-biscuit-credentials.md).
4. The credential carries machine id and labels (`os=linux`, `gpu=true`, `pool=ci`). Revoking the machine revokes the credential.

### 2.2 Connection: the node dials out

The node opens a WebSocket over TLS to the server (`wss://…/v1/node`), over Tailscale in practice, and keeps it open ([ADR-0006](../adr/0006-daemon-dials-out-over-websocket.md)). No inbound ports, works through NAT and HTTP proxies. QUIC is a possible later transport behind the same protocol.

Frames are protobuf messages from `shiioo-protocol` (prost). One frame per WebSocket binary message.

```protobuf
syntax = "proto3";
package shiioo.node.v1;

message Frame {
  oneof msg {
    Hello hello = 1;            // node → server, first frame
    Welcome welcome = 2;        // server → node
    Heartbeat heartbeat = 3;    // both ways
    ToolInvocation invoke = 4;  // server → node
    ToolOutputDelta delta = 5;  // node → server, ephemeral
    ToolCompletion done = 6;    // node → server
    JournalQuery query = 7;     // server → node
    JournalAnswer answer = 8;   // node → server
    SnapshotRequest snapshot = 9;
    SnapshotTaken snapshot_taken = 10;
    Cancel cancel = 11;
  }
}

message Hello {
  uint32 protocol_version = 1;          // major; minor changes are additive
  repeated string capabilities = 2;     // "snapshot.git", "isolation.landlock", "mcp.stdio", …
  string node_version = 3;
  repeated InFlight in_flight = 4;      // resume: calls this node is still running or finished unacknowledged
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

- **Version negotiation.** `Hello` carries the protocol major version and capabilities. The server answers with the version and capability set it will use, or closes with a clear error telling the owner to upgrade. Unknown fields are ignored (proto3), so minor versions are additive.
- **Heartbeats** every 15 s both ways; a connection silent for 45 s is closed. The machine's status (`online`, `offline`) is derived from its session, not stored as truth.
- **Reconnect with resume.** After any drop the node reconnects with backoff and lists in `Hello.in_flight` every call it is still running or has finished without an acknowledged `ToolCompletion`. The server re-attaches those calls to whichever worker now holds the run, so a network blip never turns into an unknown outcome.
- **Stale workers.** `ToolInvocation.epoch` lets the node drop invocations from a worker that has been fenced: the node records the highest epoch it has seen per run and refuses older ones. *(built in `LocalMachine`: `Invocation.epoch`, `MachineError::Fenced`.)*

### 2.3 Routing

- **One server:** the server holds every node connection in process; a map from machine id to connection is enough. This is the product ([ADR-0010](../adr/0010-single-binary-server-and-nodes.md)).
- *(deferred, ADR-0010)* Several server instances would need a `machine_sessions` table recording which instance holds each connection.
- Tool output deltas flow node → server → SSE subscribers directly, without touching the event store.

## 3. Executing a call on the machine *(built for the local machine)*

### 3.1 Journal

The machine keeps a local, fsynced journal (`LocalMachine`: an append-only JSON-lines file, `<data>/journal.jsonl`; a torn last line from a crash mid-write is ignored), one record per step:

| Record | Written when | Contains |
|---|---|---|
| `fence` | When a run's highest seen epoch rises | run id, epoch |
| `started` | Immediately before the tool runs | run id, call id |
| `finished` | After the result is captured, before it is returned | run id, call id, output |
| `reset` | When a pure or idempotent call that started but never finished is run again | run id, call id |

*(planned)* A `received` state for remote nodes (acknowledged before start), and compaction of old entries to `call_id → result digest` after 7 days; today the journal only grows.

### 3.2 Deduplication *(built)*

An invocation whose `call_id` is already in the journal is not executed again (`LocalMachine::execute`):

- `finished`: return the stored output.
- `started`: refuse with `OutcomeUnknown`; only `rerun` (for pure and idempotent calls) resets and runs it again.
- *(planned, remote nodes)* `started` with the process still alive: re-attach output streaming.

### 3.3 Cancellation and limits

- *(planned)* `Cancel { call_id }` sends SIGTERM to the process group, then SIGKILL after a grace period. A cancelled side-effecting call is reported as cancelled-after-start, which the runtime treats like an unknown outcome unless the tool says otherwise.
- *(built)* `shell` has a 120 s timeout and the recorded result keeps the first and last 32 KiB. *(planned)* Per-rule timeouts; the full output in the blob store when the tool asks for it.

### 3.4 Reconciliation after a worker crash *(built for the local machine)*

When a new worker takes over a run with a dangling intent, it asks the machine before deciding (`LocalMachine::status`, which also raises the run's fence so the old worker is locked out):

| Journal says | Outcome recorded by the new worker |
|---|---|
| Not present | The call never reached the machine: run it now (safe, nothing ran) |
| `started`, not `finished` | Pure or idempotent: run again. Side-effecting: `ToolOutcomeUnknown`, the run stops for a human |
| `finished` | Record its output as the `ToolResult` |
| *(planned)* `received`, not `started` | Re-send; the node will start it once |
| *(planned)* `started`, still running on a remote node | Re-attach and wait for the result |
| *(planned)* Node unreachable, or journal lost | `ToolOutcomeUnknown` → reconcile hook or human |

This keeps `ToolOutcomeUnknown` for the cases where nobody can know, instead of every crash.

## 4. Isolation *(`worktree` and `container` built; `openshell` experimental)*

The levels are the three named in [positioning.md](../positioning.md#isolation-levels), chosen per office (`[isolation] default`), per repository (`[isolation.repos]`) or per run (`--isolation`, the API's `isolation`); a machine declares which it supports (`Hello.capabilities` for remote nodes), and a run is only scheduled on a machine that meets it. Decision record: [ADR-0011](../adr/0011-containers-through-the-owners-engine.md).

| Level | Mechanism | Status |
|---|---|---|
| `worktree` | Own git worktree and branch; host process as the owner's user; no OS sandbox. Claude Code runs without one, and Codex's own sandbox is turned off because it blocks commits ([runtimes-and-providers.md](runtimes-and-providers.md)) | built |
| `container` | One OCI container per run through the owner's Docker or Podman CLI: read-only root, `--cap-drop ALL`, `no-new-privileges`, pid limit, optional `--memory`/`--cpus`; the worktree and the repository's `.git` bind-mounted at their host paths (so the worktree's `.git` pointer resolves); a per-run home under `.shiioo/sandboxes/<run>/home` that persists for resumes; Codex's `auth.json` mounted read-only, Claude Code's OAuth token from the secret store; the hook is `curl` to `POST /v1/runs/{id}/hook/pre-tool-use` over `host.docker.internal`; egress as below. The image is built on first use from an embedded Dockerfile (Node, git, `gh`, build tools, both harnesses) and tagged by its hash; `shiioo sandbox build` builds it ahead of time | built |
| `openshell` | NVIDIA OpenShell sandbox through the `openshell` CLI: created with a policy generated from the egress rules and an uploaded copy of the worktree, the harness run with `sandbox exec`, deleted afterwards. The upload is a copy, so results come back through git | experimental: built against the documentation, not run against a gateway |

Every harness start records a `SandboxStarted { level, detail }` event: for `container` the image, container name, mounts and egress mode.

What the process inside a container runs as: root, with no capabilities. The user id is not the boundary (root in a container with `CAP_DROP ALL` is an ordinary user to the host), and under colima bind mounts are writable only by root; files it creates belong to the owner on the host. The office and its repositories must be under a directory the engine shares with its VM (the home directory by default); a preflight mount check reports a path that is not.

Earlier drafts had a longer ladder (`host`, `process`, `container`, `microvm`, `provider`). `host` became `worktree`; `process` (Landlock/seccomp/Seatbelt) is covered by `openshell`; `microvm` and third-party `provider` sandboxes are *(deferred, ADR-0008)*: not needed for one owner on one machine. If a provider comes back, it plugs in as before: it starts a sandbox, the node inside it enrolls with a short-lived token minted for that run, and from then on it is an ordinary machine:

```rust
pub trait SandboxProvider: Send + Sync {
    async fn create(&self, spec: &SandboxSpec, enrollment: EnrollmentToken) -> Result<SandboxId>;
    async fn destroy(&self, id: &SandboxId) -> Result<()>;
    fn capabilities(&self) -> &[Capability];
}
```

**Egress** *(built for `container`)*. Above `worktree`, network access is denied by default. A `container` run joins an internal Docker network (`shiioo-sandbox`) that has no route anywhere; its only way out is a tinyproxy sidecar per run (`shiioo-egress-<run>`) attached to that network and to the default bridge, which forwards `CONNECT` on port 443 and plain HTTP to the host patterns of `[egress] allow` (shell patterns such as `*.github.com`; the built-in list covers the model providers, GitHub and the common package registries; `also` extends it) and refuses the rest. The office's host is always allowed, for the hook. The harness learns of the proxy through `HTTPS_PROXY`, `HTTP_PROXY` and their lower-case forms; a program that ignores them cannot connect. `[egress] mode = "open"` runs the container on the default bridge without a proxy. `worktree` runs are not restricted. For `openshell`, the same list becomes the sandbox's `network_policies` (every endpoint `enforce`d on 443, plus the office's host and port).

## 5. Workspaces and snapshots *(workspaces built; snapshots planned)*

### 5.1 Workspaces *(built, `shiioo-runtime::workspace`)*

A workspace is a directory on the machine. A run on a repository gets its own git worktree, `<data>/worktrees/<run>` on branch `shiioo/<run prefix>` created from the repository's `HEAD`, reused as is when the run is resumed after a crash; a run without a repository works in the machine's workspace directory. The branch outlives the run: the owner reviews, merges or deletes it (`GET /v1/runs/{id}/diff` shows it against `HEAD`). *(planned)* Shared workspaces for long-lived agents (with a lock), and sources other than a local repository (a git URL and ref, an archive, a snapshot of another run).

### 5.2 Snapshots *(planned)*

The node snapshots the workspace at turn boundaries (after all tool results of a turn are recorded) and on request. Each snapshot is recorded as an event:

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

## 6. Secrets *(store, references and redaction built; injection at the native tool boundary planned)*

Built (`shiioo-secrets`, `shiioo-runtime::secrets`, `shiioo-runtime::redact`; [security.md §3.2](security.md#32-secrets-never-reach-the-record-built-never-reach-the-model-holds-for-the-native-loop-planned)):

- Secrets are stored encrypted in the office directory (`.shiioo/secrets.enc`, XChaCha20-Poly1305 under `.shiioo/secrets.key`, both mode 0600) and managed with `shiioo secret set|list|rm`; a value is read from stdin, an environment variable or a file, never from the command line.
- A harness run's environment is declared with references: `[env] GH_TOKEN = "secret:gh-token"` in `config.toml` for every run, or `env` in the run's spec. The worker resolves them when the harness starts; the spec in `RunCreated` holds the reference. A reference nobody can resolve fails the run with the secret's name.
- Before an event is recorded, every string in it is scanned for the known values (raw, base64, URL-encoded) and each occurrence becomes `[secret:NAME]`; a `SecretRedacted { name, count }` event names what was found. The office's API token is treated the same way.
- In a `container` run the environment reaches the process through a 0600 env file in the run's sandbox directory, removed at teardown, not through the command line.

Planned:

- The native loop injects values at the tool boundary, after the policy check, so the model never sees them; tool inputs reference secrets by name (`{"$secret": "GITHUB_TOKEN"}`) and the `ToolIntent` records only references. Each resolution is an event (`SecretResolved { call_id, name }`).
- Remote nodes fetch values just-in-time with a short-lived grant bound to the run, and never persist them.
- Envelope encryption: the key wrapped by an `age` identity or a KMS key, so rotation re-wraps the key without re-encrypting values.

## 7. Streaming tool output *(planned)*

- stdout/stderr chunks are sent as `ToolOutputDelta` frames and forwarded to SSE subscribers as ephemeral deltas (see [api.md](api.md)). They are not events and are not hashed; a client that reconnects sees the final `ToolResult` instead.
- Only the final, redacted, size-limited `ToolResult` is part of the log.

## 8. Open questions

- Windows machines: isolation story (AppContainer, Job objects) not designed yet.
- Whether `shell` should parse commands to infer `Pure` automatically, or only via policy allowlists. Starting with allowlists.
- Snapshot retention and its interaction with archival in [event-log.md](event-log.md).
