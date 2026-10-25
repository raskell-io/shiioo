# Runtimes and providers

What actually drives a run: the native agent loop or an external agent harness, the model providers behind them, and how context is assembled from the log.

Read [architecture.md](../architecture.md) first. Related: [execution.md](execution.md) (leases, turns, recovery), [tools-and-machines.md](tools-and-machines.md) (where tools run), [governance.md](governance.md) (budgets and approvals), [fork-and-replay.md](fork-and-replay.md) (why responses are recorded the way they are).

## 1. Two kinds of runtime *(planned)*

| | Native runtime | Harness runtime |
|---|---|---|
| What runs the loop | `shiioo-runtime` in a worker | An external harness (Claude Code, Codex, Gemini CLI, OpenCode, Goose) on a Machine |
| How Shiioo talks to it | Directly | Agent Client Protocol (ACP) through an adapter |
| Model calls | Shiioo calls the provider and records every response | The harness calls its model; Shiioo sees what ACP exposes |
| Tools | Shiioo tools, MCP | The harness's own tools; Shiioo MCP tools can be offered to it |
| Policies and approvals | Before every tool call | On every permission request the harness raises, and on Shiioo tools it uses |
| Replay and fork | Turn-level, deterministic | At the granularity the harness exposes; workspace snapshots still work |

An agent definition picks its runtime (`runtime: native` or `runtime: { harness: claude-code }`). Both write to the same event log, use the same leases, budgets, policies and audit.

## 2. The native runtime *(planned, `shiioo-runtime`)*

One turn, as executed by the worker that holds the run's lease:

1. **Fold** the log into the run state (transcript, pending calls, budget used).
2. **Recover** if this lease is a takeover: settle dangling intents (see [execution.md](execution.md) and [tools-and-machines.md §3.4](tools-and-machines.md)).
3. **Build context** (section 4).
4. **Check budget** before the call; stop or ask for approval if the estimate exceeds it.
5. **Call the model** through the capacity broker, streaming deltas to subscribers.
6. **Record** `ModelResponseRecorded` with the complete normalized response before acting on it.
7. For each tool call: policy decision, approval if needed, `ToolIntent`, execute, `ToolResult`.
8. Record `BudgetCharged`, take a workspace snapshot, then either loop, park (waiting for input or approval), or finish.

The loop is a plain async function over a `RunContext` that only touches the outside world through traits (`EventStore`, `Provider`, `ToolExecutor`, `Clock`), so the simulator in `shiioo-sim` drives it unchanged.

## 3. Harness runtime *(Claude Code and Codex built; ACP planned)*

Harnesses are the primary employees (ADR-0009). Shiioo drives them as subprocesses on a node and records what it sees; the harness executes its own tools. Anthropic's documented path for driving Claude Code from another program is its print mode, so that is what the adapter uses; the Agent Client Protocol (ACP, Zed's editor protocol) comes later for harnesses that only speak it.

### 3.1 Claude Code *(built, `shiioo-runtime::harness::claude_code`)*

`claude -p "<prompt>" --output-format stream-json --verbose --permission-mode <mode> --setting-sources <sources> --max-turns N [--model M] [--append-system-prompt S] [--allowedTools …] [--max-budget-usd X] [--resume <session>]`, run in the run's worktree, stdin closed. Each stdout line is one JSON object; the shapes were recorded from Claude Code 2.1.310 (`crates/shiioo-runtime/tests/fixtures/claude-code-read.jsonl`) and map onto events:

| Claude Code line | Event |
|---|---|
| `system`/`init` (session id, cwd, model, version) | `HarnessSessionStarted` |
| `assistant` (a Messages API message) | `ModelResponseRecorded` with the message verbatim, plus one `ToolIntent` per `tool_use` block (class by tool name: `Read`/`Glob`/`Grep`/… pure, `Edit`/`Write`/… idempotent, `Bash` and the rest side-effecting) |
| `user` with `tool_result` blocks | one `ToolResult` per block |
| `rate_limit_event` (five-hour and seven-day windows) | `CapacityObserved` |
| `result` (`success` or `error_*`, cost, usage, `permission_denials`) | `HarnessResult`, then `RunCompleted` or `RunFailed` |

Rules the adapter follows:

