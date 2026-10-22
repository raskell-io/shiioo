# 0004. REST + resumable SSE as the only public API

- Status: Accepted
- Date: 2026-10-22

## Context

Legacy Shiioo exposed REST, GraphQL and GraphQL-over-WebSocket subscriptions. Three protocols mean three sets of auth, error handling, docs and SDK code. The main interactive need is streaming a run's events, which must survive dropped connections without losing anything.

## Decision

- One public API: JSON over HTTP under `/v1`, described by a generated OpenAPI document.
- Streaming uses Server-Sent Events. Event IDs are the run's `seq`, so a client reconnects with `Last-Event-ID` and resumes exactly where it stopped, served from the log.
- Ephemeral output (model tokens, tool stdout as it happens) is sent as a separate SSE event type without an ID. It is a preview; the recorded event that follows is authoritative.
- Commands that change state accept an `Idempotency-Key`.
- No GraphQL. WebSockets only for the daemon protocol ([ADR-0006](0006-daemon-dials-out-over-websocket.md)).

## Consequences

- SSE works through proxies and load balancers and needs no client library; resume falls out of the event log.
- SDKs are generated from one spec (TypeScript first, then Python).
- Clients that want flexible queries get dedicated list and filter endpoints instead of GraphQL.

## Revisit when

A significant client needs bidirectional streaming over one connection that SSE plus POST can't serve.
