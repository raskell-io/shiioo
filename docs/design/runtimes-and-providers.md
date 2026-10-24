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

## 3. Harness runtime *(Claude Code built; Codex and ACP planned)*

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

### 3.2 Codex *(planned)*

`codex exec --json [--worktree] [-s workspace-write] [-C dir] "<prompt>"` emits `thread.started`, `turn.started`, `item.completed` (agent messages, commands, file changes) and `turn.completed` with usage (`crates/shiioo-runtime/tests/fixtures/codex-ok.jsonl`); `codex exec resume <thread>` continues a thread. Same mapping, same rules.

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

## 5. Providers *(planned, `shiioo-providers`)*

### 5.1 Provider trait

```rust
pub trait Provider: Send + Sync {
    fn id(&self) -> &ProviderId;
    fn models(&self) -> &[ModelInfo];            // context window, pricing, capabilities
    async fn stream(
        &self,
        req: &ModelRequest,                       // normalized
        deltas: &mut dyn DeltaSink,               // ephemeral token stream to subscribers
    ) -> Result<ModelResponse, ProviderError>;    // complete, normalized
}

pub enum ProviderError {
    RateLimited { retry_after: Option<Duration> },
    Overloaded,
    ContextTooLong,
    InvalidRequest(String),   // not retried
    Auth,                     // not retried; key disabled in the pool
    Transport(String),        // retried
}
```

### 5.2 Normalized format

Events store a provider-neutral form: messages with text, image and document parts, tool calls (`call_id`, name, input JSON), tool results, stop reason and usage (input, cached input, output tokens). The **raw provider response** is stored as a blob and referenced from `ModelResponseRecorded` by digest, so nothing provider-specific is lost and the hash chain still covers it.

Provider-specific features that don't normalize cleanly (extended thinking blocks, citations, server-side tools) are kept as typed extension parts and passed back verbatim on the next turn when the same provider is used.

### 5.3 Adapters

| Adapter | Covers |
|---|---|
| `anthropic` | Claude models via the Messages API, including prompt caching and extended thinking |
| `openai` | Responses API (preferred) and Chat Completions |
| `openai-compatible` | Ollama, vLLM, LM Studio, OpenRouter, LiteLLM, and any server speaking Chat Completions |
| `bedrock` | AWS Bedrock (Converse API) |
| `gemini` | Google Gemini API and Vertex AI |

Local models through `openai-compatible` are first-class: `shiioo up` with Ollama is a fully offline setup.

### 5.4 Capacity broker

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

- Tracks rate-limit headers and in-flight requests per member; a member that returns `RateLimited` is cooled down until `retry_after`.
- Retries `Overloaded` and `Transport` errors with jittered backoff, on another member when possible. Never retries `InvalidRequest`.
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
