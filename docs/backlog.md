# Backlog

Known rough edges and small debts, found while reviewing or documenting the code. None blocks the phase-4 feature set; each should be fixed or consciously kept. Bigger items live in the [roadmap](positioning.md#roadmap).

## Correctness

- **`EventPayload`'s `Deserialize` decodes with the build's `SCHEMA_VERSION`**, not the event's own `schema` field (serde sees the payload before its sibling). Harmless with one schema; must be fixed before a second schema version exists (decode `Event` with a custom `Deserialize` that reads `schema` first).
- **`RunHead::renew_lease` does not check `closed`.** Renewing on a finished run succeeds; every append then fails with `RunClosed`. Refusing the renewal would stop a worker a little earlier.
- **Approval requests do not survive a server restart while pending.** The hook's HTTP request dies with the server; the harness sees a deny after its timeout. The `ApprovalRequested` event is in the log, and `decide()` still records a late answer, but nothing re-asks the owner. A restart could re-notify open approvals.
- **Cancellation is noticed at the next event.** A harness deep in a long command stops when that command ends. An in-process cancel token per active run would stop it at once.

## Behaviour

- **Default rules**: `rm -rf /*` matches any absolute path, so `rm -rf /tmp/build` is denied rather than asked. Broader than its reason suggests; decide.
- **`shiioo-providers` sleeps twice** on a retryable status with `Retry-After`: the header's delay in `send`, then the loop's backoff. A 429 with `Retry-After: 30` waits 31 s. Drop the loop backoff when the header was honoured.
- **The Telegram bot does not persist its update offset**, so a restart re-delivers unacknowledged updates. The owner gate and the office's own idempotency tolerate it; a persisted offset in `.shiioo/` would be cleaner.
- **Intake never forgets an issue.** An issue relabelled `shiioo` after `done`/`failed` is not re-run; editing `intake-state.json` is the only way. A `shiioo:again` label or a `/rerun` command would do.
- **`Idempotency-Key` on `POST /v1/runs` is kept in memory** and lost on restart.
- **Codex's capacity windows** are not in its `exec` stream; the broker treats Codex as always available. Its app-server exposes `account/rateLimits`.
- **`worktree` runs have no OS sandbox** (Claude Code never had one in this mode; Codex's is off because it blocks commits). The `container` level is the answer; `worktree` stays the default until the owner changes `[isolation] default`.
- **The `openshell` level is untested against a gateway**: its commands and policy follow the OpenShell documentation (ADR-0011); the first live run will show what needs adjusting (policy keys, upload semantics, `sandbox exec` output).
- **Egress for `container` runs allows TLS on 443 and plain HTTP only.** `git` over SSH, other ports and UDP do not leave the sandbox; remotes are rewritten to HTTPS inside. A per-run port list could follow if a toolchain needs one.
- **Claude Code's and Codex's own telemetry and update checks** fail quietly behind the allow list; nothing is lost, but their stderr may mention it.
- **Secrets in a `worktree` or `container` run are visible to the agent** (they are in its environment). Redaction protects the record; injection at the tool boundary exists only in the design of the native loop.
- **Codex ignores `max_turns` and `max_budget_usd`**: neither has a `codex exec` equivalent that is passed today; `max_minutes` is the budget that applies to both harnesses.
- **Codex's hook matcher is `^(Bash|shell)$`**, so its file edits and MCP calls are recorded but not gated by the rules (the rules know `Edit`/`Write`, not `apply_patch`).
- **The intake's branch placeholder** is filled with the literal `$(git branch --show-current)`, which only expands when the agent runs the command in a shell. Letting the caller choose the run id would allow the real branch name.
- **The Linux systemd unit** does not pass `--keep-awake` (macOS-only today) and the install path is untested on Linux.
- **The `Command::Hook` arm** in the CLI's dispatch is unreachable (the hook path returns earlier); harmless, but tidy it when touching that match.

## Code health

- Retry helpers are duplicated between `shiioo-runtime/anthropic.rs` and `shiioo-providers`; a small `retry` module would remove that.
- `RedbStore::list_runs` scans the heads table; fine for hundreds of runs, a projection later.
- Simulator races run only on the memory backend (redb's thread pool makes the winner non-reproducible), by design.
- The CLI's `print_event` and `shiioo_api::describe` are one function now; the dashboard's JavaScript `describe()` is a third copy of the same logic and must be kept in step by hand.
