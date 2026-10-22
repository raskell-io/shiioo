<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/shiioo-mark-dark.svg">
    <img src="assets/brand/shiioo-mark-light.svg" alt="Shiioo: Virtual Company OS" width="560">
  </picture>
</p>

<h1 align="center">Shiioo</h1>

<p align="center">
  <strong>Virtual Company OS.</strong> Hire AI agents, give them policies and budgets, and keep every decision on the record.<br>
  The open-source agent platform where every run is governed, replayable, forkable and provable.
</p>

Shiioo runs AI agents on any model and any machine, with tools, secrets, approvals and live streaming. It's built in Rust on Tokio, runs on-device from a single embedded database file (redb) or as a cluster on Postgres, and is designed around one rule: **the per-run event log is the only state.** Everything else follows from that:

- **Rock-solid execution.** Workers are stateless and hold fenced leases. Every write carries an epoch, and a stalled worker that wakes up after a takeover is rejected. Side-effecting tool calls are recorded as an intent before they run, so a crash never causes a blind retry of a `git push` or a payment.
- **Governed.** Policies as code, approval boards with quorum rules, budgets, and change management for configuration an agent proposes.
- **Forkable.** Rewind a run to any step, fork it with a different prompt, model or policy, and diff the branches, including the workspace files *(planned)*.
- **Replayable.** Re-run recorded production runs against a new model as an eval suite *(planned)*.
- **Provable.** Every event is hash-chained to the one before it. Tampering, gaps and reordering are detectable.
- **Bring your own harness.** Run Claude Code, Codex and others as runtimes through the Agent Client Protocol, with the same log, policies and replay *(planned)*.

> **Status: early.** This is a ground-up rewrite. Today the repo contains the execution kernel (event model, hash chain, leases with fencing, crash-recovery rules) and three store backends (redb, Postgres, in-memory) that pass one shared conformance suite. Start with the [architecture overview](docs/architecture.md), the [design docs](docs/README.md) and the [roadmap](docs/architecture.md#10-roadmap).
>
> The previous Shiioo codebase lives at [raskell-io/shiioo-legacy](https://github.com/raskell-io/shiioo-legacy). Its governance features are being ported onto the new kernel; see [`docs/legacy-port.md`](docs/legacy-port.md).

## Layout

| Crate | Purpose |
|---|---|
| `shiioo-core` | Event types, hash chain, leases, recovery rules, the `EventStore` trait. No I/O. |
| `shiioo-store` | Store backends: `RedbStore` (embedded, default), `PostgresStore` (cluster, `postgres` feature), `MemoryStore` (reference and simulation). |

## Development

```sh
cargo test
cargo clippy --all-targets --all-features

# Postgres backend: point the suite at a scratch database
SHIIOO_TEST_DATABASE_URL=postgres://user:pass@localhost/shiioo cargo test --all-features
```

## Brand

The mascot is an original character: a seasoned strategist in a three-piece suit who runs the company while you set the direction. "Virtual Company OS" is the tagline; "agent platform" is what Shiioo technically is. Marks, icons and the favicon are in [`assets/brand`](assets/brand), generated from the pixel-art sources in `assets/brand/source` by `python3 assets/brand/build.py`.

## License

Apache-2.0. See [LICENSE](LICENSE). The brand assets in `assets/brand` are covered by the same license, but the name and mascot identify the project: please don't use them to suggest endorsement.
