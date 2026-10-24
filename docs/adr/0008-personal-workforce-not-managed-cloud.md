# 0008. The product is a personal agent workforce, not a managed agent cloud

- Status: Accepted
- Date: 2026-10-24

## Context

"Agent platform" covers several different products: model APIs, agent frameworks, harnesses, durable execution engines, sandboxes, managed agent clouds, fleet supervision tools, and agent workforce / governance products. The first design docs leaned towards a managed agent cloud (organisations, tenants, Postgres clusters, SDKs), where Omnara and Claude Managed Agents already exist and compete on breadth.

The actual need is different: one owner with an always-on machine, subscriptions to frontier coding agents, and work to delegate, who wants to direct and supervise agents from a phone while away. See [positioning.md](../positioning.md).

## Decision

Shiioo is a **personal agent workforce**: single owner, local-first, autonomous by default, approvals by exception, phone-first. The kernel stays general enough to be used headless, but the product, the roadmap and the defaults serve the single-owner case.

Dropped or deferred: organisations and tenants, Biscuit delegation, Cedar quorum boards, OpenAPI-generated SDKs, Docker Compose deployment, a Postgres-first cluster story.

## Consequences

- The roadmap is reordered around what the owner can do from a phone at the end of each phase.
- Rules become simple (allow / ask / deny per tool, repo and action) with "ask" routed to the owner; the richer governance from legacy Shiioo returns only if several owners share an office.
- Breadth (many providers, integrations, sandbox vendors) is not a goal. Harnesses bring their own breadth.
- A hosted, multi-tenant offering would be a new product on the same kernel, not an extension of this one.

## Revisit when

A second kind of user shows up with a need that the single-owner model can't express, or the owner wants to share the office with a team.
