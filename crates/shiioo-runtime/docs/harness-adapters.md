# Harness adapters

A harness adapter turns one external coding agent into a stream of Shiioo
events. There are two: `harness/claude_code.rs` for Claude Code and
`harness/codex.rs` for Codex. Both implement the same trait in
`harness/mod.rs`:

```rust
pub trait HarnessDriver: Send {
    fn name(&self) -> &'static str;
    fn binary(spec: &HarnessSpec) -> PathBuf;
    fn command(&self, spec, agent, cwd, prompt, resume: Option<&str>,
               hooks: Option<(&HookEndpoint, &str)>) -> Command;
    fn interpret(&mut self, line: &str) -> Result<Interpretation, HarnessError>;
}
```

`Interpretation { payloads, session_started, outcome }` is what one stdout
line means: zero or more event payloads to append, whether this line opened a
session, and whether it closed the run. `HarnessProcess` owns the child:
`spawn(cmd)`, `next_line()` (skips blank lines), `finish()` (exit status plus
captured stderr) and `kill()`.

`driver_for(spec, agent, cwd)` picks the adapter from `spec.harness`.

## Claude Code (`claude-code`)

### Command line

```
<binary> [binary_args…] --print <prompt>
    --output-format stream-json --verbose
    --permission-mode <mode> --setting-sources <sources>
    --max-turns <agent.max_turns>
    [--model <agent.model>]
    [--append-system-prompt <agent.system>]
    [--allowedTools <tool>…]            # when agent.tools is not empty
    [--max-budget-usd <n>]
    [--resume <session_id>]             # on continuation
    [--settings <hook settings JSON>]   # when a hook endpoint is attached
```

The prompt is placed right after `--print` so later variadic flags
(`--allowedTools`) cannot swallow it. `mode` is `HarnessSpec.permission_mode`
(default `acceptEdits`), except that an attached hook forces `default`: in
`acceptEdits` Claude Code would not consult the hook for edits.
`sources` is `HarnessSpec.setting_sources` (default `project`, so the owner's
personal hooks and MCP servers stay out of unattended runs).

The binary is `HarnessSpec.binary`, else `$SHIIOO_CLAUDE_BIN`, else `claude`
on `PATH`. In a `container` or `openshell` run the image's `claude` is used
unless the spec names a binary.

### Environment handling

Removed from the child environment on every start (`SESSION_ENV`), so a
Shiioo worker started from inside a Claude Code session does not look like a
nested session: `CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`,
`CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_CODE_SESSION_ATTENDED`,
`CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_EXECPATH`,
`CLAUDE_CODE_MESSAGING_SOCKET`, `CLAUDE_CODE_MESSAGING_TOKEN`, `CLAUDE_PID`,
`CLAUDE_EFFORT`. Kept: `CLAUDE_CODE_OAUTH_TOKEN`, `CLAUDE_CONFIG_DIR`.

With `billing = subscription` (the default) `ANTHROPIC_API_KEY` and
`ANTHROPIC_AUTH_TOKEN` are removed too. Headless Claude Code prefers an API
key over the owner's login, and that would bill API credits instead of the
Max plan. `billing = api_key` leaves them in place.

Set when a hook endpoint is attached: `SHIIOO_SERVER_URL`, `SHIIOO_API_TOKEN`,
`SHIIOO_RUN_ID`. On resume: `CLAUDE_CODE_RESUME_INTERRUPTED_TURN=1`.

Stdin is `/dev/null`; stdout and stderr are piped; the child is killed when
the handle is dropped.

### Hook settings

`hook_settings(&HookCommand)` produces the JSON passed to `--settings`:

```json
{"hooks":{"PreToolUse":[{"hooks":[{"type":"command",
  "command":"<hook command>","timeout":21600}]}]}}
```

`HookEndpoint { server_url, token, command }` is what the API server hands
the worker. `HookCommand::Exe(path)` is `<shell-quoted path> hook
pre-tool-use`, the Shiioo binary itself, for `worktree` runs.
`HookCommand::Curl` is for sandboxes, which have no `shiioo` inside:

```sh
curl -sSf -X POST --max-time 21660 \
  -H "Authorization: Bearer $SHIIOO_API_TOKEN" -H "Content-Type: application/json" \
  --data-binary @- "$SHIIOO_SERVER_URL/v1/runs/$SHIIOO_RUN_ID/hook/pre-tool-use" \
  || printf '%s' '{"hookSpecificOutput":{…"permissionDecision":"deny"…}}'
```

The endpoint answers in the hook's own output format; an unreachable office
is a deny. The sandbox rewrites `server_url` to `host.docker.internal`
([sandboxes.md](sandboxes.md)). `HOOK_TIMEOUT_SECS` is 21 600 (six hours):
the hook blocks while the owner decides, so the timeout has to be long.

### Line mapping

Parsed from `--output-format stream-json`:

| Line | Events |
| --- | --- |
| `system` / `init` | `HarnessSessionStarted { harness, session_id, model, cwd, version }` |
| `assistant` | `ModelResponseRecorded { model, response: message }` plus one `ToolIntent` per `tool_use` block, class from `claude_tool_class` |
| `user` with tool results | `ToolResult { call_id: tool_use_id, output: content, is_error }` |
| `rate_limit_event` | `CapacityObserved { provider: "claude-code", windows: rate_limit_info.unifiedWindows }` |
| `result` | `HarnessResult { harness, session_id, subtype, is_error, cost_usd, usage, permission_denials }` then `RunCompleted { output: result text or last assistant text }`, or `RunFailed("harness ended with <subtype>")` / `RunFailed("<subtype>: <errors>")` |
| anything else | ignored |

`claude_tool_class` decides what recovery may do after a crash:

