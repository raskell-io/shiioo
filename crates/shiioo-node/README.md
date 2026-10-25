# shiioo-node

A worker on another machine (`docs/design/tools-and-machines.md` §2,
[ADR-0012](../../docs/adr/0012-nodes-are-workers-over-the-offices-api.md)).
`shiioo node` runs the same `Worker` as the office, against a `RemoteStore`
that speaks the office's HTTP API, so leases, fencing and recovery hold
across the wire. The node dials out; the office never connects to it.

## Position in the crate graph

```
shiioo-core, shiioo-runtime  ──►  shiioo-node  ──►  shiioo (binary)
```

Depends on nothing but the core types and the runtime's `Worker`. The API
crate is a dev-dependency (the tests start a real office).

## API

- `RemoteStore::new(server_url, machine_token)`: `EventStore` over
  `POST /v1/runs/{id}/lease`, `…/lease/renew`, `…/events`, `GET …/events`,
  `GET /v1/runs`. `create_run` and `append_external` are refused: those are
  the office's. Store errors come back as `StoreError` through their JSON
  form (`StoreError::from_json`), so `Fenced`, `LeaseExpired`, `LeaseHeld`,
  `SeqConflict` and `RunClosed` mean what they mean locally.
- `enroll(server_url, enrollment_token, name, labels, capabilities, version)
  -> machine token`: `POST /v1/machines/enroll`, no other credential.
- `NodeConfig::new(name)` (`version`, `capabilities`, `poll_wait_secs` 25,
  `max_concurrent` 4) and `run_node(config, store, workers)`: heartbeat,
  long-poll `GET /v1/machines/{name}/work`, drive each run with a worker
  from the factory (three attempts on a lost lease), forever.

The binary wires it (`shiioo node enroll`, `shiioo node`): a `LocalMachine`
with the node's own journal, worktrees and sandboxes, the node's secret
store, its `[repos]` by label (`Worker::with_repos`), and the hook endpoint
pointing at the office with the machine token.

## Testing

```sh
cargo test -p shiioo-node
```

`tests/remote.rs` starts an office in-process, enrolls a node, addresses a
run to it by label, lets `run_node` drive it and checks the lease holder,
then exercises every fencing error across the wire.
