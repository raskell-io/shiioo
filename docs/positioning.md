# What Shiioo is

**Shiioo is your virtual company: a staff of AI agents that work on your own machines around the clock, which you direct and supervise from your phone.**

This document is the product definition. The [architecture](architecture.md) serves it; where they disagree, this one wins and the architecture gets fixed.

## The picture

- **You are the owner.** You hand out work, approve the few things that need a human, and read the results. You never babysit a session.
- **Employees are agent harnesses:** Claude Code on your Claude subscription, Codex on your OpenAI subscription, local models on your own hardware. Each has a role, tools, limits and a workspace.
- **The office is your always-on machine** (a Mac Studio, to start): it holds the record, runs the agents, and keeps every decision and result.
- **Work comes in** as a message from your phone, a labelled GitHub issue, or a schedule. **Work goes out** as pull requests, files, reports and mail drafts.
- **Trust comes from the record:** every run is a log you can read, every side effect was either allowed by a rule you wrote or approved by you.

Single owner. Local-first. Autonomous by default, approvals by exception.

## What it is not

- Not a managed, multi-tenant agent cloud. No organisations, tenants or billing. Others (Omnara, Claude Managed Agents) do that well; Shiioo's kernel could serve it, but it is not the product.
- Not an agent framework or SDK. You don't write agents in Shiioo; you hire ones that exist.
- Not a chat app. Conversations happen, but the unit of work is a task with an outcome.

## Who it serves first

One person with real work to delegate, a capable always-on machine, and subscriptions to frontier coding agents. Teams come later, if ever, as several owners sharing one office.

## Principles

1. **Delegation must be effortless.** A two-line message from a beach should become a pull request. The planner, not the owner, turns vague asks into concrete runs.
2. **The phone is the primary interface.** Status in one glance, approvals in one tap, results in a few lines. Transcripts exist for when you want them.
3. **Nothing silently repeats or silently disappears.** A crashed worker, a slept laptop, a lost connection: the run continues from its log, side effects happen once, and the owner can always see what happened.
4. **Subscriptions and hardware are the scarce resources, not money.** Capacity is tracked and routed: Claude Code first, Codex next, local models and queues after.
5. **One binary.** The office, the nodes and the CLI are the same `shiioo` executable with an embedded database. No Postgres, no containers to run Shiioo itself. Sandboxes for *agents* are optional and layered.
6. **Rules over prompts.** What an agent may do is decided by Shiioo's rules and the owner's approvals, not by the agent's instructions.
7. **Built to last.** The kernel's guarantees (event log, fencing, recovery) are the part that must not change in a year; everything above it is replaceable.

## Topology

Like HashiCorp Nomad: a **server** holds state and makes decisions, **nodes** run work and dial out to the server. One binary, three roles.

```
   iPhone ──Telegram──▶ Telegram API ◀──long-poll──┐
   iPhone ──Tailscale──▶ https://office.tailnet ───┤
                                                   ▼
                              ┌──────── shiioo server ────────┐
                              │ event log (redb) · scheduler  │
                              │ rules & approvals · capacity  │
                              │ API + SSE · Telegram bot      │
                              │ planner (local model)         │
                              └───────────┬───────────────────┘
                     dial out, WebSocket + TLS over Tailscale
          ┌───────────────────┬───────────┴──────────┬──────────────────┐
     shiioo node          shiioo node            shiioo node        shiioo node
     (same process as     (DGX Spark #1:         (DGX Spark #2)     (a laptop, a CI box)
      the server on the    local inference,
      Mac Studio)          heavy jobs)
     runs harnesses, tools, sandboxes; owns workspaces (git worktrees)
```

- `shiioo up` runs server and node in one process: the whole system on one Mac.
- `shiioo node --server …` joins another machine. Nodes need no inbound ports.
- `shiioo` (CLI) and the phone talk to the server only.

## Employees

| Kind | How it runs | Pays with | Best for |
|---|---|---|---|
| Claude Code | harness subprocess on a node, driven headless | Claude subscription | serious software work |
| Codex | harness subprocess on a node | OpenAI subscription | second opinion, overflow |
| Local model | Shiioo's native loop against Ollama / MLX / LM Studio on the node | electricity | planning, triage, summaries, bulk and private work |
| Other harnesses (OpenCode, Gemini CLI, Goose) | via the Agent Client Protocol where supported | their own | as they prove useful |

The native loop can also call the Anthropic and OpenAI APIs directly, which bills API credits rather than the subscriptions; that is a fallback, not the default.

## Isolation levels

Chosen per agent and per task; the node reports which it supports.

| Level | What it is | Needs | Use |
|---|---|---|---|
| `worktree` | own git worktree and branch, host process, no sandbox | nothing | trusted repos, phase 1 |
| `container` | Apple containers on macOS, OCI container on Linux | Apple `container` / podman | untrusted dependencies, isolation from the home directory |
| `openshell` | NVIDIA OpenShell sandbox: Landlock, seccomp, egress policy, credential placeholders | OpenShell gateway (VM on macOS) | unattended work with real credentials, strongest |

Secrets (GitHub tokens, mail credentials) are injected at execution time and never shown to the model, at every level that can enforce it.

## The owner's surfaces

- **Telegram** (primary): delegate ("fix the flaky test in sango"), approve with one tap, get told when something finished or needs you, ask "what's everyone doing?". The bot long-polls Telegram, so the office needs no public address.
- **Mobile web dashboard** over Tailscale: runs, logs, diffs, capacity, the day's report. Served by the server, no app store.
- **GitHub**: label an issue to delegate it; review and merge the pull request in the GitHub app.
- **CLI** on any machine on the tailnet.

## Roadmap

Each phase ends with something new you can do from your phone.

1. **One employee, one repo** *(built except the dashboard)*. Claude Code as a harness on the Mac, a worktree and branch per run, pushes to a branch, run log in the dashboard over Tailscale. *Start a task, watch it, open the PR.*
2. **Approvals and notifications** *(built; Telegram verified against a fake server, live check pending)*. Rules (allow / ask / deny per tool, repo and action, `rules.toml`); harness permission requests routed through the rules to Telegram; notifications on finish and on "needs you"; `/run`, `/status`, `/log`, `/cancel`. *Approve a push or a risky command with one tap.*
3. **Staff and capacity.** Codex and local models as employees, the capacity broker across subscriptions, budgets per task, GitHub-issue intake. *Label an issue, come back to a PR; see remaining weekly capacity.*
4. **Unattended safety.** `container` and `openshell` levels, secrets injection, egress rules. *Let it run overnight.*
5. **The planner and the company.** A Chief-of-Staff agent that breaks goals into runs, routines on a schedule, a morning report. Roles as reusable configs. *"Ship the v0.3 milestone this week."*
6. **Beyond code.** Asset machines (audio, image, 3D), mail with send-approval, remote nodes (DGX Sparks).
7. **Replay and forks**, for improving agents' instructions once there is a history worth replaying.

## Decisions recorded

- [ADR-0008](adr/0008-personal-workforce-not-managed-cloud.md): the product is a personal workforce, not a managed agent cloud.
- [ADR-0009](adr/0009-harness-first-runtimes.md): harnesses are the primary runtime; the native loop serves local models.
- [ADR-0010](adr/0010-single-binary-server-and-nodes.md): one binary, server and nodes, embedded store; no Postgres or containers for Shiioo itself.
