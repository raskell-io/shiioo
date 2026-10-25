# shiioo (binary)

The single binary that is a Shiioo office. `shiioo serve` runs the API, the
dashboard, the Telegram bot, the GitHub issue intake and the workers in one
process; `shiioo install` makes that a background service that starts at
login and keeps the Mac awake; `shiioo run` drives one task from the terminal
without a server; `shiioo secret` manages the encrypted secret store;
`shiioo sandbox` builds the container images; `shiioo hook pre-tool-use` is
what Claude Code and Codex call to ask permission. There is one source file, `src/main.rs`, and no
library: all behaviour lives in the crates it wires together.

## Position in the crate graph

```
shiioo-core, shiioo-store (redb), shiioo-runtime, shiioo-api,
shiioo-rules, shiioo-providers, shiioo-intake, shiioo-telegram, shiioo-secrets
                          │
                          ▼
                    shiioo (binary)
```

Nothing depends on this crate. It owns the things that are policy rather
than mechanism: the data directory layout, the config file, how the Telegram
bot maps onto the API's library calls, how issues become runs, and the
service definitions for launchd and systemd.

## Map of `src/main.rs`

| Section | What it does |
| --- | --- |
| `Cli`, `Command`, `HookEvent`, `SecretAction`, `SandboxAction` | clap definitions, see the command reference below. |
| `run`, `resume`, `log`, `drive` | Terminal commands against the local redb store; `drive` retries up to three times on a lost lease. |
| `serve` | Builds the whole office: store, machine, provider, API state, hook endpoint, capacity tracker, Telegram bot, intake. [docs/office-layout.md](docs/office-layout.md). |
| `Config`, `DEFAULT_CONFIG`, `load_rules`, `api_token` | `.shiioo/config.toml` (including `[isolation]`, `[egress]`, `[env]` → `IsolationPolicy`), `.shiioo/rules.toml`, `.shiioo/api-token`. |
| `secret_command`, `sandbox_command`, `StoreSecrets` | The secret store CLI, image building and pruning, and the `SecretSource` the workers read. |
| `TelegramOffice` | Implements the bot's `Office` trait on top of `ApiState`: `/run` (with roles), `/status`, `/log`, `/cancel`, approvals, `/plan`, `/routines`, `/report`. |
| `GhIssues` | Implements `shiioo_api::IssueCreator` over the intake's `gh` wrapper: an approved plan's issues. |
| `TelegramNotifications` | Implements `shiioo_api::Notifications`: approval questions with Allow/Deny buttons, run endings. |
| `IssueSink` | Implements the intake's sink: a labelled GitHub issue becomes a run on that repository. |
| `install`, `uninstall`, `stay_awake` | launchd and systemd units, `caffeinate`. [docs/service.md](docs/service.md). |
| `hook`, `ask_server` | The permission hook client. [docs/hook.md](docs/hook.md). |

## Command reference

Global: `--workspace, -w <dir>` (default `.`). The workspace is the office's
root; all state lives in `<workspace>/.shiioo/`.

### `shiioo run <task>`

Create a run and drive it in this process until it finishes.

