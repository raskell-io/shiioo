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

## 3. Harness runtime via ACP *(planned)*

The [Agent Client Protocol](https://agentclientprotocol.com) (started by Zed) is JSON-RPC over stdio between a client (normally an editor) and an agent harness. Shiioo plays the client role: the daemon starts the harness inside the run's sandbox and an adapter in `shiioo-tools` translates in both directions.

| ACP message | Shiioo event |
|---|---|
| `session/new`, `session/prompt` from Shiioo | `UserMessage` (new) delivered to the harness |
| `session/update` agent message chunks | Ephemeral deltas; the final message as `HarnessMessageRecorded` (new) |
| `session/update` tool call started / updated | `ToolIntent` / `ToolResult`, `class: SideEffecting` unless the harness marks it read-only |
| `session/request_permission` | Policy decision → allow, deny, or `ApprovalRequested`; the ACP answer is sent only after the decision is recorded |
| File system and terminal requests from the harness | Served by the daemon inside the sandbox, recorded like Shiioo tool calls |
| Turn end (`stopReason`) | Turn boundary: `BudgetCharged` (if the harness reports usage), `SnapshotTaken` |

What works fully: policies and approvals on everything the harness asks permission for, workspace snapshots, rewind and fork of the workspace, cost and time budgets, audit.

Honest limits:

- **Determinism.** Shiioo doesn't see the harness's raw model calls, so it can't replay them exactly. Replay re-runs the harness from a snapshot with the recorded user messages; the outcome may differ.
- **Fork granularity.** You can fork at any turn boundary the harness exposes, not inside a turn.
- **Permission coverage.** Actions a harness performs without asking (its own auto-approved tools) are only visible as tool-call updates. Isolation of the machine, not the harness's goodwill, is what bounds them; harness runs require isolation `process` or stronger.
- **Credentials.** The harness needs its own model credentials; they are injected as Secrets and never written to the log.

Adapters are versioned per harness and tested against recorded ACP sessions.

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