- `Pure`: Read, Glob, Grep, LS, WebFetch, WebSearch, NotebookRead, TodoRead,
  ToolSearch, LSP.
- `Idempotent { key }`: Edit, Write, MultiEdit, NotebookEdit, TodoWrite; the
  key is a digest of the input.
- `SideEffecting`: everything else, including Bash.

### Resume

`HarnessSessionStarted.session_id` is remembered; a later start passes it to
`--resume` with the continue prompt. Claude Code then has its own
conversation back, and the Shiioo log has every turn from both sessions.

## Codex (`codex`)

### Command line

```
<binary> [binary_args…] exec [resume <thread_id>] --json -C <cwd>
    -c approval_policy="never"
    --dangerously-bypass-approvals-and-sandbox          # danger-full-access
  | -c sandbox_mode="read-only" | "workspace-write"     # otherwise
    [-m <agent.model>]
    [--skip-git-repo-check]                              # when spec.repo is None
    [-c features.hooks=true -c hooks.PreToolUse=[…] --dangerously-bypass-hook-trust]
    <prompt>
```

The prompt is the last argument. With a system prompt it becomes
`Instructions:\n<system>\n\nTask:\n<prompt>`. `max_budget_usd` is not passed
to Codex; `max_turns` is not passed either.

`permission_mode` maps to a sandbox (`sandbox_for`):

| `permission_mode` | Codex sandbox |
| --- | --- |
| `default`, `plan`, `read-only` | `read-only` |
| `bypassPermissions`, `danger-full-access` | `danger-full-access` (`--dangerously-bypass-approvals-and-sandbox`) |
| anything else (`acceptEdits`, `workspace-write`, …) | `workspace-write` |

`HarnessSpec::codex()` defaults to `danger-full-access`. Under
`approval_policy = never` Codex cannot ask to escalate, and `workspace-write`
makes the checkout's resolved git directory read-only, so an agent in a
worktree could never commit. Shiioo's rules and the owner's approvals govern
each command instead; OS isolation is the job of the isolation levels
([sandboxes.md](sandboxes.md)).

The binary is `HarnessSpec.binary`, else `$SHIIOO_CODEX_BIN`, else `codex`.
`codex::version` runs `<binary> --version` with a 10 s timeout and keeps the
last whitespace-separated token (for example `0.163.0`), or `""` if that
fails; it is recorded in `HarnessSessionStarted.version`.

### Environment handling

With `billing = subscription`, `OPENAI_API_KEY` and `CODEX_API_KEY` are
removed. The hook variables `SHIIOO_SERVER_URL`, `SHIIOO_API_TOKEN` and
`SHIIOO_RUN_ID` are set when a hook endpoint is attached.

### Hook config

`hook_config(&HookCommand)` is the `-c` value, with the same command
variants as Claude Code's (`<exe> hook pre-tool-use` on the host, the
`curl` call in a sandbox), quoted for TOML:

```
hooks.PreToolUse=[{matcher="^(Bash|shell)$",hooks=[{type="command",
  command="<hook command>",timeout=21600}]}]
```

Only shell commands reach the hook. Codex's file changes and MCP calls are
recorded as events but are not gated by rules.

### Line mapping

Parsed from `codex exec --json`:

| Line | Events |
| --- | --- |
| `thread.started` | `HarnessSessionStarted { session_id: thread_id, model, cwd, version }` |
| `item.started` command_execution | `ToolIntent { tool: "Bash", class: SideEffecting, input: {command} }` |
| `item.completed` agent_message | `ModelResponseRecorded { response: { model, content: [text] } }` |
| `item.completed` command_execution | `ToolIntent` (if `item.started` was not seen) and `ToolResult { output: aggregated_output, is_error: exit_code != 0 }` |
| `item.completed` file_change | `ToolIntent { tool: "Edit", class: Idempotent }` and `ToolResult` with a `kind path, …` summary |
| `item.completed` mcp_tool_call | `ToolIntent { tool: "mcp__<server>__<tool>", class: SideEffecting }` and its result |
| `item.completed` web_search | `ToolIntent { tool: "WebSearch", class: Pure }` and an empty `ToolResult` |
| `item.completed` reasoning, todo_list, error, unknown | ignored |
| `turn.completed` | `HarnessResult { subtype: "success", cost_usd: None, usage, permission_denials: [] }` then `RunCompleted { output: last agent message }` |
| `turn.failed`, `error` | `HarnessResult { subtype: "error", usage: null }` then `RunFailed("codex: <message>")` |

`agent.model` empty is recorded as model `default`. Codex's exec stream
carries no rate-limit windows, so no `CapacityObserved` is produced; the
capacity broker treats Codex as always available until that is wired in
*(planned, via Codex's app-server)*.

### Resume

The thread id from `thread.started` is passed as `exec resume <thread_id>`
on the next start, with the continue prompt.

## Adding an adapter

1. Add a `HarnessKind` variant in `spec.rs` and a module under `harness/`.
2. Implement `HarnessDriver`: build the headless command, parse one line at a
   time, emit `HarnessSessionStarted` on the first line that carries a
   session id, map tool activity to `ToolIntent`/`ToolResult` with an honest
   `ToolClass`, and end with `HarnessResult` plus `RunCompleted`/`RunFailed`.
3. Wire it in `driver_for`.
4. Record a real session as a JSONL fixture under `tests/fixtures/` and
   replay it through a fake binary in an integration test, as
   `tests/harness_claude.rs` and `tests/harness_codex.rs` do.

See also: [runtimes and providers design](../../../docs/design/runtimes-and-providers.md),
[ADR-0009](../../../docs/adr/0009-harness-first-runtimes.md),
[worker-loop.md](worker-loop.md) for the loop around the adapter.