| Flag | Default | Meaning |
| --- | --- | --- |
| `--harness claude-code\|codex` | none (native loop) | Run the task with an external harness using its own login. |
| `--repo <path>` | none | Git repository for the harness; the run gets a worktree under `.shiioo/worktrees` and a `shiioo/<run>` branch. Requires `--harness`. |
| `--model <name>` | native `claude-opus-5-5`; harness: its own default | Model, passed to the harness as `--model`/`-m`. |
| `--tools a,b` | native `read_file,write_file,shell`; harness `Read,Edit,Write,Glob,Grep,Bash(git *)` | Native: Shiioo tool names. Harness: its allow-list syntax. |
| `--system <text>` | `""` | Extra instructions. |
| `--max-turns <n>` | `50` | Model turns before the run fails. |
| `--effort low\|medium\|high\|xhigh\|max` | `high` | Reasoning effort for native runs. |
| `--permission-mode <mode>` | harness default (`acceptEdits`, Codex `danger-full-access`) | Override the harness permission mode. Requires `--harness`. |
| `--api-billing` | off | Bill to `ANTHROPIC_API_KEY` instead of the subscription. Requires `--harness`. |
| `--max-budget-usd <n>` | none | Stop at this spend (API billing, Claude Code only). Requires `--harness`. |
| `--max-minutes <n>` | none | Fail the run after this long across resumes. Requires `--harness`. |
| `--isolation worktree\|container\|openshell` | the office's policy (`[isolation]` in `config.toml`) | Where the harness runs. Requires `--harness`. The level's settings and `[env]` come from `config.toml`. |
| `--role <name>` | none | Run as a configured role (`[[roles]]`): its instructions, tools, model, isolation and limits over the harness's defaults. Requires `--harness`. |

Prints `run <id>` and, for repo runs, `branch <name> in <path>`, then every
event as it happens. Exit codes: `0` completed, `1` failed, `2` needs
attention, `3` lost the lease three times, `4` cancelled.

Terminal runs have no hook endpoint, so the harness runs in its own
permission mode without Shiioo's rules. Rules and approvals need a server.

### `shiioo resume <run> [--effort high]`

Drive an existing run again, for example after the process was killed. A
harness run resumes its own session with the continue prompt.

### `shiioo log <run>`

Print every event as `#<seq> <description>` and verify the hash chain:
`chain verified: N events` or `CHAIN BROKEN`.

### `shiioo serve` (alias `shiioo up`)

| Flag | Default | Meaning |
| --- | --- | --- |
| `--listen <addr>` | `127.0.0.1:7878` | Loopback by default; reach it from the phone over Tailscale. |
| `--keep-awake` | off | macOS `caffeinate -s -i` for the lifetime of the process. The installed service sets it. |
| `--model <name>` | `claude-opus-5-5` | Default agent model for runs that name no employee. |
| `--tools a,b` | `read_file,write_file,shell` | Default agent tools. |
| `--effort <level>` | `high` | Native provider effort. |

Prints the Telegram state, `listening on http://<addr>` and the dashboard
URL with the path of the token file to paste into it. Stops on Ctrl-C.

### `shiioo install` / `shiioo uninstall`

| Flag | Default | Meaning |
| --- | --- | --- |
| `--listen <addr>` | `127.0.0.1:7878` | Baked into the service. |
| `--telegram-token <token>` | `$SHIIOO_TELEGRAM_TOKEN` if set | Stored in `.shiioo/telegram-token` (mode 0600). |
| `--print` | off | Print the unit instead of installing it. |

`uninstall` removes the service and leaves `.shiioo/` alone.

### `shiioo secret set|list|rm`

| Command | Meaning |
| --- | --- |
| `secret set <name>` | Store a secret read from stdin (`echo -n … \| shiioo secret set gh-token`), `--from-env VAR` or `--from-file PATH`. Never from the command line. |
| `secret list` | The names, never the values. |
| `secret rm <name>` | Remove one. |

Names are `[a-z0-9][a-z0-9._-]*`. Refer to a secret as `secret:<name>` in
`[env]`. The secret `claude-code-oauth-token` is what Claude Code uses in a
container (`claude setup-token` makes one). See
[shiioo-secrets](../shiioo-secrets/README.md).

### `shiioo sandbox build|prune`

`build` builds the harness image (`shiioo-harness:<hash>`, from the
Dockerfile embedded in `shiioo-runtime`) and the egress image now, and
creates the `shiioo-sandbox` network, so the first `container` run does not
wait minutes. `prune` removes containers named `shiioo-*` left behind by
runs that did not clean up. Both need the engine running (`docker` by
default, `[isolation.container] engine`).

