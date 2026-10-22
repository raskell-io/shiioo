# 0005. Cedar for policies, Biscuit for agent credentials

- Status: Accepted
- Date: 2026-10-22

## Context

Governance is Shiioo's core value: deciding what an agent may do, what needs a human, and proving it afterwards. Legacy Shiioo had a custom policy engine. Per-tool allow/ask/block switches (as in other agent platforms) can't express rules like "deploys to production need two approvers outside business hours", and custom engines are hard to analyse and easy to get subtly wrong.

Agents also spawn subagents. A subagent must never get more power than its parent, even if the parent is tricked by prompt injection into asking for it.

## Decision

- **Policies are written in Cedar** (AWS, implemented in Rust). The worker evaluates them before every tool and model call. Outcomes: allow, deny, or require approval from a named board. Each evaluation is recorded as a `PolicyDecision` event with the hash of the policy set that made it.
- **Agent credentials are Biscuit tokens** (implemented in Rust). A run's token carries its project, agent and allowed scopes. A child run gets a token attenuated from its parent's, offline, and attenuation can only remove rights.
- Humans authenticate with OIDC and services with API keys; both map to Cedar principals.

## Consequences

- Policies are declarative, testable and analysable (Cedar can check policies for conflicts and gaps).
- Users learn Cedar's syntax. We ship templates for common rules and a policy playground in the dashboard.
- Delegation is safe by construction: no server-side bookkeeping is needed to stop privilege escalation through subagents.
- Two security-critical dependencies, both well-reviewed Rust crates.

## Revisit when

Cedar or Biscuit stop being maintained, or a policy need appears that Cedar can't express.
