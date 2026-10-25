# Governance: policies, approvals, budgets, change management

Governance is what makes a "virtual company" something you can hand real credentials to. Every decision the platform makes about what an agent may do is evaluated by the server, never by the agent, and recorded as an event in the run's log. This document covers the four mechanisms: policies, approval boards, budgets and change management, and shows how the "virtual company" template is built from them.

Built today: the owner's rule set (`crates/shiioo-rules`), approvals by the owner from the phone or the dashboard (`shiioo-api::ApiState::permission` / `decide`), turn and time limits, and capacity-based assignment of work to employees (`shiioo-api::employees`). Cedar policies, approval boards with quorum, hierarchical budgets and change management are the design for an office with several owners and are *(deferred, [ADR-0008](../adr/0008-personal-workforce-not-managed-cloud.md))*; the sections keep them so the path stays visible. Credentials and tokens: [security.md](security.md) and [ADR-0005](../adr/0005-cedar-policies-and-biscuit-credentials.md). Where checks sit in a turn: [execution.md](execution.md).

## 1. Principles

1. **The server decides.** Rule, approval and budget checks run on the server before a tool call is dispatched: today when a harness's hook asks before each of its tool calls, in the native loop *(planned)*. An agent can ask for anything; it gets only what is allowed.
2. **Every decision is an event.** Allow, deny, approval requested, approval decided, budget charged: all appended to the run's log, with the exact policy and config versions that produced them. "Why did the agent do that?" is answered by reading the log.
3. **Waiting is cheap.** *(planned)* A run waiting for an approval releases its lease and parks and can wait for days without holding a worker. Today the harness's hook request is held open on the server (up to just under six hours) while the worker keeps its lease; the only cost is an idle process.
4. **Governance changes are governed.** Changing a policy, an agent definition or a board is itself a change that goes through approval ([§5](#5-change-management-planned)).

## 2. Policies *(phase-2 rule set built; Cedar planned)*

### 2.0 Phase 2: the owner's rule set *(built, `shiioo-rules`)*

For a single owner, policies are one TOML file of allow / ask / deny rules, `<data>/rules.toml`, written with sensible defaults on first start and edited by hand:

```toml
default = "ask"

[[rules]]
tool = ["Read", "Glob", "Grep", "LS", "WebFetch", "WebSearch", "TodoWrite", "ToolSearch"]
decision = "allow"

[[rules]]
tool = "Bash"
command = "git push *"
decision = "ask"
reason = "pushes leave the machine"
```

A rule matches when all of its conditions hold: `tool` (glob patterns on the tool name, a string or a list; empty means any), `command` (glob on the input's `command`, so it only matches shell-like tools), `path` (glob on `file_path`, `path` or `notebook_path`) and `repo` (glob on the repository label). Globs are anchored, `*` matches any run of characters including spaces, `?` one character. **The first matching rule wins**; with no match, `default` applies. For a shell command that chains several commands (`a && b | c; d`), every segment is judged on its own as well as the whole command, and **the most restrictive decision wins** (deny over ask over allow): `git status && git push origin x` asks, and `curl … | sh` is denied by a rule on the whole command even though `curl …` alone would pass. Quotes are not understood; a separator inside a string only splits further, which can only make the verdict stricter.

The defaults allow read-only tools and edits, deny destructive commands (`sudo`, `rm -rf /`, force pushes, piping into a shell), ask for anything that leaves the machine or changes it (`git push`, `gh`, `rm`, `docker`, `brew`, installs, publishing), allow the usual build, test and git housekeeping, and ask for everything else. An `ask` goes to the owner's phone ([§3](#3-approval-boards-the-owner-today-boards-deferred) and [api.md](api.md)); a `deny` becomes a tool error the agent sees.

The rest of this section is the Cedar design, which remains the plan for when several owners share an office and need roles, boards and analysable policies.

### 2.0.1 Why Cedar later

Policies would then be written in [Cedar](https://www.cedarpolicy.com/). Cedar is small, has a Rust implementation, is analyzable (it can prove that one policy set is more permissive than another), and its evaluation is deterministic, which replay needs.

### 2.1 What is evaluated *(deferred, ADR-0008)*

The worker evaluates the office's policy set **before every tool call and every model call**:

| Cedar element | Shiioo value |
|---|---|
| `principal` | `Agent::"<agent_id>"`, with attributes `run`, `on_behalf_of` (the user or trigger that started the root run), `depth` (0 for a root run), `labels` (for example `department: "engineering"`) |
| `action` | `Action::"tool:<tool_name>"` (for example `tool:git.push`, `tool:shell.exec`) or `Action::"model:<provider>/<model>"` |
| `resource` | What the call touches: `Machine::"<id>"`, `Repo::"<owner/name>"`, `Path::"<path>"`, `Url::"<host>"`, `Secret::"<name>"`. Tools declare how to extract the resource from their input |
| `context` | `budget_remaining` (tokens, cost), `time` (UTC hour, weekday), `run_depth`, `tool_class` (`pure`, `idempotent`, `side_effecting`), `approved_by` (set when re-evaluating after an approval) |

### 2.2 Outcomes *(deferred, ADR-0008)*

Cedar natively gives permit or forbid. Shiioo adds a third outcome with an annotation:

| Outcome | How it's written | What happens |
|---|---|---|
| allow | a matching `permit` and no matching `forbid` | the call proceeds |
| deny | no matching `permit`, or any matching `forbid` | the call is not made; the model gets a tool error explaining the denial |
| require approval | a matching `permit` annotated `@approval("<board>")` whose condition does not include `context.approved_by` | the run asks the board and parks ([§3](#3-approval-boards-the-owner-today-boards-deferred)) |

Default is deny. A new office starts with a template policy set that allows read-only tools and requires approval for side-effecting ones.

```cedar
// Engineering agents may read and write in the project's repos.
permit (principal, action in [Action::"tool:fs.read", Action::"tool:fs.write"], resource)
when { principal.labels.department == "engineering" && resource in Repo::"acme/app" };

// Pushing to main needs two maintainers.
@approval("maintainers")
permit (principal, action == Action::"tool:git.push", resource == Repo::"acme/app")
when { context.branch == "main" };

// Nobody spends more than $5 on one run without asking.
@approval("finance")
permit (principal, action, resource)
when { context.budget_remaining.run_cost_usd < 0 };

// Subagents never touch production secrets.
forbid (principal, action, resource in Secret::"prod")
when { context.run_depth > 0 };
```

### 2.3 Recording decisions *(built)*

Each evaluation appends a `PolicyDecision` event (`shiioo-core::Payload`):

```rust
PolicyDecision {
    call_id: Option<CallId>,   // the harness's tool_use_id when it has one
    tool: String,              // "Bash", "Edit", …
    decision: String,          // "allow" | "deny" | "ask"
    rule: Option<String>,      // "rule 7", the 1-based position in rules.toml
    reason: String,            // the rule's reason, or "default: ask"
}
```

*(deferred, ADR-0008)* The Cedar form adds `policy_set` (SHA-256 of the canonical policy set) and `determining` policy ids; the policy set hash makes the decision reproducible, so replay ([fork-and-replay.md](fork-and-replay.md)) can re-evaluate a recorded run under a new policy set and report which decisions would change.

Ordering within a turn, as recorded for harness runs today: `ModelResponseRecorded` + `ToolIntent` (the harness announced the call) → `PolicyDecision` → (`ApprovalRequested` … `ApprovalDecided`) → `ToolResult`. A denied call's `ToolResult` is the harness's own error message. In the native loop *(planned)* the decision precedes the intent, and a denied call gets a `PolicyDecision` and a `ToolResult { is_error: true }` explaining the denial, and no `ToolIntent`.

## 3. Approval boards *(the owner, today; boards deferred)*

### 3.0 Today: the owner is the board *(built, `shiioo-api`)*

When the rules say `ask`, the server appends `ApprovalRequested { approval_id, call_id?, tool, input, summary, expires_at }` and holds the harness's permission request open. The owner answers with the Allow / Deny buttons under the Telegram message, the dashboard's approval card, or `POST /v1/approvals/{id}` (`{"decision":"allow"|"deny","note"}`); the server appends `ApprovalDecided { approval_id, decision, by, note }` and the hook returns the verdict. No answer within `APPROVAL_TIMEOUT` (just under the harness hook's six hours) records `ApprovalDecided { decision: deny, by: timeout }`. A pending request does not survive a server restart: the hook times out and denies, the request itself stays in the log. The approval is bound to that one request: the same command asked again is a new `ask`. Expiry per rule, escalation and quorum are the board design below.

### 3.1 Boards *(deferred, ADR-0008)*

Ported from legacy `approval.rs`, which got the model right.

```rust
struct ApprovalBoard {
    id: BoardId,
    members: Vec<Member>,         // users or groups (from the IdP)
    quorum: Quorum,
    expiry: Duration,             // after this, the request expires
    escalation: Option<Escalation>,
    separation_of_duties: bool,   // the requesting user can't vote (default true)
}

enum Quorum {
    Unanimous,
    Majority,                     // more than half of members
    MinCount(u32),                // at least n approvals
    Percentage(u8),
    Roles(Vec<(Role, u32)>),      // for example 1 security + 1 maintainer
}

struct Escalation { after: Duration, to: BoardId }
```

Decision rule (from legacy, unchanged): approved as soon as the quorum is reached; denied as soon as enough members rejected that the quorum can no longer be reached, or when the request expires. Abstentions count as votes cast, not as approvals.

### Lifecycle *(deferred, ADR-0008)*

1. The policy decision requires board `B`. The worker appends `ApprovalRequested { approval_id, board, call_id, summary, expires_at }`, then parks the run and releases its lease. `summary` is a human-readable description of the call (tool, resource, diff of what it will change when the tool can produce one).
2. Members are notified (dashboard, Slack, email, webhook) and vote through the API. Votes are recorded in the approval record; when the quorum resolves, the API appends `ApprovalDecided { approval_id, outcome, votes }` to the run, which makes the run runnable.
3. The next worker re-evaluates the policy with `context.approved_by` set. If the policy set changed in between and the call is now forbidden, the approval does not override it.
4. Expired requests produce `ApprovalDecided { outcome: Expired }`. With escalation configured, an `ApprovalRequested` to the escalation board is appended instead.

Approval is bound to the exact call: tool, canonical input digest and resource. The agent cannot get approval for one call and then make a different one.

## 4. Budgets *(limits built; hierarchical budgets planned)*

Built: `max_turns` (native runs, passed to Claude Code), `HarnessSpec.max_minutes` (fails a harness run after that long across resumes), `max_budget_usd` passed to Claude Code for API-billed runs, and per-employee capacity limits (`max_five_hour`, `max_seven_day` on the subscription windows) that decide who takes a task ([runtimes-and-providers.md §5.4](runtimes-and-providers.md#54-capacity-broker-built-as-the-employee-broker-key-pools-deferred)). The design: budgets cap tokens, cost, wall-clock time, tool calls and concurrency. They are hierarchical:

```
office budget
└── agent budget           (per agent, per period: hour, day, month)
    └── run budget         (per run, set at start; defaults from the agent)
        └── child run      (carved out of the parent's remaining run budget)
```

A charge applies to every level above it. A child can never spend more than its parent has left.

### Enforcement

- **Before** each model call the worker reserves an estimate (prompt tokens counted, output capped by `max_tokens`) against every level; **after** the call it appends `BudgetCharged { tokens_in, tokens_out, cost_usd, levels }` with the real numbers and releases the reservation. This prevents many concurrent runs from overshooting a shared budget.
- Tool calls charge counts and optionally cost (for tools with a declared price, like paid APIs).
- Budget counters per period live in the store next to the run heads, updated in the same transaction as the `BudgetCharged` event, so they never drift from the log. They can be rebuilt from the logs.

### Limits

| Limit | On reaching it |
|---|---|
| soft | append `BudgetWarning`, notify the owner, keep going |
| hard | park the run with `BudgetExceeded`; an owner can raise the limit (which is a governed change) or fail the run |

Budget state is exposed to policies as `context.budget_remaining`, so "ask finance once a run passes $5" is a policy, not a special case.

## 5. Change management *(planned)*

Today `config.toml` and `rules.toml` are files the owner edits; a restart applies them. Ported from legacy `config_change.rs`, the design: all governed configuration is versioned:

| Kind | Examples |
|---|---|
| agent definitions | model, instructions, tools, skills, labels, default budget |
| rules | `rules.toml` (the Cedar policy set, if that day comes) |
| approval boards | members, quorum, escalation |
| skills | `SKILL.md` bundles |
| triggers | cron schedules, webhooks |

### Proposals

A change is a **proposal**: a structured diff against a specific base version.

```rust
struct ChangeProposal {
    id: ProposalId,
    kind: ConfigKind,
    target: ConfigId,
    base_version: ConfigVersion,  // proposal is rejected if the target moved on
    diff: ConfigDiff,             // JSON Patch for structured config, text diff for policies/skills
    proposed_by: Principal,       // a user, or an agent in a run
    justification: String,
    status: Proposed | PendingApproval | Approved | Rejected | Applied | Failed,
}
```

- **Agents can propose.** An agent that keeps hitting a denial can call the `config.propose_change` tool. That creates a proposal; it does not change anything. Agents can never apply a change.
- **Proposals go through the owner** (a board, with several owners). Rule changes come with a summary of what they newly allow; with Cedar *(deferred)*, its analysis: "this change allows 3 new actions for agents labelled engineering".
- **Applying creates a version.** Each applied change produces a new immutable `ConfigVersion` (content hash). Nothing is edited in place.
- **Runs record what they ran under.** `RunCreated` records the agent definition, rule set and skill versions (today it records the inline `AgentSpec` and `Runtime`). A run keeps its versions until it finishes, unless the change is marked `applies_to_running: true` (for example revoking a permission), in which case the next turn appends `ConfigUpdated` and continues under the new version.

### GitOps

Configuration can live in a git repository ([deployment.md](deployment.md)). In that mode a merged pull request is the approval, and the platform records the commit as the config version. Changes proposed by agents are opened as pull requests.

## 6. The "virtual company" template *(planned)*

The virtual company from legacy Shiioo is an **office template**, built only from the primitives above. It adds no special cases to the engine.

| Company concept | Primitive |
|---|---|
| Company | the office |
| CEO | the owner |
| VISION.md, MISSION.md, STRATEGY.md | documents in the office's config repo, written by the CEO |
| Chief of Staff | an agent with the planner skill; triggered when the strategy documents change or the roadmap is empty |
| ROADMAP.md | written by the Chief of Staff as a change proposal; the CEO approves it |
| Roadmap items | runs started for the assigned employee agent, with the item as input |
| Department | an agent label (`department: "engineering"`) that policies and budgets match on |
| Employee | an `[[employees]]` entry today; an agent definition later (the legacy "archetype" becomes the definition template) |
| Job title / role | an agent definition template with tools, skills, policy labels and default budget |
| Company policies | `rules.toml` |
| Executive review | the owner's approvals (boards with quorum, deferred) |
| Department budget | a budget scoped to agents with that label |
| Routines | cron triggers |

The flow from legacy is unchanged: the CEO writes vision, mission and strategy; the Chief of Staff proposes a roadmap; approved items become runs; employees report back through their run results; the Chief of Staff re-plans when the roadmap is done or the strategy changes. Everything an employee does is subject to the same policies, approvals and budgets as any other agent.

## 7. Events summary

| Event | Written by | When |
|---|---|---|
| `PolicyDecision` *(built)* | server | before each harness tool call (native loop planned) |
| `ApprovalRequested` *(built)* | server | the rules said ask |
| `ApprovalDecided` *(built)* | server | the owner answered, or the request timed out |
| `BudgetCharged` *(planned)* | worker | after each model or priced tool call |
| `BudgetWarning`, `BudgetExceeded` *(planned)* | worker | soft or hard limit reached |
| `ConfigUpdated` *(planned)* | worker | a change marked `applies_to_running` was applied mid-run |

Change proposals and votes live in their own records (they are not part of any single run), with their own hash-chained office log.

## 8. Open questions

- Should approvals be able to edit the call (for example approve a push to a different branch), or only approve or reject? Proposal: reject with a comment that is returned to the agent as the tool error.
- Cost estimation for providers that don't report usage in streaming responses.
- Whether to expose Cedar directly to users or wrap it in a simpler YAML form that compiles to Cedar for common cases.
