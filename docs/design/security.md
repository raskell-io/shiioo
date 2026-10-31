# Security

Agents act with real credentials on real systems, and they read untrusted text all day. This document states what Shiioo defends against, how, and, just as important, what it does not defend against.

Related: [governance.md](governance.md) (rules, approvals), [tools-and-machines.md](tools-and-machines.md) (nodes and isolation), [event-log.md](event-log.md) (hash chain), [telegram.md](telegram.md#security) (the phone), [ADR-0005](../adr/0005-cedar-policies-and-biscuit-credentials.md), [ADR-0006](../adr/0006-daemon-dials-out-over-websocket.md) (the node, then called the daemon), [ADR-0008](../adr/0008-personal-workforce-not-managed-cloud.md) (one owner, one machine).

Status: the controls are the design for the full platform. [§3.0](#30-todays-posture-built) says what stands today, for one owner on one Mac, including the phase-4 controls ([ADR-0011](../adr/0011-containers-through-the-owners-engine.md)): the `container` level, egress allow lists, the secret store and redaction; the multi-owner controls (Biscuit, tenant isolation, SSO) are *(deferred, ADR-0008)*.

## 1. Assets

| Asset | Why it matters |
|---|---|
| Credentials and secrets | API keys, deploy keys, cloud credentials the agents use |
| Machines and workspaces | source code, data, anything the node can reach |
| The event log | the record of what happened; its integrity is the audit |
| Configuration | rules, employees, agent definitions: changing them changes what agents may do |
| Model and tool spend | budgets are money |

## 2. Threat model

| # | Threat | Example |
|---|---|---|
| T1 | **Prompt injection** through tool outputs | a web page, issue comment, README or email tells the agent to exfiltrate a secret or push to main |
| T2 | **Malicious or compromised agent definition** | instructions or a skill crafted to abuse tools |
| T3 | **Runaway agent** | loops, fork bombs of subagents, budget exhaustion |
| T4 | **Compromised machine** | the node's host is taken over; it can lie about tool results and see its workspace |
| T5 | **Compromised worker** | an attacker controls a worker process: it holds a lease and can call models and dispatch tools |
| T6 | **Insider with database access** | edits or deletes events to hide what happened |
| T7 | **Cross-tenant access** | one organization reads or affects another's runs *(deferred, ADR-0008: one owner)* |
| T8 | **Supply chain** | a compromised dependency, MCP server, skill or harness |
| T9 | **Stolen client credentials** | a leaked API key or session token |

## 3. Controls

### 3.0 Today's posture *(built)*

One owner, one Mac, the owner's own repositories. What stands:

- **Nothing listens publicly.** The API binds to loopback and needs the office token on every call; the phone reaches it over Tailscale. Telegram is long-polled, and the bot obeys one user id ([telegram.md](telegram.md#security)). (T9: a stolen token is the one credential that matters; rotate by deleting `.shiioo/api-token` and restarting.)
- **Every harness tool call goes through the server.** Claude Code and Codex run with their own permission prompts off and a hook that asks `POST /v1/runs/{run}/permissions` before each tool call; the owner's `rules.toml` answers allow, deny or ask, and ask reaches the phone ([governance.md](governance.md)). A dead server means deny. The default rule set allows reads, edits and build commands inside the worktree and asks for `git push`, `gh`, package installs and anything it does not recognize. (T1, T2: the line of defence is the owner's judgement on each ask.)
- **Runaway runs are bounded** by `max_turns`, `max_minutes` and `max_budget_usd`, and the owner can `/cancel` from the phone (T3).
- **Two isolation levels in use.** `worktree` (the default): a harness runs as the owner's user with the owner's `PATH`, keychain and network, in its own git worktree; Codex's own sandbox is switched off because it blocks commits in a worktree ([runtimes-and-providers.md](runtimes-and-providers.md)). `container`: the harness runs in a Docker (or Podman) container per run with a read-only root, no capabilities, `no-new-privileges`, a pid limit, optional CPU and memory limits, only the worktree and the repository's `.git` mounted, and egress through an allow-listing proxy ([tools-and-machines.md §4](tools-and-machines.md#4-isolation-worktree-and-container-built-openshell-experimental)). The level is set per office and per repository in `config.toml`, or per run. `openshell` is an experimental adapter. Every run records where it executed (`SandboxStarted`).
- **Egress for `container` runs is an allow list** enforced at the network layer: the container's only route out is a proxy that admits TLS tunnels and HTTP to the listed host patterns (model providers, GitHub and package registries by default). `worktree` runs are not restricted.
- **Credentials are the harnesses' own**, and the office has a secret store. Claude Code uses its keychain login on the host, Codex its auth file; the worker strips `ANTHROPIC_API_KEY`, `OPENAI_API_KEY` and `CODEX_API_KEY` from their environment so the subscription pays. In a container, Codex's auth file is mounted read-only and Claude Code gets an OAuth token from the secret store (§3.2). Secrets for the agents (a `GH_TOKEN`) are stored encrypted in `.shiioo/secrets.enc`, referenced by name in `config.toml` (`[env] GH_TOKEN = "secret:gh-token"`) and resolved when the harness starts; every value is redacted from anything recorded. The office's own tokens live in `.shiioo/api-token` and `.shiioo/telegram-token`, mode 0600, or in environment variables; the API token is redacted too.
- **The record is hash-chained** (§3.5) and verified by `shiioo log`; it is not anchored anywhere outside the machine.
- **Other machines are the owner's.** A node enrolls with a one-time token and holds a machine token that the office accepts like its own; tokens are stored hashed in `.shiioo/machines.json`, and revoking a machine stops its token at once ([ADR-0012](../adr/0012-nodes-are-workers-over-the-offices-api.md)). Nodes reach the office over Tailscale; the office never connects to a node. A compromised node can read every run and answer approvals (T4, T5): with one owner that is the owner's own machine; scopes per machine come with multi-owner use.
- **Mail never gives the agent the account.** `shiioo mail send` asks the office; the office judges it as the tool `mail.send` (the default rule asks the owner), sends from the account in `[mail]` with the password from the secret store, and records `MailSent`. The agent sees neither the password nor the SMTP session.
- **One machine for everything else.** T6 to T8 are either the same host as the server (nothing to isolate against), or deferred with multi-owner use (T7).

### 3.1 Least privilege by construction *(deferred, ADR-0008)*

- Every run gets a **Biscuit token** minted by the server, scoped to its office, agent, run and the tools and resources its policy allows. Child runs get a token **attenuated** from the parent's: Biscuit attenuation can only remove rights, offline, so a subagent can never do more than its parent (T2, T3, T5).
- The node checks the token on every tool invocation, independently of the worker. A compromised worker cannot use a machine it was not authorized for (T5).
- Tokens are short-lived (default: lease TTL plus margin) and bound to the run id.

### 3.2 Secrets never reach the record *(built; "never reach the model" holds for the native loop, planned)*

- Secrets are referenced by name (`[env] GH_TOKEN = "secret:gh-token"` in `config.toml`, `env` in a run's spec) and resolved by the worker when the harness starts (`shiioo-runtime::secrets`). The run's spec, and so the log, holds the reference.
- **Redaction** *(built)*: before any event is appended, every string in it is scanned for the known values, raw, base64 and URL-encoded, and each occurrence becomes `[secret:<name>]`; a `SecretRedacted { name, count }` event follows (`shiioo-runtime::redact`). The office's API token, which the harness holds for its hook, is redacted the same way. Values shorter than six characters are not redacted.
- **What the model sees** *(partly)*: a harness runs its own tools, so a secret in its environment is visible to the agent that runs `env`; redaction protects the record, the phone and the notifications, not the model (T1 is bounded by the rules and by egress, not by hiding the value). In the native loop, where Shiioo runs the tools, values are to be injected at the tool boundary and never placed in model context *(planned)*.
- **At rest** *(built)*: `.shiioo/secrets.enc` is the name → value map sealed with XChaCha20-Poly1305 under a random key in `.shiioo/secrets.key` (mode 0600), written through a temporary file and a rename (`shiioo-secrets`). `shiioo secret set|list|rm` manage it; values are read from stdin, a variable or a file, never from the command line. *(planned)* Envelope encryption: the key wrapped by an `age` identity on the Mac, or by a KMS key where one exists, so rotation re-wraps the key without re-encrypting values.
- Claude Code's login for containers is one such secret: `claude setup-token`, then `shiioo secret set claude-code-oauth-token`.

### 3.3 Approval gates on side effects *(built for harness runs)*

- The default rule set asks before anything that leaves the machine or changes it outside the worktree (`git push`, `gh`, installs, unknown commands) and allows the rest; the owner loosens or tightens it per tool, command, path and repo in `rules.toml` (`default = "ask"` is the fallback). *(planned)* The native loop's built-in tools go through the same rules; today only `shell` is side-effecting and the native loop has no hook.
- Approval binds to the exact call (one `ApprovalRequested` per request, answered once), so an injected instruction cannot get one call approved and then make another (T1). *(planned)* An input digest in the request so the binding is checkable after the fact.
- *(deferred, ADR-0008)* Separation of duties: the user who started a run cannot approve its requests by default. With one owner there is no second person.

### 3.4 Sandboxing and egress *(`worktree` and `container` built; `openshell` experimental)*

Runs execute at one of these levels, chosen per office, per repository or per run ([positioning.md](../positioning.md#isolation-levels), [tools-and-machines.md §4](tools-and-machines.md#4-isolation-worktree-and-container-built-openshell-experimental), [ADR-0011](../adr/0011-containers-through-the-owners-engine.md)):

| Level | Isolation | Use | Status |
|---|---|---|---|
| `worktree` | own git worktree and branch; none beyond the node's OS user | trusted repos on the owner's Mac | built |
| `container` | OCI container per run (Docker or Podman): read-only root, no capabilities, `no-new-privileges`, pid limit, optional CPU and memory limits; only the worktree, the repository's `.git` and a per-run home mounted; egress through the proxy below | unattended work, repos whose build or tests should not touch the host | built |
| `openshell` | NVIDIA OpenShell sandbox: filesystem, syscall and network policy per run, generated from the egress rules | Linux nodes with a gateway | experimental, not verified against a gateway |

Earlier drafts called these `host`, `process`, `container` and `microvm`; `host` became `worktree`, `process` is covered by `openshell`, and `microvm` is *(deferred, ADR-0008)*.

**Egress allow lists** *(built for `container`)*: the harness container joins an internal network with no route out; a tinyproxy sidecar per run, on that network and on the default bridge, forwards TLS tunnels (port 443) and plain HTTP only to the patterns in `[egress] allow` (plus `also`), with the office's host always allowed for the hook. An injected "send this to evil.example" fails at the proxy even if the rule check was fooled (T1, T8). A program that ignores `HTTPS_PROXY` cannot connect at all. `egress.mode = "open"` removes the sidecar. For `openshell`, the same list becomes the sandbox's network policy. `worktree` runs are unrestricted: there is nothing to enforce it with on the host.

### 3.5 Tamper-evident log *(built: chain; planned: anchoring)*

- Every event is hash-chained to the previous one in its run ([event-log.md](event-log.md)). Edits, deletions, insertions and reordering inside a run are detectable by `verify_chain` *(built)*.
- The redb file is only ever appended to by the server (`shiioo-store`); the Postgres backend *(built, `postgres` feature; ADR-0010)* rejects `UPDATE`, `DELETE` and `TRUNCATE` on `events` with triggers. Either stops accidents and casual edits; neither stops someone with the file or a database superuser, which is why anchoring exists.
- **Signed checkpoints** *(planned)*: periodically (default every 5 minutes, and at every run completion) the server builds a Merkle tree over the latest hash of every run that changed, signs the root with the office's key, and appends the checkpoint to an office checkpoint log.
- **External anchoring** *(planned)*: checkpoint roots are published somewhere the operator does not control alone: a transparency log (Sigstore Rekor), a signed tag in a separate git repository, or an RFC 3161 timestamp authority. Verifiers check that a run's history is consistent with every anchored checkpoint (T6).

### 3.6 Machine and node identity *(planned with remote nodes)*

- Nodes authenticate with **mTLS client certificates** issued at enrollment (one-time enrollment token from the dashboard or CLI). Certificates are short-lived and rotated automatically over the existing connection.
- The node dials out ([ADR-0006](../adr/0006-daemon-dials-out-over-websocket.md)); machines expose no ports.
- A compromised machine can lie about tool results for tools it runs (T4). Mitigations: the blast radius is limited to runs scheduled on that machine; results are recorded with the machine id, so they can be found and re-evaluated after an incident; machines can be revoked instantly, which fences all their pending calls.

### 3.7 Tenant isolation *(deferred, ADR-0008)*

One office per server process is the design; there are no tenants to isolate. If several owners ever share one:

- Every row and every blob key carries the organization and project id; every query is scoped by them in the store layer, not in handlers.
- Biscuit tokens and API keys are bound to one organization.
- Machines belong to one project. Sandbox pools are per organization; microVMs are never reused across organizations (T7).

### 3.8 Admin and configuration audit *(planned)*

- Configuration changes go through change management and produce versioned, hash-chained records ([governance.md](governance.md)).
- Administrative actions (changing rules or employees, revoking machines, raising budgets, exporting data) are written to an office admin log with the same chaining and anchoring as run logs. Today `config.toml` and `rules.toml` are files; git history in the office directory is the audit, if the owner keeps one.

### 3.9 Client credentials *(one token built; the rest planned or deferred)*

- Today: one bearer token for the office ([api.md §6](api.md#6-authentication-one-token-built-the-rest-planned-or-deferred)), held by the dashboard, the bot process and the hook.
- *(planned)* API keys scoped by role and expiry, stored hashed. *(deferred, ADR-0008)* OIDC sign-in for several owners.
- *(planned)* Destructive or governance-relevant API calls (approving, revoking, changing rules) require a session with recent authentication (T9).

### 3.10 Supply chain *(planned)*

- Release binaries are built reproducibly in CI, signed (Sigstore), with an SBOM; `cargo deny` and `cargo audit` gate dependencies.
- MCP servers, skills and harnesses are pinned by version and digest in agent definitions. Changing them is a governed config change. Today the harness is whatever `claude` or `codex` is on the service's `PATH`; the version it reported is recorded in `HarnessSessionStarted`.
- Remote MCP servers are subject to the same egress and policy rules as any tool (T8).

## 4. What is not protected

Be explicit, so nobody relies on a guarantee that isn't there:

- **A fully compromised server** (all workers and the signing key) can stop writing events, refuse to serve them, or rewrite any history **after the last external anchor**. The anchoring interval bounds that window. History before the last anchor cannot be rewritten undetected. Today there is no anchor, so whoever controls the Mac controls the record.
- **Prompt injection is not solved.** The controls above limit what an injected agent can do (approvals today; scoped tokens, egress and secret redaction when built); they do not prevent the model from being persuaded. Rules and approvals are the line of defence for anything that matters.
- **A compromised machine controls its own tool results.** Shiioo records what the machine said, signed by the machine's identity once nodes exist; it cannot know whether it was true.
- **`worktree` offers no isolation** beyond a separate branch and directory: the agent has the owner's user, files and network. It is meant for the owner's own repositories on the owner's own machine. A `rm -rf ~` typed by a persuaded agent is stopped only by the rules asking first. Set `[isolation] default = "container"` for anything else.
- **`container` is a container, not a VM.** Root inside with no capabilities and a read-only root filesystem bounds mistakes and persuasion, not a kernel exploit. The worktree and the repository's `.git` are writable by design: the agent could rewrite branches there; the rules ask before `git push` and the branch is the owner's to review.
- **Secrets handed to a harness are visible to it.** Redaction keeps them out of the record; it does not keep the agent from reading its own environment.
- **Model providers see the context you send them.** Shiioo does not protect data from the provider you chose; use self-hosted models for data that must not leave.
- **Redaction is best-effort** for secrets that appear transformed in ways the scanner does not recognize, and does not apply to values shorter than six characters.

## 5. Security-relevant events

| Event | Meaning |
|---|---|
| `PolicyDecision` *(built)* | a rule decision, with the rule that decided and its reason (the policy set version, with Cedar) |
| `ApprovalRequested` / `ApprovalDecided` *(built)* | human gate on a call; `by` is the owner's surface or `timeout` |
| `CancelRequested` *(built)* | the owner stopped a run from the phone or the API |
| `InterruptRequested` *(built)* | the owner stopped a harness mid-step to deliver a message at once ("Now") |
| `SandboxStarted` *(built)* | where the harness executed: the level, and for `container` the image, container name, mounts and egress mode |
| `SecretRedacted` *(built)* | something about to be recorded contained a secret value; records the secret name and the count, not the value |
| `MachineRevoked` *(planned)* | appended to every run with pending calls on a revoked machine |
| checkpoint log entries *(planned)* | signed Merkle roots and their external anchors |
