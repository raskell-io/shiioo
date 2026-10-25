<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/shiioo-mark-dark.svg">
    <img src="assets/brand/shiioo-mark-light.svg" alt="Shiioo: Virtual Company OS" width="560">
  </picture>
</p>

<h1 align="center">Shiioo</h1>

<p align="center">
  <strong>Virtual Company OS.</strong> Hire AI agents, give them policies and budgets, and keep every decision on the record.<br>
  The open-source agent platform where every run is governed, replayable, forkable and provable.
</p>

Shiioo is a staff of AI agents (Claude Code, Codex, local models) that work on your own machines around the clock, which you direct and supervise from your phone. It's one Rust binary with an embedded database: a server that keeps the record and makes decisions, nodes that run the work and dial out to it. It is designed around one rule: **the per-run event log is the only state.** Everything else follows from that:

- **Rock-solid execution.** Workers are stateless and hold fenced leases. Every write carries an epoch, and a stalled worker that wakes up after a takeover is rejected. Side-effecting tool calls are recorded as an intent before they run, so a crash never causes a blind retry of a `git push` or a payment.
- **Governed.** Policies as code, approval boards with quorum rules, budgets, and change management for configuration an agent proposes.
- **Forkable.** Rewind a run to any step, fork it with a different prompt, model or policy, and diff the branches, including the workspace files *(planned)*.
- **Replayable.** Re-run recorded production runs against a new model as an eval suite *(planned)*.
- **Provable.** Every event is hash-chained to the one before it. Tampering, gaps and reordering are detectable.
- **Bring your own harness.** Run Claude Code, Codex and others as runtimes through the Agent Client Protocol, with the same log, policies and replay *(planned)*.

> **Status: early.** This is a ground-up rewrite. Today the repo contains the execution kernel (event model, hash chain, leases with fencing, crash-recovery rules) and three store backends (redb, Postgres, in-memory) that pass one shared conformance suite. Start with the [architecture overview](docs/architecture.md), the [design docs](docs/README.md) and the [roadmap](docs/architecture.md#10-roadmap).
>
> The previous Shiioo codebase lives at [raskell-io/shiioo-legacy](https://github.com/raskell-io/shiioo-legacy). Its governance features are being ported onto the new kernel; see [`docs/legacy-port.md`](docs/legacy-port.md).

## Layout

| Crate | Purpose |
|---|---|
| `shiioo-core` | Event types, hash chain, leases, recovery rules, the `EventStore` trait. No I/O. |
| `shiioo-store` | Store backends: `RedbStore` (embedded, default), `PostgresStore` (cluster, `postgres` feature), `MemoryStore` (reference and simulation). |
| `shiioo-runtime` | The agent loop: workers, the Anthropic provider, built-in tools (`read_file`, `write_file`, `shell`), and the local machine with its call journal. |
| `shiioo-api` | HTTP API: create, inspect and resume runs; events as JSON or as a resumable SSE stream. Bearer-token auth, problem+json errors. |
| `shiioo` | The binary. Today: `run`, `resume`, `log` and `serve` on this machine. |

## Try it

Hire Claude Code on your subscription (log in once with `claude`), on a repository of yours:

```sh
cargo run -p shiioo -- --workspace ~/shiioo-office run --harness claude-code --repo ~/code/myproject \
    "Fix the flaky test in tests/net.rs and commit"
```

The run gets its own git worktree and a `shiioo/<run>` branch; every step Claude Code takes is in the run's log (`shiioo log <run-id>`), and `shiioo resume <run-id>` continues the same Claude Code session after a crash. No API key needed.

Run the office and direct it from your phone:

```sh
cargo install --path crates/shiioo                      # one binary: shiioo
export SHIIOO_TELEGRAM_TOKEN=...                        # a bot from @BotFather
shiioo --workspace ~/shiioo-office install              # launchd service, starts at login, keeps the Mac awake
tailscale serve --bg 7878                               # https://<mac>.<tailnet>.ts.net from your phone
```

`install` prints the dashboard link to open on your phone: runs, live logs, diffs, Allow/Deny and Cancel buttons, as a home-screen web app. (`shiioo up` runs the same thing in the foreground.)

The first start writes `~/shiioo-office/.shiioo/config.toml` (put your Telegram user id under `[telegram] owner_chat_id` and your repositories under `[repos]`) and `rules.toml` (what agents may do without asking). Then, from Telegram: `/run myproject fix the flaky test`, `/status`, `/log <run>`, `/cancel <run>`. Before every tool call Claude Code asks Shiioo; the rules allow or deny most calls, and the rest arrive on your phone with Allow / Deny buttons. Everything is recorded in the run's log.

Or use Shiioo's own loop with a model API key:

```sh
export ANTHROPIC_API_KEY=...
cargo run -p shiioo -- --workspace /path/to/project run "Summarise what this project does in SUMMARY.md"
cargo run -p shiioo -- --workspace /path/to/project log <run-id>      # events + chain check
cargo run -p shiioo -- --workspace /path/to/project resume <run-id>   # after Ctrl-C or kill -9
```

Or serve the API and watch a run live:

```sh
cargo run -p shiioo -- --workspace /path/to/project serve        # http://127.0.0.1:7878
TOKEN=$(cat /path/to/project/.shiioo/api-token)
curl -s -H "Authorization: Bearer $TOKEN" -d '{"task":"List the files"}' \
     -H 'content-type: application/json' http://127.0.0.1:7878/v1/runs
curl -N -H "Authorization: Bearer $TOKEN" http://127.0.0.1:7878/v1/runs/<run-id>/events/stream
```

The stream is resumable: reconnect with `Last-Event-ID: <seq>` and it continues right after that event. `serve` and `run` can't share a workspace at the same time (the embedded database allows one process).

The agent can read, write and run shell commands in the workspace, with no sandbox yet: use a directory you don't mind it changing. Shiioo keeps its data in `.shiioo/` there. The default model is `claude-opus-5-5`, with server-side refusal fallback enabled.

## Development

```sh
cargo test
cargo clippy --all-targets --all-features

# Postgres backend: point the suite at a scratch database
SHIIOO_TEST_DATABASE_URL=postgres://user:pass@localhost/shiioo cargo test --all-features
```

## Brand

The mascot is an original character: a seasoned strategist in a three-piece suit who runs the company while you set the direction. "Virtual Company OS" is the tagline; "agent platform" is what Shiioo technically is. The wordmark is set in [Bricolage Grotesque](https://github.com/ateliertriay/bricolage) and the tagline in [Inter](https://github.com/rsms/inter), both under the SIL Open Font License (see `assets/brand/fonts`). Marks, icons and the favicon are in [`assets/brand`](assets/brand), generated from the sources there by `python3 assets/brand/build.py` (needs fontTools, ImageMagick and resvg).

## License

Apache-2.0. See [LICENSE](LICENSE). The brand assets in `assets/brand` are covered by the same license, but the name and mascot identify the project: please don't use them to suggest endorsement.
