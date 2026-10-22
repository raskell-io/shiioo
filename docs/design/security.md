# Security

Agents act with real credentials on real systems, and they read untrusted text all day. This document states what Shiioo defends against, how, and, just as important, what it does not defend against.

Related: [governance.md](governance.md) (policies, approvals), [tools-and-machines.md](tools-and-machines.md) (daemon and sandboxes), [event-log.md](event-log.md) (hash chain), [ADR-0005](../adr/0005-cedar-policies-and-biscuit-credentials.md), [ADR-0006](../adr/0006-daemon-dials-out-over-websocket.md).

## 1. Assets

| Asset | Why it matters |
|---|---|
| Credentials and secrets | API keys, deploy keys, cloud credentials the agents use |
| Machines and workspaces | source code, data, anything the daemon can reach |
| The event log | the record of what happened; its integrity is the audit |
| Configuration | policies, boards, agent definitions: changing them changes what agents may do |
| Model and tool spend | budgets are money |

## 2. Threat model

| # | Threat | Example |
|---|---|---|
| T1 | **Prompt injection** through tool outputs | a web page, issue comment, README or email tells the agent to exfiltrate a secret or push to main |
| T2 | **Malicious or compromised agent definition** | instructions or a skill crafted to abuse tools |
| T3 | **Runaway agent** | loops, fork bombs of subagents, budget exhaustion |
| T4 | **Compromised machine** | the daemon host is taken over; it can lie about tool results and see its workspace |
| T5 | **Compromised worker** | an attacker controls a worker process: it holds a lease and can call models and dispatch tools |
| T6 | **Insider with database access** | edits or deletes events to hide what happened |
| T7 | **Cross-tenant access** | one organization reads or affects another's runs |
| T8 | **Supply chain** | a compromised dependency, MCP server, skill or harness |
| T9 | **Stolen client credentials** | a leaked API key or session token |

## 3. Controls

### 3.1 Least privilege by construction *(planned)*

- Every run gets a **Biscuit token** minted by the server, scoped to its project, agent, run and the tools and resources its policy allows. Child runs get a token **attenuated** from the parent's: Biscuit attenuation can only remove rights, offline, so a subagent can never do more than its parent (T2, T3, T5).
- The daemon checks the token on every tool invocation, independently of the worker. A compromised worker cannot use a machine it was not authorized for (T5).
- Tokens are short-lived (default: lease TTL plus margin) and bound to the run id.

### 3.2 Secrets never reach the model *(planned)*

- Secrets are referenced by name in tool configuration (`env: { GITHUB_TOKEN: secret("gh-deploy") }`) and resolved on the machine or in the worker at the tool boundary, after the policy check.
- Secret values are **never** placed in model context, tool inputs the model sees, or event payloads.
- Tool outputs are scanned for known secret values (exact and common encodings: base64, URL-encoded) and redacted before they are recorded or returned to the model (T1).
- Secrets are encrypted at rest with **envelope encryption**: a per-project data key encrypts the values; the data key is wrapped by a KMS key (AWS KMS, GCP KMS, Vault Transit) or, for on-device and self-hosting, by an `age` identity. Rotation re-wraps data keys without re-encrypting values.

### 3.3 Approval gates on side effects *(planned)*

- The default policy template requires approval for every `side_effecting` tool. Projects loosen it deliberately.
- Approval binds to the exact call (tool, input digest, resource), so an injected instruction cannot get one call approved and then make another (T1).
- Separation of duties: the user who started a run cannot approve its requests by default.

### 3.4 Sandboxing and egress *(planned)*

Machines run tools at one of these levels, set per machine and enforceable by policy:

| Level | Isolation | Use |
|---|---|---|
| `host` | none beyond the daemon's OS user | trusted personal machine, `shiioo up` |
| `process` | Landlock (filesystem), seccomp (syscalls), separate user, no access outside the workspace | default on Linux machines |
| `container` | OCI container per run with read-only root | shared servers |
| `microvm` | Firecracker microVM per run | untrusted code, multi-tenant pools |

**Egress allowlists** per machine and per policy (hosts and ports) are enforced by the daemon (proxy for HTTP, network namespace rules for `container` and `microvm`). An injected "send this to evil.example" fails at the network layer even if the policy check was fooled (T1, T8).

