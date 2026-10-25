# Sandboxes: the isolation levels

Where a harness process runs. The worker builds the harness command as if it
ran on the host (`HarnessDriver::command`), then hands it to a `Sandbox`
(`sandbox/mod.rs`), which runs it as is, or translates it into `docker run …`
or `openshell sandbox exec …`. The sandbox also rewrites the hook endpoint
for use from inside and describes itself in the run's `SandboxStarted` event.
Design: [tools-and-machines.md §4](../../../docs/design/tools-and-machines.md#4-isolation-worktree-and-container-built-openshell-experimental),
[ADR-0011](../../../docs/adr/0011-containers-through-the-owners-engine.md).

```rust
pub enum Sandbox { Worktree { env, hooks, detail }, Container(ContainerSandbox), Openshell(OpenshellSandbox) }

impl Sandbox {
    pub async fn prepare(isolation: &Isolation, ctx: &SandboxContext<'_>) -> Result<Self, SandboxError>;
    pub fn hooks(&self) -> Option<HookEndpoint>;   // as the harness must use it from inside
    pub fn wrap(&self, cmd: Command) -> Command;    // the command that runs it at this level
    pub fn started(&self) -> Payload;               // SandboxStarted { level, detail }
    pub async fn teardown(&self);                   // containers, proxies; the worktree and home stay
}
```

`SandboxContext` carries the run id, the harness kind, the working
directory (worktree or workspace), the repository, the sandbox root
(`.shiioo/sandboxes`, from `LocalMachine::with_sandboxes`), the host's hook
endpoint, the resolved environment, the secret source and whether the spec
named its own binary.

The worker (`Session::run_harness`): resolve `env` (secret references) →
`Sandbox::prepare` → `driver.command(…, sandbox.hooks())` → `sandbox.wrap`
→ append `SandboxStarted` → run the process, redacting every batch → `teardown`.
A failure in `prepare` fails the run with the level's name and the reason
(`RunFailed`), so the owner sees it on the phone.

## `worktree`

The host process, the owner's user. `wrap` only adds the resolved
environment. The hook is `<shiioo> hook pre-tool-use`.

## `container` (`sandbox/container.rs`)

```
docker run --rm --name shiioo-<run8> --hostname shiioo
    --read-only --tmpfs /tmp:rw,exec,size=4g --tmpfs /run:rw,size=64m
    --cap-drop ALL --security-opt no-new-privileges --pids-limit 2048
    --add-host host.docker.internal:host-gateway
    [--memory <m>] [--cpus <n>]
    --mount type=bind,src=<worktree>,dst=<worktree>
    --mount type=bind,src=<repo>/.git,dst=<repo>/.git
    --mount type=bind,src=.shiioo/sandboxes/<run>/home,dst=/root
    [--mount type=bind,src=~/.codex/auth.json,dst=/root/.codex/auth.json,readonly]
    [--mount …,readonly]                          # ContainerSpec.mounts, same path inside
    [--network shiioo-sandbox]                    # unless egress is open
    -w <worktree> --env-file .shiioo/sandboxes/<run>/env
    <image> <claude|codex> <harness args…>
```

- **Paths.** The worktree and the repository's `.git` are mounted at their
  host paths, so the worktree's `.git` file (an absolute pointer into
  `<repo>/.git/worktrees/<run>`) resolves inside. Both are writable: that is
  where the agent commits. The engine must be able to see them: with colima
  or Docker Desktop only shared directories (the home directory by default)
  can be mounted. `check_mountable` runs `docker run … --mount … true` first
  and turns the engine's "bind source path does not exist" into a message
  that says so.
- **Home.** `/root` is `.shiioo/sandboxes/<run>/home`, created on first
  start and kept: Claude Code's `~/.claude/projects/<cwd>/<session>.jsonl`
  and Codex's `~/.codex/sessions` live there, so `--resume` and
  `exec resume` work after a crash. The process runs as root because colima
  lets only root write to a bind mount; root with no capabilities is an
  ordinary user to the host, and files it creates belong to the owner.
- **Environment.** Everything the driver set (`SHIIOO_*`,
  `CLAUDE_CODE_RESUME_INTERRUPTED_TURN`), the resolved `env`, `HOME=/root`,
  the proxy variables and `GIT_CONFIG_*` rewrites go through an env file
  (mode 0600) in the run's sandbox directory, deleted at teardown. Nothing
  from the host environment is inherited: `docker run` starts clean. The
  `env_remove` calls of the drivers (session variables, API keys) are
  therefore moot inside.
- **Binary.** Unless the spec names a `binary`, the image's `claude` or
  `codex` runs; the host's path or `SHIIOO_*_BIN` would mean nothing inside.
  A named binary (a test double, a wrapper) is passed through and is
  expected to bring its own login.
- **Credentials.** Codex: `~/.codex/auth.json` (or `$CODEX_HOME/auth.json`)
  mounted read-only; missing means a `Credentials` error. Claude Code:
  `CLAUDE_CODE_OAUTH_TOKEN` from the `env`, else the secret
  `claude-code-oauth-token`, else the server's own environment, else a
  Linux host's `~/.claude/.credentials.json` mounted read-only; nothing
  means a `Credentials` error naming `claude setup-token` and
  `shiioo secret set`.
