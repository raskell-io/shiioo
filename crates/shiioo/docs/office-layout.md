# Office layout

An office is a directory. `shiioo --workspace <dir>` (default: the current
directory) keeps everything under `<dir>/.shiioo/`; delete that directory and
the office is gone, copy it and the office moves.

## `.shiioo/`

| Path | Written by | Contents |
| --- | --- | --- |
| `shiioo.redb` | every command | The event log, redb store. The only state. |
| `journal.jsonl` | `serve` | The local machine's tool journal (started/finished/fence records). |
| `worktrees/<run id>/` | harness runs with a repo | One git worktree per run, branch `shiioo/<8 chars>`. |
| `sandboxes/<run id>/` | `container` and `openshell` runs | `home/` (the container's `/root`, kept for resumes), `env` (the run's environment, mode 0600, removed at teardown), `egress/` (the proxy's configuration), `openshell-policy.yaml`. `sandboxes/build/` is the empty build context for the images. |
| `secrets.key`, `secrets.enc` | `shiioo secret` | The encrypted secret store, both mode 0600 ([shiioo-secrets](../../shiioo-secrets/README.md)). |
| `config.toml` | `serve` on first start | Office configuration, see below. |
| `rules.toml` | `serve` on first start | Permission rules; defaults from `RuleSet::defaults().to_toml()`. |
| `api-token` | `serve` on first start | Bearer token for the API and dashboard, 32 hex bytes, mode 0600. |
| `telegram-token` | `install --telegram-token` | Bot token, mode 0600. |
| `intake-state.json` | intake | Which issues were already picked up. |
| `logs/serve.log` | the installed service | stdout and stderr of `serve`. |

`shiioo run`, `resume` and `log` only touch `shiioo.redb`; they do not read
the config file or the rules.

## `config.toml`

Written with every section commented when missing:

```toml
[telegram]
# token = "..."            # or SHIIOO_TELEGRAM_TOKEN, or .shiioo/telegram-token
# owner_chat_id = 123456789 # or SHIIOO_TELEGRAM_OWNER; the bot obeys nobody else

[dashboard]
# url = "https://<machine>.<tailnet>.ts.net"   # notifications link here

[repos]
# shiioo = "/Users/me/code/shiioo"             # label = absolute path

[intake]
enabled = false
label = "shiioo"
interval_secs = 60

# [[employees]]
# name = "claude"
# kind = "claude-code"
# max_five_hour = 0.95
# max_seven_day = 0.90
# [[employees]]
# name = "codex"
# kind = "codex"
# [[employees]]
# name = "local"
# kind = "native"
# model = "ollama/qwen3:32b"
```

- **telegram.** Token resolution: `SHIIOO_TELEGRAM_TOKEN`, then
  `[telegram] token`, then `.shiioo/telegram-token`. Owner:
  `SHIIOO_TELEGRAM_OWNER`, then `owner_chat_id`. A token without an owner
  prints a warning and leaves the bot off; the bot never answers anyone but
  the owner.
- **dashboard.url.** Prefix for the links in Telegram messages
  (`<url>/#run=<id>`). Without it links use the listen address, which only
  works on the machine itself.
- **repos.** Label to absolute path. Labels are what `/run <repo> <task>`,
  the dashboard's repository field, `POST /v1/runs {"repo"}` and the intake
  use. `serve` warns about a path without a `.git` entry.
- **intake.** When `enabled`, every `interval_secs` (minimum 10) the office
  lists open issues labelled `label` in each configured repo through the
  `gh` CLI and starts a run for new ones. `gh` must be logged in to an
  account that can comment on the repos.
- **employees.** Who takes `employee: "auto"` tasks, in order, skipped when
  their subscription window is above the limits. Missing section: `claude`
  (Claude Code) then `codex`. Kinds: `claude-code`, `codex`, `native` with a
  `model` (routed through `shiioo-providers`, so `ollama/…` works).


### `[isolation]`, `[egress]`, `[env]`

```toml
[isolation]
default = "worktree"            # worktree | container | openshell
[isolation.repos]
myproject = "container"         # per repository label
[isolation.container]
image = ""                      # empty: Shiioo's own image, built on first use
engine = "docker"               # or podman
memory = "8g"
cpus = 4.0
mounts = ["/Users/me/data:ro"]  # extra host directories, same path inside
git_https = ["git@github-work:"]  # SSH aliases rewritten to https://github.com/
[isolation.openshell]
image = ""
cpus = 2.0
memory = "4Gi"

[egress]
mode = "allowlist"              # or open
# allow = [...]                 # replaces the built-in list
also = ["pypi.io"]              # adds to it

[env]
GH_TOKEN = "secret:gh-token"    # resolved from the secret store at harness start
```

`Config::isolation_policy()` turns these into `shiioo_api::IsolationPolicy`
(default level, per-repo levels, a `ContainerSpec` and an `OpenshellSpec`
that both carry the `[egress]` rules); `[env]` becomes `ApiOptions.env`,
copied into every harness run's spec as references. `shiioo run --isolation`
reads the same config.

## `rules.toml`

The permission rules, documented in `shiioo-rules`. First match wins,
otherwise the default. `serve` names the file in its `listening on …`
line. Terminal `shiioo run` does not apply rules (there is
no server for the hook to ask).

## What `serve` assembles

In order, from `fn serve`:

1. `stay_awake()` if `--keep-awake` (macOS only; `caffeinate -s -i -w <pid>`,
   killed with the process).
2. `Config::load` and `load_rules`, writing defaults when the files are
   missing.
3. `RedbStore::open(.shiioo/shiioo.redb)` wrapped in `Notifying` so SSE
   streams wake on every write.
4. The model provider for native runs: `shiioo_providers::from_env`, with
   `AnthropicProvider::from_env` when an Anthropic key is present or
   `UnconfiguredProvider` otherwise (native runs then fail with `no model
   provider configured …`; harness runs are unaffected).
5. `LocalMachine::open(workspace, .shiioo/journal.jsonl, Toolbox::builtin())
   .with_worktrees(.shiioo/worktrees)`.
6. `api_token(.shiioo/api-token)`, created on first start.
7. `HookEndpoint { server_url: http://<listen ip or 127.0.0.1>:<port>,
   token, exe: current_exe() }`: what workers pass to the harness.
8. A `CapacityTracker`; every worker gets an observer that feeds it.
9. The worker factory: `Worker::new(store, provider, machine,
   "local-<pid>").with_hooks(endpoint).with_observer(tracker)`.
10. The Telegram bot from the config (or off), `TelegramNotifications` with
    the dashboard url.
11. `ApiState::with_options` with the rules, employees, repos, tracker and
    notifications; default agent from `--model`, `--tools`, `max_turns` 50.
12. Intake, if enabled, with `IssueSink` turning issues into
    `assign("auto", repo)` runs.
13. `seed_capacity()` to replay stored `CapacityObserved` events.
14. The bot's long-polling loop and `axum::serve`, until Ctrl-C.

It then prints:

```
intake: watching 2 repo(s) for issues labelled shiioo     # only when enabled
telegram bot: on, owner chat 123456789
   # or: telegram bot: off (set SHIIOO_TELEGRAM_TOKEN and [telegram] owner_chat_id in …/config.toml)
listening on http://127.0.0.1:7878  (2 repo(s), rules: …/.shiioo/rules.toml)
dashboard: http://127.0.0.1:7878/#token=<contents of …/.shiioo/api-token>
```

The token itself is not printed by `serve`; `install` prints the full
dashboard link once.

## The Telegram office

`TelegramOffice` maps the bot's commands onto `ApiState`:

| Command | Does |
| --- | --- |
| `/run <repo> <task>`, `/run <task>`, plain text | `assign("auto", repo)` then `start_run`. With one configured repo it is used; with several and no label the bot asks `which repo? /run <repo> <task>, one of: …`. Replies `▶️ <id> started on <repo> (<employee>): <task>` and the branch. |
| `/status` | The 15 most recent runs, one line each with a glyph and the task (70 chars), then the capacity text. |
| `/capacity` | `CapacityTracker::text()`. |
| `/log <run>` | The last 25 events of a run; `<run>` may be a unique prefix of at least four characters. |
| `/cancel <run>` | `cancel(run, "owner:telegram", …)`; replies `🛑 <id> will stop at its next step`. |
| `/repos` | The configured labels and paths. |
| `/approve <id>`, `/deny <id>`, or the buttons | `decide(approval_id, …, by "owner:telegram")`. |
| `/help`, `/start` | The command list. |

`TelegramNotifications` sends `🔐 <id> wants to run\n<summary>` with
Allow/Deny buttons and a dashboard link, edits that message when the
approval is resolved or expires, and reports endings: `✅ finished` (with up
to 600 chars of output), `❌ failed`, `🛑 cancelled`, `⚠️ needs you`, `⏸
stopped without finishing`.

## The issue sink

With `[intake] enabled = true`, `IssueSink` receives each new labelled issue:

1. `repo_path(label)` for the issue's repository.
2. `assign("auto", Some(path))` picks the employee.
3. The task text is the intake's rendering of the issue; its branch
   placeholder is filled with `$(git branch --show-current)`, which only
   expands if the agent runs it in a shell.
4. `start_run`, then the intake comments on the issue with the run id and
   `shiioo/<8 chars>` branch.
5. When the run ends, `run_outcome` reports `Ok(output)` for completed runs
   and `Err(reason)` for failed, cancelled or needs-attention runs; open or
   waiting runs are not reported yet.

See also: [deployment design](../../../docs/design/deployment.md),
[telegram design](../../../docs/design/telegram.md),
[intake design](../../../docs/design/intake.md),
[approvals and capacity in shiioo-api](../../shiioo-api/docs/approvals-and-capacity.md).
