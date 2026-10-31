# 0013. A WebSocket for the takeover terminal

- Status: Accepted
- Date: 2026-10-31

## Context

Taking over a run ([desk.md](../design/desk.md) R10–R13, R15–R18) gives the owner the harness's own terminal: Claude Code or Codex running interactively under a PTY on the office machine, resumed from the run's session, with the same hook configuration as the headless driver, so the rules still judge every tool call. The desk shows that terminal in a pane.

A terminal is bidirectional and sensitive to latency: every keystroke goes to the PTY, every byte of output comes back, and an echo should arrive within 50 ms on the tailnet (R16). It also needs a few control messages: the window size when the pane is resized, and the end of the takeover.

[ADR-0004](0004-rest-and-sse-api.md) makes REST plus resumable SSE the only public API, and names this as its "revisit when": a client that needs bidirectional streaming over one connection, which SSE plus POST can't serve. One POST per keystroke on top of an SSE stream for the output would work on paper. In practice it costs a request for every key typed, it can deliver keys out of order across connections, and it needs a sequencing protocol to put them back in order. A WebSocket is the plain tool for this.

[ADR-0006](0006-daemon-dials-out-over-websocket.md) used WebSockets for the machine daemon; [ADR-0012](0012-nodes-are-workers-over-the-offices-api.md) replaced that with long-polling over the API. So the office serves no WebSocket today, and adding one is a real change to the API's surface.

## Decision

- The office serves **one** WebSocket endpoint: `GET /v1/runs/{run}/terminal?ticket=<ticket>`. Everything else stays REST and SSE under ADR-0004.
- **Attach tickets.** `POST /v1/runs/{run}/takeover` (office token, `Idempotency-Key`) starts or joins the takeover and returns a ticket: random, single use, valid for 30 seconds, bound to the run. The WebSocket upgrade is accepted only with a fresh ticket. That way the URL never carries the office token: browsers can't set an `Authorization` header on a WebSocket, and a URL with a token ends up in logs and history. A used or expired ticket gets `401` before the upgrade.
- **Frames.**
  - Binary frames carry PTY bytes as they are, in both directions.
  - Text frames carry small JSON control messages:
    - `{"type":"resize","cols":…,"rows":…}` from the client;
    - `{"type":"ended","reason":…}` from the office, before it closes.
  - No other message types in this version; an unknown control message is ignored.
- **One process, any number of viewers.** The PTY belongs to the takeover, not to a connection. Closing the tab detaches; the process keeps running until the owner hands back, or until no client has been attached for the detach timeout (D6, 30 minutes). On attach, the office replays the last 64 KiB of output so the screen can be redrawn. Several clients may attach at once (desk and phone); all of them type into the same PTY.
- **On the record.** The takeover is recorded in the log, not on the socket: `TakeoverStarted { by }` and `TakeoverEnded { reason, transcript_blob? }` (desk T18). Every hook decision during the takeover is appended as for any harness run, and the transcript is imported from the harness's session file at the end. The socket stream itself is not an event. Whether to also keep it, capped and redacted, as a blob is D4.
- **Scope.** Only the office machine, in this version. Nodes reach the office by long-polling (ADR-0012) and have no streaming path to carry a terminal; takeover on a node waits for one (D7).

## Consequences

- Keystrokes and output share one ordered connection, and echo latency is the tailnet's round trip.
- The API now has two transports to secure, document and test. The ticket keeps the WebSocket's auth separate from the office token. The endpoint is listed in the OpenAPI document as an upgrade with its frame formats described in prose, since OpenAPI can't express frames.
- Proxies in front of the office must pass WebSocket upgrades. A tailnet does; a reverse proxy may need configuration, and the docs say so.
- The native client (desk.md section 10) attaches the same way, so a terminal widget can be shared between clients.
- What the owner types is not an event. The record of a takeover is what the agent did, as judged by the rules, plus the imported transcript; commands the owner runs through the harness's own shell escape bypass the hook (R11), and the docs say so plainly.

## Revisit when

- Takeover needs to work on remote nodes and their transport gains a streaming path.
- A second feature needs bidirectional streaming. At that point, decide whether one multiplexed socket per client beats one socket per feature.