- **Hook.** `HookCommand::Curl`: `curl -sSf -X POST … "$SHIIOO_SERVER_URL/v1/runs/$SHIIOO_RUN_ID/hook/pre-tool-use" || printf '<deny>'`,
  with `SHIIOO_SERVER_URL` rewritten to `http://host.docker.internal:<port>`.
  The endpoint answers in the hook's own output format. Through the proxy
  (lower-case `http_proxy`, which curl honours; it ignores upper-case
  `HTTP_PROXY` by design) the office's host is always on the allow list.
- **Image.** `default_image()` is `shiioo-harness:<12 hex of the Dockerfile's
  SHA-256>`; `ensure_image` builds it with `docker build -f - <sandbox dir>`
  (the Dockerfile on stdin, an empty context) when missing, printing a note
  to stderr: the first run takes minutes. `ContainerSpec.image` names
  another image, which the engine pulls if needed.
- **Git remotes.** The container has no SSH keys. The image rewrites
  `git@github.com:` and `ssh://git@github.com/` to `https://github.com/`;
  `ContainerSpec.git_https` adds the owner's aliases (`git@github-work:`)
  as `GIT_CONFIG_*` `insteadOf` entries. Pushes authenticate through
  `gh auth git-credential` with `GH_TOKEN` from the `env`.
- **Teardown.** `docker rm -f` on the harness container (killing the
  `docker run` client does not stop the container) and on the proxy, and
  the env file is removed. `Drop` spawns the same `rm -f` without waiting,
  for a worker that dies without teardown. `shiioo sandbox prune` removes
  anything named `shiioo-*` that was left.

### Egress (`sandbox/egress.rs`)

With `Egress::Allowlist { hosts }` (the default):

1. `docker network create --internal shiioo-sandbox` once. An internal
   network has no gateway: no host, no internet.
2. Per run: `tinyproxy.conf` and `filter` are written to
   `.shiioo/sandboxes/<run>/egress/` and a sidecar `shiioo-egress-<run8>`
   starts from `shiioo-egress:<hash>` (Alpine + tinyproxy, built on first
   use) with that directory mounted read-only at `/etc/tinyproxy`,
   read-only root, no capabilities, on the internal network; then
   `docker network connect bridge` gives it the way out. If it is not
   running right after start, its log becomes the error.
3. The harness container joins the internal network with
   `HTTPS_PROXY`/`HTTP_PROXY`/`https_proxy`/`http_proxy` set to
   `http://shiioo-egress-<run8>:3128` (Docker's embedded DNS resolves the
   name on a user-defined network).

tinyproxy: `FilterDefaultDeny Yes`, `FilterType fnmatch` (so `*.github.com`
works), `ConnectPort 443` (TLS tunnels to 443 only), plain HTTP to allowed
hosts, `host.docker.internal` always allowed. A refused request is answered
`403` and logged as "Proxying refused on filtered domain". `Egress::Open`
puts the container on the default bridge with no proxy.

`Egress::default_hosts()`: `api.anthropic.com`, `statsig.anthropic.com`,
`api.openai.com`, `chatgpt.com`, `auth.openai.com`, `github.com`,
`*.github.com`, `*.githubusercontent.com`, `registry.npmjs.org`,
`crates.io`, `*.crates.io`, `pypi.org`, `files.pythonhosted.org`,
`deb.debian.org`, `security.debian.org`, `proxy.golang.org`,
`sum.golang.org`.

## `openshell` (`sandbox/openshell.rs`)

Experimental: written against the OpenShell documentation (sandbox
lifecycle, policy schema v1), exercised only on the commands it builds.

1. `openshell --version` must work (`SHIIOO_OPENSHELL_BIN` overrides the
   binary); otherwise `Unavailable`.
2. A policy is written to `.shiioo/sandboxes/<run>/openshell-policy.yaml`:
   `version: 1`, `network_policies` with the office's host and port and one
   endpoint per allowed host on 443, all `enforcement: enforce`, binaries
   `/**`. Filesystem and process settings are left to the gateway's defaults.
3. `openshell sandbox create --name shiioo-<run8> --policy <file> --detach
   --no-auto-providers [--from <image>] [--cpu n] [--memory m]
   --upload <worktree>:/workspace [--upload ~/.codex/auth.json:/root/.codex/auth.json]
   -- sleep infinity`.
4. `wrap`: `openshell sandbox exec -n <name> --workdir /workspace
   --env K=V … -- <claude|codex> <args>`.
5. `teardown`: `openshell sandbox delete <name>`.

Because the upload is a copy, the agent's work does not land in the local
worktree; the task should end with a push and the egress rules must allow
the remote. Claude Code needs the OAuth token secret here too.

## Testing

- Unit tests in `sandbox/mod.rs`, `container.rs`, `egress.rs` and
  `openshell.rs` check the URL rewriting, the exact `docker run` arguments
  and env file, the tinyproxy configuration, the policy YAML and the
  OpenShell commands, without an engine.
- `tests/container.rs` runs against a real Docker daemon, ignored by
  default: `cargo test -p shiioo-runtime --test container -- --ignored`. One
  test runs a fake harness in `alpine:3` with the worktree mounted and a
  secret injected and redacted; the other runs it in Shiioo's harness image
  behind an allow list and checks that `api.github.com` and the office (a
  stand-in HTTP server on the host's loopback) are reachable through the
  proxy and `example.com` is not. Temporary directories go under
  `~/.cache/shiioo/tests` (or `SHIIOO_TEST_DIR`), because the system temp
  directory is not shared with the engine's VM.
