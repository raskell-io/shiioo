# shiioo (binary)

The single binary that is a Shiioo office. `shiioo serve` runs the API, the
dashboard, the Telegram bot, the GitHub issue intake and the workers in one
process; `shiioo install` makes that a background service that starts at
login and keeps the Mac awake; `shiioo run` drives one task from the terminal
without a server; `shiioo hook pre-tool-use` is what Claude Code and Codex
call to ask permission. There is one source file, `src/main.rs`, and no
library: all behaviour lives in the crates it wires together.

## Position in the crate graph

```
shiioo-core, shiioo-store (redb), shiioo-runtime, shiioo-api,
shiioo-rules, shiioo-providers, shiioo-intake, shiioo-telegram
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
| `Cli`, `Command`, `HookEvent` | clap definitions, see the command reference below. |
| `run`, `resume`, `log`, `drive` | Terminal commands against the local redb store; `drive` retries up to three times on a lost lease. |
| `serve` | Builds the whole office: store, machine, provider, API state, hook endpoint, capacity tracker, Telegram bot, intake. [docs/office-layout.md](docs/office-layout.md). |
| `Config`, `DEFAULT_CONFIG`, `load_rules`, `api_token` | `.shiioo/config.toml`, `.shiioo/rules.toml`, `.shiioo/api-token`. |
| `TelegramOffice` | Implements the bot's `Office` trait on top of `ApiState`: `/run`, `/status`, `/log`, `/cancel`, approvals. |
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
| `SHIIOO_CLAUDE_BIN`, `SHIIOO_CODEX_BIN` | harness adapters | Harness executables when not on `PATH`. |

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
'shiioo/*'`.

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
