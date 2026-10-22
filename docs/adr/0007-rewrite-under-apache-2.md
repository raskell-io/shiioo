# 0007. Rewrite Shiioo from scratch under Apache-2.0

- Status: Accepted
- Date: 2026-10-22

## Context

Legacy Shiioo ([raskell-io/shiioo-legacy](https://github.com/raskell-io/shiioo-legacy)) has valuable domain logic (approval boards, budgets, change management, the capacity broker, skills) but a single-node core: events buffered in memory before being written, cluster locks in a process-local `HashMap`, an audit chain held in memory. It is licensed BUSL-1.1, which forbids running it as a managed service for third parties.

The goal is an open-source agent platform that competes with existing open-source and managed offerings on reliability and governance.

## Decision

- Start a new codebase, `raskell-io/shiioo`, around a new execution kernel. Rename the old repository to `shiioo-legacy`.
- License the new codebase Apache-2.0.
- Port legacy domain logic onto the new kernel module by module (see [legacy-port.md](../legacy-port.md)), once the legacy code is relicensed or ported by its copyright holders.
- Keep "Virtual Company OS" as the product tagline; "agent platform" is the technical description.

## Consequences

- Anyone can self-host, embed or offer Shiioo as a service. Hosted competitors are possible; we compete on quality and the hosted offering.
- Short-term feature regression versus legacy until governance modules are ported.
- A clean kernel designed for the guarantees in [architecture.md](../architecture.md), rather than retrofitted.

## Revisit when

Not expected. A license change later would need agreement from all contributors.
