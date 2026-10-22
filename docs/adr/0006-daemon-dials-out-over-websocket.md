# 0006. Machines run a daemon that dials out over WebSocket

- Status: Accepted
- Date: 2026-10-22

## Context

Tools (shell, files, builds, tests) must run where the code and environment are: a developer's laptop, a CI box, a cloud sandbox. Those machines are usually behind NAT or firewalls. Opening inbound ports is a security and support burden. The connection carries tool calls that must not be lost or run twice.

## Decision

- Every machine runs `shiioo daemon`, which dials **out** to a gateway. No inbound ports.
- Transport: WebSocket over TLS (passes corporate proxies and load balancers). QUIC may be added later as an option.
- Frames are protobuf messages defined in `shiioo-protocol`, versioned, with capability negotiation in the opening handshake.
- Machines enrol with a one-time token and then authenticate with a long-lived, rotatable credential.
- The daemon keeps a local, fsynced journal of tool calls by `call_id`. It deduplicates redelivered calls and answers "did call X run, and what was the result?" after a worker crash.

## Consequences

- Works anywhere with outbound HTTPS.
- Gateways hold long-lived connections, so in a cluster the system must track which gateway holds which machine.
- The daemon journal turns most "outcome unknown" cases after a crash into a definite answer.
- protobuf adds a schema compiler to the build, in exchange for safe evolution of a protocol spoken by daemons we don't upgrade in lockstep.

## Revisit when

Connection counts or latency make QUIC worth its operational cost, or WebSocket proves unreliable through common proxies.
