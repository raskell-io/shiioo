# Shiioo

**The open-source agent platform where every run is governed, replayable, forkable and provable.**

Shiioo runs AI agents on any model and any machine, with tools, secrets, approvals and live streaming. It's built in Rust on Tokio and Postgres, and is designed around one rule: **the per-run event log is the only state.** Everything else follows from that:

- **Rock-solid execution.** Workers are stateless and hold fenced leases. Every write carries an epoch, and a stalled worker that wakes up after a takeover is rejected. Side-effecting tool calls are recorded as an intent before they run, so a crash never causes a blind retry of a `git push` or a payment.
- **Governed.** Policies as code, approval boards with quorum rules, budgets, and change management for configuration an agent proposes.
- **Forkable.** Rewind a run to any step, fork it with a different prompt, model or policy, and diff the branches, including the workspace files *(planned)*.
- **Replayable.** Re-run recorded production runs against a new model as an eval suite *(planned)*.
- **Provable.** Every event is hash-chained to the one before it. Tampering, gaps and reordering are detectable.
- **Bring your own harness.** Run Claude Code, Codex and others as runtimes through the Agent Client Protocol, with the same log, policies and replay *(planned)*.

> **Status: early.** This is a ground-up rewrite. Today the repo contains the execution kernel (event model, hash chain, leases with fencing, crash-recovery rules) and an in-memory store with conformance tests. See [`docs/architecture.md`](docs/architecture.md) and the [roadmap](docs/architecture.md#roadmap).
>
> The previous Shiioo ("Virtual Company OS") lives at [raskell-io/shiioo-legacy](https://github.com/raskell-io/shiioo-legacy). Its governance features are being ported onto the new kernel; see [`docs/legacy-port.md`](docs/legacy-port.md).

## Layout

| Crate | Purpose |
|---|---|
| `shiioo-core` | Event types, hash chain, leases, recovery rules, the `EventStore` trait. No I/O. |
| `shiioo-store` | Store backends: `MemoryStore` (reference and simulation), Postgres next. |

## Development

```sh
cargo test
cargo clippy --all-targets
```

## License

Apache-2.0. See [LICENSE](LICENSE).
