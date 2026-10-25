# The permission hook

Every tool call a harness wants to make goes through Shiioo first. The
mechanism is a `PreToolUse` hook that both Claude Code and Codex support: the
harness runs a command, feeds it the planned call as JSON on stdin, and reads
the decision from stdout. That command is the Shiioo binary itself:

```
shiioo hook pre-tool-use
```

It is hidden from `--help` because people never run it.

## How it gets installed

The worker (`shiioo-runtime`) installs it when it is built `with_hooks`,
which `serve` always does. Terminal `shiioo run` has no server and therefore
no hook.

- **Claude Code**: `--settings '{"hooks":{"PreToolUse":[{"hooks":[{"type":
  "command","command":"<exe> hook pre-tool-use","timeout":21600}]}]}}'` and
  `--permission-mode default`, so nothing is auto-approved behind the rules'
  back. Every tool is covered.
- **Codex**: `-c features.hooks=true -c 'hooks.PreToolUse=[{matcher="^(Bash|
  shell)$",hooks=[{type="command",command="<exe> hook pre-tool-use",
  timeout=21600}]}]' --dangerously-bypass-hook-trust`. Only shell commands
  are covered; Codex's own file edits and MCP calls are recorded but not
  gated.

Both get three environment variables from the worker:

| Variable | Value |
| --- | --- |
| `SHIIOO_SERVER_URL` | `http://<listen host>:<port>` of the office |
| `SHIIOO_API_TOKEN` | the office's bearer token |
| `SHIIOO_RUN_ID` | the run this harness session belongs to |

## What the hook does

1. Reads all of stdin as JSON. Claude Code sends `{"tool_name", "tool_input",
   "tool_use_id", "session_id", …}`; Codex sends the equivalent for the
   shell command.
2. `POST {SHIIOO_SERVER_URL}/v1/runs/{SHIIOO_RUN_ID}/permissions` with
   `Authorization: Bearer {SHIIOO_API_TOKEN}` and the body `{tool_name,
   tool_input, tool_use_id?, session_id?}`. The HTTP timeout is
   `HOOK_TIMEOUT_SECS + 60` (6 h 1 min), longer than the server's own
   approval timeout, so the server always answers first.
3. Prints the decision in the shape the harness expects:

   ```json
   {"hookSpecificOutput":{"hookEventName":"PreToolUse",
     "permissionDecision":"allow",
     "permissionDecisionReason":"allowed by rule 3: git commands"}}
   ```

   `permissionDecision` is `allow` or `deny`; the reason is the server's
   (`rule N: …`, `allowed by the owner (…)`, `denied by the owner (…)`,
   `no answer within 6 hours`).
4. Exits 0. The decision travels in the JSON, not in the exit code.

### Failing closed

If anything goes wrong on the hook's side (no environment variables, the
server is down, a network error, a non-2xx response, unparseable JSON) the
hook prints

```json
{"hookSpecificOutput":{"hookEventName":"PreToolUse",
  "permissionDecision":"deny",
  "permissionDecisionReason":"shiioo could not decide: <error>"}}
```

and still exits 0. A harness that cannot reach Shiioo therefore cannot run
anything that needs permission; it does not fall back to its own judgement.

## What happens on the server

Described in full in
[approvals-and-capacity.md](../../shiioo-api/docs/approvals-and-capacity.md):
the call is logged as `PermissionRequested`, judged by `rules.toml`
(`PolicyDecision`), and, for `ask`, turned into an `ApprovalRequested` the
owner answers from Telegram or the dashboard. The hook request blocks for
the whole time; from the harness's point of view the tool call just takes a
while. If the owner does not answer within about six hours the server
denies with `by: "timeout"`.

## Trying it by hand

With an office running and a run id at hand:

```sh
export SHIIOO_SERVER_URL=http://127.0.0.1:7878
export SHIIOO_API_TOKEN=$(cat .shiioo/api-token)
export SHIIOO_RUN_ID=<run uuid>
echo '{"tool_name":"Bash","tool_input":{"command":"git status"}}' | shiioo hook pre-tool-use
```

A rule that matches answers immediately; an `ask` rule leaves the command
hanging until you tap Allow or Deny, and `GET /v1/runs/<id>` shows
`waiting_approval` in the meantime.

See also: [security design](../../../docs/design/security.md),
[governance design](../../../docs/design/governance.md),
[harness-adapters.md in shiioo-runtime](../../shiioo-runtime/docs/harness-adapters.md).
