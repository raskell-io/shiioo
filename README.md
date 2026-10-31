<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/shiioo-mark-dark.svg">
    <img src="assets/brand/shiioo-mark-light.svg" alt="Shiioo: Virtual Company OS" width="560">
  </picture>
</p>

<h1 align="center">Shiioo</h1>

<p align="center">
  <strong>Virtual Company OS.</strong> A staff of AI agents that work on your own machines around the clock, which you direct and supervise from your phone.<br>
  One Rust binary. Every decision on the record.
</p>

Shiioo runs coding agents you already pay for (Claude Code on a Claude subscription, Codex on an OpenAI subscription, local models on your hardware) as employees of a small virtual company whose only human is you. You hand out work from Telegram or a labelled GitHub issue, the agents work in their own git worktrees, your rules decide what they may do on their own, the rest comes to your phone as an Allow / Deny question, and everything any agent did is in a tamper-evident log you can read live.

It is built for one owner with an always-on machine (a Mac Studio, say), not for teams or tenants. The technical term is "agent platform"; the product is a Virtual Company OS. See [What Shiioo is](docs/positioning.md).

## How it works

- **Employees are harnesses.** Claude Code and Codex run headless in a worktree of your repository, on your subscriptions; nothing is billed to an API key unless you ask for it. Local models (Ollama, LM Studio, MLX) serve Shiioo's own agent loop. Each is one `[[employees]]` entry; new tasks go to the first employee whose subscription window isn't exhausted.
- **The office is one binary.** `shiioo` is the server (the record, the rules, the API and dashboard, the Telegram bot, the workers) and the node that runs the agents, in one process on your machine. Its data is one directory with an embedded database. No Postgres, no containers.
- **The record is the only state.** Every run is an append-only, hash-chained event log: each model message, tool call, result, rule verdict and approval. Workers are stateless and hold fenced leases, so a crash, a sleeping laptop or a lost connection means the run resumes from its log, and a side effect never silently happens twice.
- **Rules first, you second.** `rules.toml` says what agents may do without asking (allow / ask / deny per tool, command, path and repo). Before every tool call the harness asks Shiioo; most calls are decided at once, the rest wait for your tap. A dead server means deny, never an unguarded agent.
- **Your phone is the interface, the desk is the big screen.** Telegram for delegating, approving and being told when something finished or needs you; a web dashboard over Tailscale for the live log, the diff and the buttons, one column on the phone and a three-column desk on a wide screen. You can also talk to an agent directly: send it a message mid-run, answer a run that waits for you, or follow up on a finished one. No public address anywhere: the bot long-polls, the dashboard sits on your tailnet.

## Status

| Area | State |
|---|---|
| Kernel: event log, hash chain, leases with fencing, crash recovery, redb store (Postgres optional) | built, conformance-tested on all backends, plus a seeded simulation of crashes and races |
| Claude Code as a harness: headless, worktree and branch per run, resume after a crash | built, verified with the real binary |
| Codex as a harness, same approval hook | built, verified with the real binary |
| Rules (`rules.toml`), approvals via the harness hook, cancellation | built, verified live |
| Telegram bot: `/run`, `/status`, `/log`, `/cancel`, `/capacity`, Allow / Deny buttons, notifications | built; tested against a fake Telegram server, live check pending |
| The desk (web dashboard): what needs you in a sidebar, runs by repository, a conversation-style live log, the diff and approval side by side, light and dark, a phone layout | built, verified in a browser |
| Talking to an agent: messages mid-run (read at its next turn), runs that park for your reply, follow-ups that continue a finished Claude Code or Codex run on its branch and session | built; verified live with Claude Code 2.1 and Codex 0.163 |
| `shiioo install`: login service, keep-awake, token handling | built (macOS launchd; Linux systemd unit untested) |
| Capacity broker across subscription windows, `[[employees]]`, `/capacity` | built (Codex reports no windows in headless mode yet) |
| Local models through `ollama/…`, `lmstudio/…`, `openai/…` names | built; untested against a live Ollama |
| GitHub-issue intake (`[intake]`, label `shiioo`) | built (fake `gh` tests) |
| `container` isolation (Docker/Podman, read-only root, worktree mounted), egress allow list through a proxy, encrypted secret store with redaction | built; container runs verified live with the real harness image |
| `openshell` isolation | experimental adapter, not run against a gateway |
| The company: `/plan` (a chief-of-staff run whose approval creates issues for the intake), `[[routines]]` on a typed schedule, the morning report, `[[roles]]` | built; planner verified live, the rest end to end over HTTP |
| Remote nodes: `shiioo node` on another machine takes runs addressed to it (`/run on:spark …`) and works against the office's log over HTTP | built; verified live with a node process and a Codex run |
| Deliverables (`/files`, `/get`, files sent to the phone) and mail with send-approval (`shiioo mail send`) | built, end to end over HTTP |
| Planner, routines, morning report; remote nodes; asset and mail work | planned |
| Fork, rewind, diff, replay as evals | planned |

By default a harness runs with the permissions of your user and no OS sandbox (`worktree`); the rules and your approvals are what stand between an agent and your machine. Set `[isolation] default = "container"` in `.shiioo/config.toml` (needs Docker, colima or Podman) to run agents in a container per run with only their worktree mounted and only the hosts you allow reachable; `shiioo secret set` keeps the tokens they need out of the record.