### Roles, routines, plans and the report

There are no commands for these besides `shiioo run --role <name>`; they
live in `config.toml` and on the phone (or the API):

| Config | Phone | Meaning |
| --- | --- | --- |
| `[[roles]]` | `/run @role …`, `/roles` | Reusable agent configurations: employee, instructions, tools, model, isolation, limits. |
| `[[routines]]` | `/routines`, `/routine <name>` | Runs on a schedule (`hourly`, `every 6h`, `daily 02:00`, `weekdays 07:00`, `mon,thu 09:30`). State in `.shiioo/routines-state.json`. |
| `[report] schedule` | `/report` | The morning report: counted from the record and the repositories. |
| (none) | `/plan <repo> <goal>`, `/plan approve <id>` | The chief of staff: a read-only planning run; approval creates labelled issues for the intake. Plans in `.shiioo/plans.json`. |

### `shiioo hook pre-tool-use` (hidden)

Called by the harness, never by people. Reads the hook event JSON on stdin,
asks the server, prints the decision. See [docs/hook.md](docs/hook.md).

## Environment variables

| Variable | Read by | Purpose |
| --- | --- | --- |
| `SHIIOO_TELEGRAM_TOKEN` | `serve`, `install` | Bot token (also `[telegram] token` or `.shiioo/telegram-token`). |
| `SHIIOO_TELEGRAM_OWNER` | `serve` | Owner chat id (also `[telegram] owner_chat_id`). |
| `SHIIOO_SERVER_URL`, `SHIIOO_API_TOKEN`, `SHIIOO_RUN_ID` | `hook` | Set by the worker in the harness's environment. |
| `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL` | native provider | Only native runs; harness runs use the harness login. |
| `OPENAI_API_KEY`, `OLLAMA_*`, … | `shiioo-providers` | Routed native models (`ollama/…`, `lmstudio/…`, `openai/…`). |
| `SHIIOO_CLAUDE_BIN`, `SHIIOO_CODEX_BIN` | harness adapters | Harness executables when not on `PATH` (host runs; a container uses its image's). |
| `SHIIOO_OPENSHELL_BIN` | `openshell` level | The OpenShell CLI when not on `PATH`. |
| `CLAUDE_CODE_OAUTH_TOKEN` | `container` level | Claude Code's token for containers when the secret `claude-code-oauth-token` is not set. |

## Testing

The binary has no tests of its own; the behaviour it wires is tested in
`shiioo-api` (end to end over HTTP with a fake harness) and `shiioo-runtime`.

```sh
cargo test --workspace
cargo run -p shiioo -- --workspace /tmp/office serve      # smoke test
curl -s http://127.0.0.1:7878/v1/health
```

A real end-to-end check needs an installed `claude` or `codex` logged in to
the owner's subscription: `shiioo run --harness claude-code --repo . "say
hello in HELLO.md"`, then `shiioo log <run>` and `git branch --list
'shiioo/*'`. For the `container` level add `--isolation container` (Docker
running, the office under your home directory); the first run builds the
image.

## Further reading

- [docs/office-layout.md](docs/office-layout.md): the `.shiioo/` directory,
  `config.toml`, `rules.toml`, tokens, what `serve` assembles.
- [docs/service.md](docs/service.md): launchd and systemd units, keep-awake,
  logs, Tailscale.
- [docs/hook.md](docs/hook.md): the permission hook end to end.
- Top-level: [positioning](../../docs/positioning.md),
  [architecture](../../docs/architecture.md),
  [deployment design](../../docs/design/deployment.md),
  [telegram design](../../docs/design/telegram.md),
  [intake design](../../docs/design/intake.md),
  [ADR-0010 single binary](../../docs/adr/0010-single-binary-server-and-nodes.md).
- Library crates: [shiioo-api](../shiioo-api/README.md),
  [shiioo-runtime](../shiioo-runtime/README.md).
