# shiioo-runtime

The worker that drives one run from its event log to a terminal event. It
owns the two execution paths Shiioo has today: the **harness runtime**, which
spawns Claude Code or Codex headless and records what they do, and the
**native runtime**, Shiioo's own model-plus-tools loop. It also owns the
local machine (tool execution with a crash journal) and per-run git
worktrees.

Everything the worker knows comes from the log, and everything it decides
goes back into the log as events. A worker that dies mid-run is replaced by
another worker that reads the same log and continues; see
[docs/worker-loop.md](docs/worker-loop.md).

## Position in the crate graph

```
shiioo-core  ──►  shiioo-runtime  ──►  shiioo-api  ──►  shiioo (binary)
                       ▲
            shiioo-store (dev-dependency, tests only)
```

- Depends on `shiioo-core` for the event types, `EventLog` store trait,
  leases and ids. Nothing here knows about HTTP, Telegram or rules.
- `shiioo-api` builds `Worker`s and reads `RunStatus`; `shiioo` wires the
  real store, machine and hook endpoint.
- `shiioo-store` is only a dev-dependency (redb and memory backends for the
  crash-recovery tests).

## Module map

| Module | What it holds |
| --- | --- |
| `worker` | `Worker`, `RunOutcome`, `WorkerError`, `Faults`/`CrashPoint` (test crash injection). The loop itself. |
| `harness` | `HarnessDriver` trait, `HarnessProcess`, `CONTINUE_PROMPT`, `driver_for`; submodules `claude_code` and `codex`. |
| `spec` | `RunSpec`, `AgentSpec`, `Runtime`, `HarnessSpec`, `HarnessKind`, `HarnessBilling`: the `spec` stored in `RunCreated`. |
| `machine` | `LocalMachine`, `Invocation`, `CallStatus`, `MachineError`: executes tools behind a fenced journal. |
| `tools` | `Tool`, `Toolbox`, `ToolOutput`; built-ins `read_file`, `write_file`, `shell`. |
| `workspace` | `Worktree`, `ensure`, `remove`, `branch_for`: one git worktree and branch per repo run. |
| `status` | `RunStatus`, `run_status`, `open_approval`: fold a log into one state. |
| `model` | `ModelProvider` trait, `ModelRequest`/`ModelResponse`, `Transcript`, `ToolCall`, `StopReason`, `Usage`. |
| `anthropic` | `AnthropicProvider`, the native runtime's Messages API client. |
| `scripted` | `ScriptedProvider`, canned responses for tests. |
| `context` (private) | Rebuilds the model transcript from the log. |
| `unconfigured` (private) | `UnconfiguredProvider`, fails native runs with a clear message. |

## Public API

The crate re-exports everything from `lib.rs`; the names below are the ones
other crates use.

**Running**

- `Worker::new(store, provider, machine, id)` then `.with_hooks(HookEndpoint)`,
  `.with_faults(Faults)`, `.with_observer(Arc<dyn Fn(&Event)>)`.
  Public fields: `id`, `ttl_ms` (lease TTL, default 30 000 ms), `faults`.
- `worker.create(&RunSpec) -> RunId` appends `RunCreated`.
- `worker.drive(run_id) -> RunOutcome` runs until the run closes or the
  worker loses its lease. `RunOutcome` is `Completed(output)`,
  `Failed(reason)`, `NeedsAttention(reason)`, `Cancelled(reason)` or
  `LostLease`.
- `run_status(&log) -> RunStatus` and `open_approval(&log)` for anyone who
  only wants to read.

**Describing a run**

- `RunSpec { task, agent: AgentSpec, runtime: Runtime }`.
- `AgentSpec { model, system, tools, max_turns }` with `max_turns` default 50.
- `Runtime::Native` (default) or `Runtime::Harness(HarnessSpec)`; serialised
  with `kind = "native" | "harness"`.
- `HarnessSpec::claude_code()` and `HarnessSpec::codex()` give the defaults
  listed in [docs/harness-adapters.md](docs/harness-adapters.md).

**Machines and tools**

- `LocalMachine::in_memory(workspace, toolbox)` or
  `LocalMachine::open(workspace, journal_path, toolbox)`, plus
  `.with_worktrees(root)` to allow repo runs.
- `Toolbox::builtin()` and `Toolbox::with(tool)`; implement `Tool` for new
  tools. See [docs/machine-and-tools.md](docs/machine-and-tools.md).

**Providers (native runtime only)**

- `AnthropicProvider::from_env()` reads `ANTHROPIC_API_KEY` or
  `ANTHROPIC_AUTH_TOKEN` and optional `ANTHROPIC_BASE_URL`; public knobs
  `effort` ("high"), `max_tokens` (16 000), `fallbacks` (true).
- `ScriptedProvider::new(responses)` for tests; `UnconfiguredProvider` when
  no key is set.

## Environment variables read here

| Variable | Used by | Effect |
| --- | --- | --- |
| `SHIIOO_CLAUDE_BIN` | `claude_code` | Harness executable when `HarnessSpec.binary` is unset; falls back to `claude` on `PATH`. |
| `SHIIOO_CODEX_BIN` | `codex` | Same for Codex; falls back to `codex`. |
| `ANTHROPIC_API_KEY`, `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_BASE_URL` | `anthropic` | Native runtime credentials and endpoint. |

Variables the harness drivers *remove* from the child environment are listed
in [docs/harness-adapters.md](docs/harness-adapters.md#environment-handling).

## Testing

```sh
cargo test -p shiioo-runtime
```

- `tests/crash_recovery.rs`: kills the worker at every `CrashPoint` on both
  the memory and redb stores and checks a second worker finishes the run
  with every tool executed exactly once, plus lease takeover and journal
  reopen.
- `tests/external_input.rs`: a cancel before any work, owner input appended
  mid-run (`SeqConflict` absorbed, not fatal), status with an open approval.
- `tests/harness_claude.rs` and `tests/harness_codex.rs`: a fake harness
  (a shell script replaying the JSONL fixtures in `tests/fixtures/`) run
  through `/bin/sh` via `HarnessSpec.binary_args`; covers recording,
  kill-and-resume, dying harnesses and worktree creation.
- Unit tests inside `anthropic`, `harness/claude_code` and `harness/codex`
  parse recorded sessions and check the exact command lines built.

No test needs network access or a real harness installed.

## Further reading

- [docs/worker-loop.md](docs/worker-loop.md): lease, heartbeat, recovery,
  append conflicts, how each `RunOutcome` is reached.
- [docs/harness-adapters.md](docs/harness-adapters.md): command lines,
  event mapping, resume and hooks for Claude Code and Codex.
- [docs/machine-and-tools.md](docs/machine-and-tools.md): journal format,
  fencing, tool classes, built-in tools, worktrees.
- Top-level design: [execution](../../docs/design/execution.md),
  [runtimes and providers](../../docs/design/runtimes-and-providers.md),
  [tools and machines](../../docs/design/tools-and-machines.md),
  [event log](../../docs/design/event-log.md),
  [architecture](../../docs/architecture.md),
  [ADR-0009 harness-first runtimes](../../docs/adr/0009-harness-first-runtimes.md).
