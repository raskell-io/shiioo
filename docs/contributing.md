# Contributing

Shiioo is a Cargo workspace of ten crates, one binary. This page is the practical side: building, testing, the conventions the code follows, and where to add things.

## Build and test

```sh
cargo build --workspace
cargo test                                             # unit, integration and simulation tests that need no services
cargo clippy --workspace --all-targets --all-features  # must be clean
cargo fmt
```

Optional services:

| What | How |
|---|---|
| Postgres backend | `SHIIOO_TEST_DATABASE_URL=postgres://shiioo:shiioo@localhost:54329/shiioo cargo test --all-features`. A throwaway database: `docker run -d --name shiioo-pg -e POSTGRES_PASSWORD=shiioo -e POSTGRES_USER=shiioo -e POSTGRES_DB=shiioo -p 54329:5432 postgres:17-alpine`. Under `CI`, a missing URL fails the Postgres suite instead of skipping it. |
| Longer simulation | `SHIIOO_SIM_SEEDS=2000 cargo test -p shiioo-sim --release`; `SHIIOO_SIM_SEED=<n>` reproduces one seed. |
| Live harness checks | Running the real `claude` or `codex` spends the owner's subscription. Tests never do; they drive fake harnesses that replay recorded output. When you check something live, keep the task tiny and name a small model (`--model claude-haiku-5-5`). |

Tests that need a fake executable write a shell script and run it as `/bin/sh <script>` (`HarnessSpec::binary_args`, `IntakeConfig::gh_prefix_args`). macOS stalls for tens of seconds on the first direct execution of a freshly written script; running it through the shell avoids that.

## Conventions

- **Lints.** The workspace forbids `unsafe` and denies `unwrap()`; `expect()` warns. Use `?`, `unwrap_or_else(|e| panic!("{e}"))` in tests, or handle the case.
- **The event log is the only state** ([ADR-0001](adr/0001-event-log-is-the-only-state.md)). A feature that needs state expresses it as events; a projection may cache, never decide.
- **Write before acting.** Record the intent (model response, tool intent) before the effect, so a crash at any point leaves a log that says what did and didn't happen.
- **Backends only persist.** The rules of the log live in `shiioo_core::RunHead`; every `EventStore` backend calls them inside its own transaction and passes the shared conformance suite (`crates/shiioo-store/tests/common`).
- **Fixtures are real.** Recorded output from the real Claude Code and Codex, sanitized (paths, anything personal), with the version noted in the doc that cites them. Don't hand-write a fixture for a shape you haven't seen.
- **Links must resolve.** `python3 scripts/check-docs.py` checks every relative link in the README, `docs/` and the crates' docs; run it before committing documentation.
- **Docs carry status markers.** Sections and table rows say *(built)*, *(planned)* or *(deferred, ADR-n)*. When you build something, flip its marker and name the crate; when the code and a doc disagree, the doc is wrong.
- **Decisions get an ADR.** Anything you'd want a future reader to not re-litigate: context, decision, consequences, when to revisit. ADRs are never edited after acceptance except for status; a new decision supersedes.
- **Commit messages** say what changed and why, in sentences. Avoid backticks inside a double-quoted `git commit -m "…"`: the shell runs them.

## Where things live

| To change… | Look in |
|---|---|
| The event format, hashing, leases, recovery rules | `shiioo-core` (`event.rs`, `head.rs`, `chain.rs`, `recovery.rs`) |
| How events are stored | `shiioo-store` (one file per backend, `notify.rs`) |
| The agent loop, harness drivers, tools, the machine, worktrees | `shiioo-runtime` (`worker.rs`, `harness/`, `tools.rs`, `machine.rs`, `workspace.rs`) |
| What agents may do without asking | `shiioo-rules` |
| Model providers for the native loop | `shiioo-runtime/anthropic.rs`, `shiioo-providers` |
| HTTP routes, approvals, capacity, employees, the dashboard | `shiioo-api` (`lib.rs`, `employees.rs`, `ui/`) |
| Telegram commands and messages | `shiioo-telegram`, plus the `Office` implementation in `crates/shiioo/src/main.rs` |
| Issue intake | `shiioo-intake` |
| CLI commands, config file, service installer, the hook program | `crates/shiioo/src/main.rs` |

## How to add…

**An event (payload variant).** Add it to `Payload` in `shiioo-core/src/event.rs`; decide whether it is external (`is_external`) or terminal (`is_terminal`); add a row to the catalogue in [design/event-log.md](design/event-log.md) with its writer; teach the printers (`shiioo_api::describe`, `shiioo_sim::compact`) and the dashboard's `describe()` in `crates/shiioo-api/ui/app.js`; if the runtime must react to it, do so in `worker.rs`. Adding a variant does not change `SCHEMA_VERSION`: old events still decode, and new events are rejected by older builds.

**A store backend.** Implement `EventStore` by calling `RunHead` for every rule (never re-implement a check), keep the payload bytes exactly as written, then add a `tests/<backend>.rs` that invokes `conformance_suite!(setup)`. If a case can't pass, the backend is wrong, not the case.

**A harness.** Implement `HarnessDriver` in `shiioo-runtime/src/harness/`: a command builder (scrub the parent session's environment, remove API keys for subscription billing, install the Shiioo hook if the harness supports one) and `interpret(line)` mapping the harness's output onto `HarnessSessionStarted`, `ModelResponseRecorded`, `ToolIntent`/`ToolResult`, `CapacityObserved`, `HarnessResult` and a terminal event. Record real output as fixtures, add `HarnessKind` and a constructor, a `tests/harness_<name>.rs` with a fake (happy path, kill-and-resume, repo run), the CLI value in `--harness`, and a section in [design/runtimes-and-providers.md](design/runtimes-and-providers.md).

**A rule kind or default.** `shiioo-rules`: extend `Rule` and `evaluate`, keep first-match and most-restrictive-segment semantics, update `defaults()` and its test, and the format doc in the crate.

**A Telegram command.** Add the variant to `Command` and the parser in `shiioo-telegram/src/command.rs`, the dispatch in `bot.rs`, a method on `Office`, the help text, the parser test, and the implementation in the binary's `TelegramOffice`.

**An API route.** A handler in `shiioo-api/src/lib.rs` under the authenticated router, a method on `ApiState` so the Telegram and intake layers can share it, a test in `crates/shiioo-api/tests/`, and the row in [design/api.md](design/api.md).

## Releasing

Not yet. Until there is a release process, `cargo install --path crates/shiioo` on the office machine and `shiioo uninstall && shiioo install` to pick up a new binary.
