# Approvals and capacity

Two things the API coordinates in memory on top of the event log: the
owner's answer to "may the agent run this?", and how much of each
subscription is left.

## The permission flow

```
harness (Claude Code / Codex)
   │ PreToolUse hook: `shiioo hook pre-tool-use`
   ▼
POST /v1/runs/{run}/permissions  {tool_name, tool_input, …}
   │
   ├─ PermissionRequested appended (append_external)
   ├─ rules.toml judged ──► PolicyDecision {decision, rule "rule N", reason}
   │        allow / deny ──► respond at once
   │        ask ───────────► ApprovalRequested {approval_id, summary, expires_at}
   │                           + pending oneshot + Notifications::approval_requested
   │                           … owner taps Allow/Deny (Telegram, dashboard, curl) …
   │                         POST /v1/approvals/{approval_id}  or  ApiState::decide
   │                           ──► ApprovalDecided {decision, by, note}
   │                           + Notifications::approval_resolved
   ▼
respond {decision, reason, approval_id}  ──►  hook prints Claude/Codex JSON
```

Step by step (`ApiState::permission`):

1. The hook's request becomes a `PermissionRequested` event so the log shows
   every call the harness wanted to make, including the ones rules allowed.
2. The rule set (`shiioo-rules`, loaded from `rules.toml`) is asked. First
   matching rule wins; the result is a `PolicyDecision` event with `rule:
   "rule N"` and the rule's reason. `allow` and `deny` return immediately.
3. `ask` appends `ApprovalRequested { approval_id (uuid), call_id, tool,
   input, summary, expires_at }`, parks a `oneshot` sender in
   `ApiState.pending`, calls `Notifications::approval_requested(run,
   approval_id, summary)` (Telegram sends a message with Allow/Deny buttons
   and a dashboard link) and waits.
4. A decision arrives through `POST /v1/approvals/{id}` or, from Telegram,
   `ApiState::decide(approval_id, allow, by, note, notify)`. `decide` first
   looks at `pending`; if the id is not there (for example after a restart)
   it scans all runs for an open approval with that id
   (`find_open_approval`). It appends `ApprovalDecided { approval_id,
   decision, by, note }` and fires the oneshot. `by` is `"owner"` from HTTP
   and `"owner:telegram"` from the bot.
5. The hook gets `{"decision": "allow"|"deny", "reason": "allowed by the
   owner (<summary>)" | "denied by the owner (<summary>)", "approval_id"}`.

### Timeout

`APPROVAL_TIMEOUT = HOOK_TIMEOUT_SECS − 120 s`, that is 5 h 58 min. If no
decision arrives, the server appends `ApprovalDecided { decision: deny, by:
"timeout", note: "no answer within N hours" }`, calls
`approval_resolved(approval_id, "expired")` and answers `deny`. The two
minutes of slack make sure Shiioo answers before the harness's own six-hour
hook timeout kills the call with a less helpful error.

### What survives a restart

The `ApprovalRequested` event is in the log, so `RunStatus::WaitingApproval`
and the dashboard card survive a restart, and `decide` still finds the
approval by scanning the log. What does not survive is the parked HTTP
request: the hook inside the harness is still waiting on a connection the
new server process never saw, so it eventually times out on its side and the
harness treats the call as denied. Re-asking on restart is *(planned)*.

### Notifications

```rust
pub trait Notifications: Send + Sync {
    fn approval_requested(&self, run: RunId, approval_id: &str, summary: &str);
    fn approval_resolved(&self, approval_id: &str, outcome: &str); // "allowed" | "denied" | "expired"
    fn run_ended(&self, run: RunId, task: &str, status: &RunStatus);
}
```

`run_ended` is called by `spawn_drive` when a drive returns anything but
`LostLease`. `spawn_drive` retries a drive up to three times on `LostLease`
before giving up on it.

## Capacity

Claude Code reports its subscription windows in `rate_limit_event` lines;
the Claude Code adapter records them as `CapacityObserved { provider:
"claude-code", windows }`. The worker's observer hands every event to
`CapacityTracker::observe_event`, so the tracker is current without polling.

- `observe(provider, windows, observed_at)` reads each window's
  `utilization`, `resetsAt` or `resets_at`, and `status`; a newer observation
  replaces an older one, an older one is ignored.
- `snapshot()` is what `GET /v1/capacity` returns; `provider(name)` one
  entry.
- `text()` is the human form used by Telegram `/capacity` and the dashboard's
  capacity line, for example `claude-code · 5h 42% (resets in 1h 20m) · 7d
  18%`. Empty tracker: `No capacity data yet; it appears after the first
  harness run.`
- `ApiState::seed_capacity()` is called once at server start; it walks the
  stored runs and replays their `CapacityObserved` events so the tracker is
  not blind after a restart.

Codex's exec stream carries no windows, so Codex is never marked busy
*(Codex app-server integration planned)*.

## Employees and the broker

`src/employees.rs` turns "who takes this task" into configuration:

```toml
[[employees]]
name = "claude"
kind = "claude-code"
max_five_hour = 0.95   # default
max_seven_day = 0.90   # default

[[employees]]
name = "codex"
kind = "codex"

[[employees]]
name = "local"
kind = "native"
model = "ollama/qwen3"
```

- `EmployeeKind` is `claude-code`, `codex` or `native { model }`;
  `provider_key()` names the capacity entry to consult (`claude-code`,
  `codex`, or the model's prefix before `/` for native).
- `pick(employees, capacity) -> Pick { employee, skipped }` returns the first
  employee whose provider is unknown to the tracker or under both thresholds;
  everyone passed over is listed in `skipped` with a reason such as
  `five_hour window at 96% (limit 95%)`.
- `default_employees()` is `claude` (Claude Code) then `codex`.

`ApiState::assign(employee, repo)` maps the choice to a run spec: Claude
Code → `HarnessSpec::claude_code()` with the repo; Codex →
`HarnessSpec::codex()` with the repo; native → `Runtime::Native` with the
employee's model and the server's default tools. `max_turns` is 50 in every
case. `"auto"` goes through `pick`; a name picks that employee even when it
is out of capacity.

See also: [governance design](../../../docs/design/governance.md),
[runtimes and providers design](../../../docs/design/runtimes-and-providers.md),
[shiioo-rules](../../shiioo-rules/README.md) for the rule syntax,
[hook.md in the binary](../../shiioo/docs/hook.md) for the client side.