- **Subscription by default.** In print mode Claude Code prefers an `ANTHROPIC_API_KEY` in its environment over the owner's login, so subscription runs get that variable removed from the child environment (`HarnessBilling::Subscription`). `CLAUDE_CODE_OAUTH_TOKEN` and `CLAUDE_CONFIG_DIR` pass through for headless servers.
- **No inherited session.** The variables that identify a parent Claude Code session (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, …) are scrubbed, so Shiioo can itself run inside Claude Code.
- **Owner settings stay out.** `--setting-sources project` by default: the repo's `.claude` settings apply, the owner's personal hooks and MCP servers don't. (`--bare` would be cleaner but never reads OAuth credentials, so it can't serve subscription runs.)
- **Crash = resume.** A worker that dies mid-run leaves `HarnessSessionStarted` in the log; the next lease holder runs `--resume <session>` with a continuation prompt and `CLAUDE_CODE_RESUME_INTERRUPTED_TURN=1`. After five such starts the run fails. A worker that loses its lease kills its harness process immediately (the heartbeat flags the loss).
- **Workspaces.** A run on a repository gets `<data>/worktrees/<run>` on branch `shiioo/<run-prefix>`, created from the repo's HEAD and reused across resumes. Pushing is the harness's job and subject to its allow rules; merging is the owner's.

Permissions today: the harness's own mode (`acceptEdits` by default) and allow rules (`Bash(git *)` form). Unanswered prompts in print mode are denied, and the denials show up in `HarnessResult.permission_denials`. Phase 2 routes them to the owner: the documented way is a `PreToolUse` / `PermissionRequest` hook passed via `--settings`, answered by Shiioo's rules or a Telegram approval (`--permission-prompt-tool` exists too, but its contract is undocumented).

### 3.2 Codex *(built, `shiioo-runtime::harness::codex`)*

`codex exec --json -C <cwd> -c approval_policy="never" -c sandbox_mode="<sandbox>" [-m model] [--skip-git-repo-check] "<prompt>"`, run in the run's worktree, stdin closed (with stdin open Codex waits for "additional input" and never starts). Resume: `codex exec resume <thread_id> --json "<prompt>"`. Shapes recorded from Codex CLI 0.163.0 (`crates/shiioo-runtime/tests/fixtures/codex-command.jsonl`, `codex-edit.jsonl`):

**Sandbox decision.** Shiioo runs Codex with `approval_policy = "never"` (headless, so nothing could answer an approval prompt) and its OS sandbox **off** (`danger-full-access`) by default. Reason, found live: Codex's `workspace-write` sandbox makes the checkout's resolved git directory read-only (for a worktree, `<repo>/.git/worktrees/<run>`), so `git commit` fails with "Unable to create index.lock", and with `never` Codex cannot escalate; `sandbox_workspace_write.writable_roots` does not lift that protection. Shiioo's rules and the owner's approvals gate every command through the hook instead, the same posture as Claude Code; OS-level isolation is the job of the isolation levels (phase 4). `permission_mode = "workspace-write"` or `"read-only"` remain available for tasks that don't need to commit.

