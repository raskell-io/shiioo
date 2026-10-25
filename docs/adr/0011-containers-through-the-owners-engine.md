# 0011. Isolation through the owner's container engine, egress through a proxy sidecar

- Status: Accepted
- Date: 2026-10-25

## Context

Phase 4 ("unattended safety") needs harness runs that cannot touch the owner's machine beyond their worktree, cannot reach arbitrary hosts, and can hold a credential (a GitHub token) without it ending up in the record. [ADR-0010](0010-single-binary-server-and-nodes.md) rules out containers *for Shiioo itself*; this decision is about containers *for the agents*, on the owner's Mac today.

Forces:

- Claude Code and Codex are Node programs with their own login state. Inside a Linux container they need credentials handed in: Codex keeps a file, Claude Code on macOS keeps its login in the keychain, which no container can read.
- The owner already runs Docker (colima) or can install Docker Desktop or Podman. Writing our own sandbox (Seatbelt, Landlock) would mean one implementation per OS and no network control.
- Under colima, bind mounts are only writable by root inside the container (host files appear owned by the VM's user mapping), and only shared directories (the home directory by default) are visible at all.
- Network egress cannot be enforced by environment variables alone: a process that ignores `HTTPS_PROXY` must still be unable to connect.
- The permission hook is a `shiioo` subcommand on the host. The container has no `shiioo` binary, and cross-compiling one per run would make the first run slow and fragile.
- NVIDIA OpenShell offers kernel-level filesystem, syscall and network policy per sandbox, with a gateway and its own CLI. It targets Linux hosts and Docker Desktop; its documentation describes uploads, not live mounts.

## Decision

1. **`container` runs through the engine the owner has** (`docker`, or `podman` by name), via its CLI: `docker run --rm --read-only --cap-drop ALL --security-opt no-new-privileges --pids-limit …`, with the run's worktree and the repository's `.git` bind-mounted at their host paths, a per-run home directory under `.shiioo/sandboxes/<run>/home` that persists for resumes, and optional CPU and memory limits. The process inside runs as root: with no capabilities it is an ordinary user to the host, and it is the only user colima lets write to a bind mount.
2. **One image, built on first use** from a Dockerfile embedded in the binary (Node, git, the GitHub CLI, build tools, Claude Code and Codex from npm), tagged by the Dockerfile's hash. The owner may name another image.
3. **Egress is an allow list enforced by a proxy sidecar**: the harness container joins an internal Docker network with no route out; a per-run tinyproxy container on both that network and the default bridge forwards TLS tunnels (443) and plain HTTP only to allowed host patterns. The office's host is always allowed for the hook. `egress = open` skips the sidecar. The list defaults to the model providers, GitHub and the common package registries.
4. **The hook inside a sandbox is `curl`** against a new endpoint, `POST /v1/runs/{id}/hook/pre-tool-use`, which speaks the hook's own dialect (hook stdin in, hook stdout out). The office is reached as `host.docker.internal`. If the office cannot be reached the printed decision is a deny.
5. **Credentials**: Codex's `auth.json` is mounted read-only; Claude Code gets `CLAUDE_CODE_OAUTH_TOKEN` from the office's secret store (`claude setup-token`, then `shiioo secret set claude-code-oauth-token`), because its keychain login cannot cross into a container. A spec that names its own binary needs neither.
6. **Secrets** live in `.shiioo/secrets.enc`, sealed with XChaCha20-Poly1305 under a random key in `.shiioo/secrets.key` (mode 0600). Specs reference them as `secret:<name>`; the worker resolves them when the harness starts, and the log keeps the reference. Every string about to be recorded is scanned for the known values (raw, base64, URL-encoded) and each occurrence becomes `[secret:<name>]`, with a `SecretRedacted` event naming the secret. The office's own API token is redacted the same way.
7. **`openshell` is an adapter over the `openshell` CLI** (create with an uploaded copy of the worktree and a generated network policy, exec the harness, delete), marked experimental: it is not exercised against a gateway in this repository, and because the upload is a copy the agent's work comes back through git, not the local worktree.
8. The level is chosen per run from the office's policy: a default, a per-repository override, or an explicit request (`--isolation`, the API's `isolation`).

## Consequences

Easier: unattended runs on repositories the owner would not hand to an agent with their user's rights; a leaked token in a command's output does not reach the phone or the log; a run that is persuaded to "send this to evil.example" fails at the proxy. The first `container` run costs an image build (minutes); later ones start in seconds.

Harder or given up:

- The agent inside sees the secrets it is given (they are in its environment); redaction protects the record, not the model. Secrets never reaching the model remains the design goal for the native loop, where Shiioo runs the tools.
- Only TLS on 443 and plain HTTP leave a sandboxed container. `git` over SSH does not; remotes are rewritten to HTTPS inside the container and pushes use the GitHub CLI's token.
- Programs that honour no proxy still cannot connect, which is the point, but means some tools need their proxy settings spelled out.
- The office and its repositories must live under a directory the engine shares with its VM (the home directory by default); a preflight mount check says so when they do not.
- Redaction is best effort (ADR-0010's single machine means a compromised engine is a compromised office anyway).
- Resource limits and a read-only root are not a security boundary against a kernel exploit; they are against mistakes and persuasion, which is the threat model of [security.md](../design/security.md).

## Revisit when

- Apple's `container` tool or Podman on macOS allows non-root writes to bind mounts: then the process inside should run as the owner's uid.
- OpenShell gains live mounts or a stable local gateway on colima: then it can take over `container`'s role on Linux nodes (phase 6) and be verified here.
- A second machine joins (phase 6): the proxy and image management move to the node, and egress rules become part of the machine's capabilities.