## Quick start

On the machine that will be the office (macOS; log in once to `claude` and/or `codex` first):

```sh
cargo install --path crates/shiioo                 # one binary: shiioo
export SHIIOO_TELEGRAM_TOKEN=...                   # a bot from @BotFather
shiioo --workspace ~/shiioo-office install         # login service, starts at boot, keeps the Mac awake
```

`install` prints the dashboard link and the Tailscale steps (`tailscale serve --bg 7878`, then `https://<mac>.<tailnet>.ts.net`). The first start writes `~/shiioo-office/.shiioo/config.toml` and `rules.toml`; put your Telegram user id under `[telegram] owner_chat_id`, your repositories under `[repos]`, and restart the service. Everything the office keeps is in that `.shiioo/` directory: see [the office layout](crates/shiioo/docs/office-layout.md).

Then, from your phone:

- `/run myproject fix the flaky test in tests/net.rs and open a PR`
- `/status`, `/log <run>`, `/cancel <run>`, `/capacity`
- Allow / Deny buttons when a command needs you; a message when a run ends.

Or from a terminal, without the office:

```sh
shiioo --workspace ~/shiioo-office run --harness claude-code --repo ~/code/myproject "Fix the flaky test and commit"
shiioo --workspace ~/shiioo-office log <run-id>        # every step, chain verified
shiioo --workspace ~/shiioo-office resume <run-id>     # after Ctrl-C or kill -9: same Claude Code session
```

`--harness codex` for Codex; without `--harness`, Shiioo's own loop runs the task with a model named by `--model` (`claude-opus-5-5` with `ANTHROPIC_API_KEY`, or `ollama/<model>` locally). `shiioo up` is the office in the foreground.

## Documentation

- [What Shiioo is](docs/positioning.md): the product definition, principles, topology, roadmap.
- [Architecture](docs/architecture.md): the one rule, the guarantees, components, crate map.
- [Design documents](docs/README.md): one per area, each section marked built or planned.
- [Decisions](docs/adr/README.md): the ADRs, including why this is a personal workforce and not a managed cloud, why harnesses come first, and why one binary.
- [Contributing](docs/contributing.md): building, testing, conventions, how to add things.
- Each crate has a `README.md` and a `docs/` directory of its own.

## Repository layout

| Crate | Purpose |
|---|---|
| [`shiioo-core`](crates/shiioo-core) | Event model, hash chain, leases, recovery rules, the `EventStore` trait. No I/O. |
| [`shiioo-store`](crates/shiioo-store) | Store backends: redb (default), Postgres (`postgres` feature), in-memory; the `Notifying` wrapper for live streams. |
| [`shiioo-runtime`](crates/shiioo-runtime) | Workers, the native agent loop, the Claude Code and Codex harness drivers, built-in tools, the local machine with its call journal, worktrees. |
| [`shiioo-rules`](crates/shiioo-rules) | The owner's allow / ask / deny rules. |
| [`shiioo-providers`](crates/shiioo-providers) | OpenAI-compatible model provider (Ollama, LM Studio, MLX, OpenAI) and the prefix router. |
| [`shiioo-api`](crates/shiioo-api) | HTTP API and resumable SSE, the approval flow, the capacity tracker and employee broker, the dashboard (the desk). |
| [`shiioo-telegram`](crates/shiioo-telegram) | Telegram Bot API client, owner-gated command loop, approval buttons, notifications. |
| [`shiioo-intake`](crates/shiioo-intake) | GitHub-issue intake through `gh`. |
| [`shiioo-sim`](crates/shiioo-sim) | Seeded simulation of worker crashes, lease races and machine restarts against the kernel's guarantees. |
| [`shiioo`](crates/shiioo) | The binary: `run`, `resume`, `log`, `serve`/`up`, `install`, `uninstall`, `hook`. |

## Development

```sh
cargo test                                      # everything that needs no services
cargo clippy --workspace --all-targets --all-features
SHIIOO_TEST_DATABASE_URL=postgres://user:pass@localhost/shiioo cargo test --all-features   # + the Postgres backend
SHIIOO_SIM_SEEDS=2000 cargo test -p shiioo-sim --release                                  # a longer simulation
```

Workspace lints forbid `unsafe` and `unwrap()`. Conventions, fixtures and how to add a harness, a backend, an event or a command are in [docs/contributing.md](docs/contributing.md).

## Brand

The mascot is an original character: a seasoned strategist in a three-piece suit who runs the company while you set the direction. The wordmark is set in [Bricolage Grotesque](https://github.com/ateliertriay/bricolage) and the tagline in [Inter](https://github.com/rsms/inter), both under the SIL Open Font License (see `assets/brand/fonts`). Marks, icons and the favicon are in [`assets/brand`](assets/brand), generated by `python3 assets/brand/build.py` (needs fontTools, ImageMagick and resvg).

The previous Shiioo codebase lives at [raskell-io/shiioo-legacy](https://github.com/raskell-io/shiioo-legacy); see [`docs/legacy-port.md`](docs/legacy-port.md) for what carries over.

## License

Apache-2.0. See [LICENSE](LICENSE). The brand assets in `assets/brand` are covered by the same license, but the name and mascot identify the project: please don't use them to suggest endorsement.