| Codex line | Event |
|---|---|
| `thread.started {thread_id}` | `HarnessSessionStarted` (harness `codex`, version from `codex --version`, the agent's model or `default`) |
| `item.completed` `agent_message {text}` | `ModelResponseRecorded` with `{"content":[{"type":"text","text":…}]}` |
| `item.started` `command_execution {command}` | `ToolIntent` (`Bash`, side-effecting, `{"command":…}`) |
| `item.completed` `command_execution {aggregated_output, exit_code}` | `ToolResult` (`is_error` when `exit_code != 0`); plus the intent if no `item.started` was seen |
| `item.completed` `file_change {changes}` | `ToolIntent` (`Edit`, idempotent on the patch digest) + `ToolResult` with a one-line summary |
| `item.completed` `mcp_tool_call`, `web_search` | intent + result, named `mcp__<server>__<tool>` / `WebSearch` |
| `reasoning`, `todo_list`, `error` items | ignored: summaries, plans and Codex's own notices (for example the hook-trust warning) are not steps of the run |
| `turn.completed {usage}` | `HarnessResult` (`success`, no cost: Codex reports tokens, not dollars) then `RunCompleted` with the last agent message |
| `turn.failed {error}` / top-level `error` | `HarnessResult` (`error`) then `RunFailed` |

Rules the adapter follows:

- **Sandbox from the permission mode.** `acceptEdits` (the default) → `workspace-write`, `default`/`plan` → `read-only`, `bypassPermissions` → `--dangerously-bypass-approvals-and-sandbox`; Codex's own names are accepted too. Codex's sandbox is real OS isolation (Seatbelt on macOS, Landlock/seccomp on Linux): commands may write only inside the worktree and have no network unless configured. `approval_policy="never"` because nobody is there to answer a prompt: a command the sandbox refuses fails, it is not escalated. (`untrusted` and `on-failure` are gone from current Codex.)
- **Shiioo's rules still decide** when the worker has a `HookEndpoint`: the adapter passes `-c features.hooks=true -c 'hooks.PreToolUse=[{matcher="^(Bash|shell)$",hooks=[{type="command",command="<shiioo> hook pre-tool-use",timeout=21600}]}]' --dangerously-bypass-hook-trust`. Codex's hook contract is the same as Claude Code's: stdin JSON with `tool_name`, `tool_input.command`, `session_id`, `turn_id`, `cwd`, `model` (no `tool_use_id` observed), stdout `hookSpecificOutput.permissionDecision` `allow`/`deny` with a reason (`ask` is parsed but unsupported). Verified live: a denying hook blocked `date` and Codex told the model. Hooks inherit the harness's environment, so the `SHIIOO_*` variables reach `shiioo hook pre-tool-use` unchanged. The matcher is limited to shell commands: `apply_patch` edits are bounded by the sandbox, and the rules know `Edit`/`Write`, not `apply_patch` (widen the matcher once the rules do).
- **Subscription by default.** `OPENAI_API_KEY` and `CODEX_API_KEY` are removed from the child environment under `HarnessBilling::Subscription`, so the saved ChatGPT login is used (`codex exec` "reuses saved CLI authentication"; an inline `CODEX_API_KEY` is the documented way to bill an API key for one run).
- **No system prompt or allow rules.** Codex has no `--append-system-prompt`; the agent's `system` text is prepended to the prompt as "Instructions: … Task: …". `tools`, `max_turns` and `max_budget_usd` have no Codex equivalent and are ignored.
- **Crash = resume**, as for Claude Code: the thread id from `HarnessSessionStarted` is resumed with the continuation prompt; a worker that loses its lease kills the process.

Limits: no cost figure (tokens only, in `HarnessResult.usage`); no rate-limit windows in the `exec` stream (see below); `item.updated` and streamed deltas are not consumed.

**Capacity data.** `codex exec --json` carries no usage windows. The experimental app-server protocol (`codex app-server`, JSON-RPC over stdio) has `account/rateLimits/read` and an `account/rateLimits/updated` notification (its JSON schema lists them; `codex app-server generate-json-schema`); that is the path for the capacity broker to learn Codex's remaining capacity. `~/.codex/config.toml` also has `memories.min_rate_limit_remaining_percent`, so Codex tracks these windows itself.

**Local models through Codex.** `codex exec --oss --local-provider ollama|lmstudio` runs the same harness against a local model server; an option for the `local model` employee kind if Shiioo's native loop is not wanted for a task.

Research sources (fetched 2026-10-25): non-interactive mode <https://learn.chatgpt.com/docs/non-interactive-mode>, config reference <https://learn.chatgpt.com/docs/config-file/config-reference>, hooks <https://learn.chatgpt.com/docs/hooks> (developers.openai.com/codex/* redirects there).

### 3.3 Other harnesses via ACP *(planned)*

OpenCode, Gemini CLI, Goose and others expose the [Agent Client Protocol](https://agentclientprotocol.com): JSON-RPC over stdio with `session/new`, `session/prompt`, `session/update` (messages and tool calls) and `session/request_permission`. The same event mapping applies, and permission requests go through Shiioo's rules exactly like hook calls.

### Honest limits

- **Determinism.** Shiioo doesn't see the harness's raw model calls, so it can't replay them exactly. Replay re-runs the harness from a snapshot with the recorded user messages; the outcome may differ.
- **Fork granularity.** Forks happen at turn boundaries the harness exposes, not inside a turn.
- **Permission coverage.** Actions a harness performs without asking are visible only as recorded tool calls. Isolation of the node, not the harness's goodwill, bounds them; unattended runs with real credentials want the `container` or `openshell` level.
- **Tool calls aren't Shiioo's.** `ToolIntent`/`ToolResult` from a harness are observations: Shiioo never re-runs or reconciles them. A tool call interrupted by a crash is the harness's to sort out when resumed.

Adapters are versioned per harness and tested against recorded output.

## 4. Context building *(planned)*

The prompt for a turn is assembled in a fixed order, from most stable to least stable, so provider prompt caches hit as often as possible:

| Order | Part | Changes when |
|---|---|---|
| 1 | Platform system prompt | Shiioo release |
| 2 | Agent instructions | Agent definition version |
| 3 | Tool definitions (pinned at run start) | Never within a run |
| 4 | Skills index (names + one-line descriptions) | Agent definition version |
| 5 | Project memory (retrieved notes) | Per run start |
| 6 | Transcript folded from the log | Every turn (append-only, so the prefix stays stable) |

Cache breakpoints are placed after parts 4 and 5 and at the end of the previous turn, for providers that support explicit breakpoints (Anthropic) or prefix caching (OpenAI, Gemini).

**Skills** use the `SKILL.md` format ported from shiioo-legacy: a folder with a markdown file whose front matter has a name and description, plus optional scripts and references. Only the index is in context; the full skill is loaded when the model asks for it (a `skill.load` tool, `Pure`).

**Compaction.** When the transcript nears the model's context limit, the runtime asks the model (or a cheaper one) to summarize a range of turns and records:

```rust
ContextCompacted { replaces: SeqRange, summary: String, model: String }
```

The original events stay in the log. Context building uses the summary in place of the range; replay, audit and fork still see everything. Compaction never covers the most recent turn or any unresolved tool call.

## 5. Providers *(Anthropic and OpenAI-compatible built; the rest planned)*

Providers serve Shiioo's **native loop** (`shiioo-runtime`): the runtime that runs local models, cheap triage and bulk work, and that can call cloud APIs as a fallback (ADR-0009). Harnesses bring their own model access and don't go through this layer.

### 5.1 Provider trait *(built)*

The trait in `shiioo_runtime::model` is deliberately small and non-streaming: one complete response per call, which is what the log records.

```rust
pub trait ModelProvider: Send + Sync {
    fn complete<'a>(&'a self, request: ModelRequest<'a>)
        -> BoxFuture<'a, Result<ModelResponse, ProviderError>>;
}
// ModelRequest { model, transcript: &Transcript, tools: &[ToolSpec] }
// ModelResponse { model, content: Vec<Value>, stop_reason, usage }
// ProviderError::{Config, Http, Status { status, body }, Decode}
```

Streaming deltas to subscribers (the `delta` SSE frames in [api.md](api.md)) are planned and will be an addition, not a change: the recorded response stays the complete one.

### 5.2 Normalized format *(built)*

`ModelResponse.content` uses the **Anthropic Messages content-block shapes** as the canonical recorded form: `{"type":"text","text"}` and `{"type":"tool_use","id","name","input"}`, with whatever else a provider returns (thinking blocks, citations) kept verbatim. Reasons:

- The Anthropic provider can send its own blocks back unchanged, which that API requires (thinking signatures, for example).
- Claude Code's stream uses the same shapes, so harness runs and native runs read identically in the log and in the dashboard.
- Every other provider's format is a projection of these blocks and back: text becomes `content`, `tool_use` becomes a `tool_calls` entry, results become `tool` messages.

Tool results are recorded as `ToolResult { call_id, output, is_error }` and rebuilt into each provider's result message by the adapter. `StopReason` is normalized to `EndTurn | ToolUse | MaxTokens | Refusal | Other(String)`; `Usage` to input and output tokens.

### 5.3 Adapters

| Adapter | Where | Covers | Status |
|---|---|---|---|
| `AnthropicProvider` | `shiioo-runtime::anthropic` | Claude models via the Messages API (raw HTTP): explicit `effort`, server-side refusal fallback, OAuth or API key, retries with `Retry-After` | built |
| `OpenAiCompatProvider` | `shiioo-providers` | any server speaking Chat Completions with tools: **Ollama**, **LM Studio**, **`mlx_lm.server`**, vLLM, OpenRouter, LiteLLM, and OpenAI itself | built |
| `ProviderRouter` | `shiioo-providers` | picks a provider from the model name's prefix | built |
| Responses API, Bedrock (Converse), Gemini / Vertex | | | planned, as needed |

`OpenAiCompatProvider` details:

- `POST {base}/chat/completions`, non-streaming; `tools` as `function` definitions with the tool's JSON schema as `parameters`, `tool_choice: "auto"` when tools are offered.
- Response mapping: `finish_reason` `stop`→`EndTurn`, `tool_calls`→`ToolUse`, `length`→`MaxTokens`, `content_filter`→`Refusal`. Tool-call `arguments` may be a JSON string (OpenAI) or an object (some local servers); both are accepted, and unparsable strings are kept as `{"_raw": …}`. Servers that omit tool-call ids get synthesized `call_<n>` ids, since the loop needs one per call.
- Error tool results are prefixed with `Error:` in the `tool` message, because Chat Completions has no error flag.
- Retries: 429, 408, 409, 5xx and connection errors, up to four attempts with backoff, honouring `Retry-After`. Bad requests are never retried.
- Constructors: `ollama()` (`http://127.0.0.1:11434/v1`), `lm_studio()` (`http://127.0.0.1:1234/v1`), `openai(key)`, or `new(url, key)` for anything else, including a DGX Spark serving models over the tailnet.

`ProviderRouter`: the model name carries the provider as a prefix, `ollama/qwen3:32b`, `lmstudio/…`, `openai/gpt-5.5`; the longest matching prefix wins and is stripped before the call; names without a known prefix (`claude-opus-5-5`) go to the default provider. `shiioo_providers::from_env(default)` builds the office's router: `ollama/` (URL from `SHIIOO_OLLAMA_URL`), `lmstudio/` (`SHIIOO_LMSTUDIO_URL`), and `openai/` when `OPENAI_API_KEY` is set. An agent's `model` field is all it takes to choose: `model = "ollama/qwen3:32b"`.

Local models through `OpenAiCompatProvider` are first-class: `shiioo up` with Ollama is a fully offline setup for native runs.

### 5.4 Capacity broker *(planned)*

Ported from shiioo-legacy's `capacity.rs`. A project configures **pools**: ordered lists of endpoints and keys per model family.

```toml
[[pools]]
name = "claude"
strategy = "least-loaded"            # or "ordered", "round-robin"
members = [
  { provider = "anthropic", key = { secret = "ANTHROPIC_KEY_A" }, rpm = 4000 },
  { provider = "anthropic", key = { secret = "ANTHROPIC_KEY_B" }, rpm = 4000 },
  { provider = "bedrock", region = "eu-central-1", model_map = { "claude-sonnet" = "…" } },
]
fallback = "local"                   # another pool, only if the agent's policy allows switching models
```

- Tracks rate-limit headers and in-flight requests per member; a member that returns a rate limit is cooled down until `retry_after`.
- Retries overload and transport errors with jittered backoff, on another member when possible. Never retries invalid requests.
- **Failover to a different model** (a fallback pool) changes behavior, so it requires the agent's policy to allow it, and is recorded in the response event.
- Each response's usage is priced with the member's price table and recorded as `BudgetCharged { tokens, cost, pool, member }` (new), which is what budgets enforce (see [governance.md](governance.md)).
- In a cluster, rate-limit state is per worker with a shared view in the store, refreshed every few seconds; exact global limiting isn't attempted.

## 6. Agent definitions as configuration *(planned)*

An agent is versioned configuration: instructions, runtime, model pool, tools and MCP servers, skills, policy set, budget, isolation requirement, machine selector.

```toml
# agents/reviewer.toml
name = "reviewer"
runtime = "native"
pool = "claude"
model = "claude-sonnet"
instructions = "file:instructions/reviewer.md"
tools = ["fs.read", "search", "shell", "mcp.github.*"]
skills = ["skills/code-review"]
policy = "policies/reviewer.cedar"
budget = { per_run_usd = 2.0, per_day_usd = 40.0 }
isolation = "container"
machines = { labels = { pool = "ci" } }
```

- Definitions can be edited through the API or synced from a git repository (GitOps). Each change creates a new immutable version; a run records the version it started with.
- Agents may propose changes to definitions (including their own). Those go through change management and approval like any other risky action; see [governance.md](governance.md).

## 7. Open questions

- Whether harness runs should be allowed to use Shiioo's capacity broker by proxying their model calls (better budgets and replay, but depends on each harness supporting a custom endpoint).
- Memory: retrieval design (embeddings in the store vs an external vector index) is not decided.