### 3.5 Tamper-evident log *(built: chain; planned: anchoring)*

- Every event is hash-chained to the previous one in its run ([event-log.md](event-log.md)). Edits, deletions, insertions and reordering inside a run are detectable by `verify_chain` *(built)*.
- The Postgres backend rejects `UPDATE`, `DELETE` and `TRUNCATE` on `events` with triggers *(built)*. This stops accidents and casual edits; it does not stop a database superuser, which is why anchoring exists.
- **Signed checkpoints** *(planned)*: periodically (default every 5 minutes, and at every run completion) the server builds a Merkle tree over the latest hash of every run that changed, signs the root with the deployment's key, and appends the checkpoint to a project-level checkpoint log.
- **External anchoring** *(planned)*: checkpoint roots are published somewhere the operator does not control alone: a transparency log (Sigstore Rekor), a signed tag in a separate git repository, or an RFC 3161 timestamp authority. Verifiers check that a run's history is consistent with every anchored checkpoint (T6).

### 3.6 Machine and daemon identity *(planned)*

- Daemons authenticate with **mTLS client certificates** issued at enrollment (one-time enrollment token from the dashboard or CLI). Certificates are short-lived and rotated automatically over the existing connection.
- The daemon dials out ([ADR-0006](../adr/0006-daemon-dials-out-over-websocket.md)); machines expose no ports.
- A compromised machine can lie about tool results for tools it runs (T4). Mitigations: the blast radius is limited to runs scheduled on that machine; results are recorded with the machine id, so they can be found and re-evaluated after an incident; machines can be revoked instantly, which fences all their pending calls.

### 3.7 Tenant isolation *(planned)*

- Every row and every blob key carries the organization and project id; every query is scoped by them in the store layer, not in handlers.
- Biscuit tokens and API keys are bound to one organization.
- Machines belong to one project. Sandbox pools are per organization; microVMs are never reused across organizations (T7).

### 3.8 Admin and configuration audit *(planned)*

- Configuration changes go through change management and produce versioned, hash-chained records ([governance.md](governance.md)).
- Administrative actions (adding users, changing roles, revoking machines, raising budgets, exporting data) are written to a per-organization admin log with the same chaining and anchoring as run logs.

### 3.9 Client credentials *(planned)*

- Humans sign in with OIDC; API keys are scoped (project, role, expiry) and stored hashed.
- Destructive or governance-relevant API calls (approving, revoking, changing policy) require a session with recent authentication (T9).

### 3.10 Supply chain *(planned)*

- Release binaries are built reproducibly in CI, signed (Sigstore), with an SBOM; `cargo deny` and `cargo audit` gate dependencies.
- MCP servers, skills and harnesses are pinned by version and digest in agent definitions. Changing them is a governed config change.
- Remote MCP servers are subject to the same egress and policy rules as any tool (T8).

## 4. What is not protected

Be explicit, so nobody relies on a guarantee that isn't there:

- **A fully compromised server** (all workers and the signing key) can stop writing events, refuse to serve them, or rewrite any history **after the last external anchor**. The anchoring interval bounds that window. History before the last anchor cannot be rewritten undetected.
- **Prompt injection is not solved.** The controls above limit what an injected agent can do (scoped tokens, approvals, egress, secret redaction); they do not prevent the model from being persuaded. Policies and approvals are the line of defence for anything that matters.
- **A compromised machine controls its own tool results.** Shiioo records what the machine said, signed by the machine's identity; it cannot know whether it was true.
- **`host` sandbox level offers no isolation.** It is meant for your own laptop.
- **Model providers see the context you send them.** Shiioo does not protect data from the provider you chose; use self-hosted models for data that must not leave.
- **Redaction is best-effort** for secrets that appear transformed in ways the scanner does not recognize.

## 5. Security-relevant events

| Event | Meaning |
|---|---|
| `PolicyDecision` | an authorization decision with the policy version |
| `ApprovalRequested` / `ApprovalDecided` | human gate on a call |
| `SecretRedacted` | a tool output contained a secret value; records the secret name, not the value |
| `MachineRevoked` | appended to every run with pending calls on a revoked machine |
| checkpoint log entries | signed Merkle roots and their external anchors |
