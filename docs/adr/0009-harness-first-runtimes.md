# 0009. Harnesses are the primary runtime; the native loop serves local models

- Status: Accepted
- Date: 2026-10-24

## Context

The first runtime built was a native agent loop calling the Anthropic Messages API. That bills API credits. The owner's capacity, however, is in subscriptions (Claude Max, OpenAI), which are only usable through their own programs: Claude Code and Codex. Those programs are also the most capable coding agents available, and they improve without any work on Shiioo's side.

## Decision

- **Harnesses are the primary employees.** Claude Code and Codex run as subprocesses on a node, driven through their headless / JSON interfaces or the Agent Client Protocol, with Shiioo recording their steps as events and routing their permission requests through Shiioo's rules.
- **The native loop is the local-model runtime.** It runs against Ollama, MLX or LM Studio on the owner's hardware for planning, triage, summaries and bulk work, and can call cloud APIs as a fallback.
- **Capacity is brokered** across subscriptions and local models: Claude Code first, Codex next, local models or the queue after.

## Consequences

- The harness adapter work moves from phase 3 to phase 1.
- Fork and replay are weaker for harness runs (replay only at the granularity the harness exposes); accepted.
- Shiioo depends on interfaces it doesn't control. Each adapter is versioned against a harness version and tested against the real binary in CI.
- Secrets and sandboxes matter more: a harness running unattended holds real credentials.

## Revisit when

Subscriptions become usable through an API that Shiioo's native loop can call directly, or a harness stops offering a scriptable interface.
