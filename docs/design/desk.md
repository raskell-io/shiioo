# The desk: the web UI, and working with an employee directly

Status: **requirements draft, for the owner's review** (phase 8 candidate). Nothing here is built. Decisions marked **D1**…**D11** are open; the rest follows from the picture.

Related: [api.md](api.md) (the endpoints the desk uses), [`shiioo-api/docs/dashboard.md`](../../crates/shiioo-api/docs/dashboard.md) (the phone dashboard today), [conversation.md](conversation.md) (phase 7, which shares the parked-run work), [execution.md §1](execution.md#1-run-states-partly-built) (parked runs), [governance.md](governance.md) (rules and approvals), [runtimes-and-providers.md](runtimes-and-providers.md) (the harness drivers), [ADR-0004](../adr/0004-rest-and-sse-api.md) (REST and SSE).

## 1. The picture

The phone is how the owner runs the company while away. The desk is how they run it while sitting at a big screen: every repository and machine in a sidebar, what needs them floated to the top, several runs open side by side, and the diff next to the request that asks to push it. Most of the time the owner still delegates. Sometimes the boss walks over to an employee's desk and says "no, do it like this", or sits down at the keyboard for ten minutes. Both are part of the job, and both stay on the record.

Long term the desk is also a native app (in the spirit of Herdr: a sidebar of spaces and tabs, terminals for agents, a core that keeps working when the app closes). Shiioo already has that core: the office keeps the log, the leases and the workers, and survives restarts. So the web desk comes first, and it is built so that a native app is one more client of the same API, not a second product.

| Today | With the desk |
|---|---|
| A phone layout, also on a desktop | A phone layout on the phone, a desk layout on a wide screen, one codebase |
| Runs in one flat list | Runs grouped by repository and machine, "needs you" first |
| One run at a time | Several runs in tabs or split panes |
| Plans, routines, files, the report: Telegram only | All of them in the desk too |
| Delegate, approve, cancel | Also: send an employee instructions mid-run, follow up on a finished run, take over its session in a terminal |

Principles that carry over unchanged: the log is the only state; the rules gate every tool call, including the ones made while the owner is at the keyboard; the office has no public address (the desk is served on the tailnet, like the dashboard).

## 2. Scope

**In:**

- A responsive desk layout of the existing web UI, served by the binary, with the phone layout kept as it is.
- Views for the whole office: runs, approvals, plans, routines, capacity, deliverables, the report, machines, and the conversation thread once phase 7 exists.
- **Instructing an employee:** a message to a running or parked run, and a follow-up to a finished one.
- **Taking over:** a live terminal in the desk attached to the run's own harness session, in the run's worktree (or container), with the rules hook installed; handing back to the employee afterwards.
- API additions that any client can use, a native app included.

**Out (for this phase):**

- The native app itself. What this phase owes it is an API with nothing the web desk alone can do (R3).
- A general shell on the office machine. A takeover opens the harness, not a login shell (see R11 on harness shell escapes).
- Several people at the desk. One owner.
- Editing files in the browser. The worktree is a git branch; the owner edits it through the employee, through a takeover, or in their own editor.

## 3. Requirements

### The desk

- **R1. One UI, two layouts.** Below a width breakpoint the desk is today's dashboard. Above it: a sidebar, a main area with tabs, and an optional side panel. Same files, same routes, same token handling.
- **R2. The sidebar.** Grouped by repository (a "space"), each with its runs as entries; a **Needs you** group on top with every pending approval, plan awaiting approval and run needing attention, across all repositories; a **Machines** group with each node, online or not, and what it is running. State glyphs and the last-activity time as in the run list today. Unread marks for runs whose log moved since the owner last looked.
- **R3. The API is the contract.** Everything the desk shows or does goes through `/v1` endpoints described in [api.md](api.md) and the OpenAPI document. No UI-only routes, no server-rendered state. A native app, the CLI or `curl` can do anything the desk can.
- **R4. Several runs at once.** Runs open as tabs; two or more can be split side by side, each with its own live log over SSE. The run view adds a side panel for the diff and the approval card, so a request to push can be read against the change it would push.
- **R5. The rest of the office.** Pages for plans (read, approve, reject), routines (list, next fire time, run now, pause), capacity per employee and window, deliverables per run (preview images, PDFs, audio and video inline; download), the report, and the conversation thread with a text box (phase 7's R12).
- **R6. Keyboard first on the desk.** A command palette (new task, jump to run, approve or deny the focused request, cancel), and shortcuts for moving between tabs and "next thing that needs me". Touch-sized targets stay on the phone layout.
- **R7. Live without polling where it matters.** Run logs stream as today. **D1:** how the sidebar learns of new runs and state changes. Proposed: an office-wide SSE stream (`GET /v1/events`) of run lifecycle and approval events, resumable like the per-run stream, replacing the 10 s refresh.

### Instructing an employee

- **R8. Message a run.** The owner types an instruction into a run ("use the existing retry helper, don't add a dependency"). It is appended as `UserMessage { author: owner }` through `POST /v1/runs/{run}/messages` (planned in [api.md](api.md)) and shown in the log at once. **D2:** when a harness run receives it. Proposed: at the next turn boundary by default: the worker lets the current harness turn finish, then resumes the same session with the message as the prompt; a **Now** option interrupts the turn (stops the harness process and resumes with the message, the interrupted-turn resume the drivers already use after a crash). Native-loop runs read it at their next model call.
- **R9. Follow up on a finished run.** A completed or failed run is closed for good (the log does not reopen). "Follow up" starts a continuation run: same repository, same worktree and branch, the harness resumed from the finished run's session, the owner's message as the prompt, and `origin: { run, seq }` linking back. **D3:** confirm that continuation runs are the shape, rather than reopening.

### Taking over

- **R10. Take the desk.** From a run, **Take over** opens a terminal in the desk running the harness interactively in the run's worktree, resumed from the run's session (`claude --resume <session>`, `codex resume <thread>`), started by Shiioo with the same hook configuration the headless driver uses, so every tool call the agent makes during the takeover still goes through the rules. The owner types to the agent as they would in their own terminal. A running run stops at its turn boundary first (as in R8); the takeover then holds the run's lease, so no worker drives it while the owner is at the keyboard.
- **R11. On the record.** The log shows `TakeoverStarted { by: owner }` and `TakeoverEnded`, every hook decision in between as for any harness run, and, at the end, the takeover's transcript imported from the harness's session file as messages and tool calls, so `shiioo log` reads through the takeover like any other stretch of the run. **D4:** whether the raw terminal stream is also kept, as a blob referenced from `TakeoverEnded`. Proposed: yes, capped in size, redacted like every other output, for the cases the transcript import misses. Commands the owner runs directly through the harness's own shell escape (Claude Code's `!`) bypass the hook, since they are the owner's own commands; the transcript records them and the docs say so plainly.
- **R12. Approvals during a takeover.** An `ask` verdict while the owner is at the keyboard appears in the desk next to the terminal and on the phone as always; the first answer wins. **D5:** whether `ask` should turn into `allow` during a takeover, since the owner is present. Proposed: no; the rules stay what they say, and a one-tap answer in the desk is cheap.
- **R13. Hand back.** **Hand back** ends the interactive process, records `TakeoverEnded`, and either leaves the run parked for the next instruction or resumes it headless with an optional note ("carry on from here"). Closing the browser tab does not end the takeover; the session waits (D6) and can be re-attached from any client.
- **R14. Isolation and machines.** In a `container` run the terminal attaches inside the run's container (`exec` with a TTY), with the same mounts and egress as the headless run. On a remote node the terminal runs on the node. **D7:** remote-node takeover needs a streaming path the long-poll protocol ([ADR-0012](../adr/0012-nodes-are-workers-over-the-offices-api.md)) does not have. Proposed: takeover on the office machine first; nodes after the transport (D8) exists.

### Trust and non-functional

- **R15. Token scope.** A takeover gives interactive control of an agent, which is more than reading logs. **D9:** whether it needs more than the office token. Proposed: the same token, plus a short-lived, single-use attach ticket per terminal (`POST /v1/runs/{run}/takeover` returns it), so the WebSocket URL can't be replayed from a log or history; tailnet-only as for everything else.
- **R16. Latency.** Keystroke echo in the takeover terminal under 50 ms on the tailnet; the desk's first paint under 1 s on the Mac itself.
- **R17. No build step, still.** The desk stays plain files embedded with `include_str!`. The terminal widget (xterm.js) is vendored as one file with its licence. **D10:** whether the UI may grow a build step if the file count grows. Proposed: no; native ES modules, split by view, embedded per file.
- **R18. Restart safety.** An office restart during a takeover ends the interactive process; the run's log has `TakeoverStarted` without `TakeoverEnded`, recovery appends `TakeoverEnded { reason: office_restarted }`, imports what the session file holds, and leaves the run parked. Nothing runs headless without the owner saying so.

## 4. Architecture sketch

**The desk** is the existing `ui/` directory grown into modules: `app.js` keeps routing and the token; one module per view (runs, run, office pages, terminal); CSS gains a desk layout under a `min-width` media query. The sidebar folds the office-wide stream (R7) into a client-side model; nothing is cached on the server for the UI.

**Messages and parking** are the kernel work phase 7 needs as well: `RunParked { waiting_for: input }`, `UserMessage` appended externally, `RunWoken`. For harness runs the worker's turn boundary is the harness process exiting; the message becomes the `--resume` prompt, which the drivers already support (`claude_code::command` and `codex::command` take `resume` and a prompt). Building phase 7a first gives this phase its kernel for free, or the other way round.

**Takeover** is a new driver mode on the machine that owns the run: spawn the harness without `--print` / `exec`, under a PTY, with the same `--settings` hook (Claude Code) or `-c` hook config (Codex) and the same environment (`SHIIOO_RUN_ID`, the server URL, the token), in the run's worktree or container. The PTY's bytes go to the client and keystrokes come back. **D8:** the transport. A terminal is bidirectional and latency-sensitive, which is the "revisit when" of [ADR-0004](../adr/0004-rest-and-sse-api.md). Proposed: one WebSocket endpoint, `GET /v1/runs/{run}/terminal?ticket=…`, binary frames for PTY bytes and a small JSON control frame for resize, recorded in a new ADR that amends 0004 for this one endpoint. The rest of the API stays REST and SSE.

**For the native app later**, nothing extra: it reads the same `/v1` endpoints, streams the same SSE, and attaches to the same terminal WebSocket. The office is already the core that keeps agents alive when a client closes, and unlike a local terminal multiplexer it resumes them after a reboot from the log.

## 5. What it feels like

```
┌ Needs you ───────────────┐ ┌ sango · fix flaky net test ─────────┬ diff ─────────────┐
│ 🔐 sango  git push        │ │ … ran cargo test (3 failed)         │ tests/net.rs      │
│ 📝 plan: v0.3 milestone   │ │ … edited src/retry.rs               │ - for i in 0..3 { │
├ sango ───────────────────┤ │ 🔐 wants: git push origin shiioo/…  │ + backoff.retry(… │
│ ▶ fix flaky net test  2m │ │    [Allow] [Deny]                   │                   │
│ ✅ bump deps          1h │ ├─────────────────────────────────────┤                   │
├ shiioo ──────────────────┤ │ > use the existing backoff helper   │                   │
│ ▶ docs pass          30s │ │   [Send at next turn] [Now]         │                   │
├ Machines ────────────────┤ │   [Take over]  [Cancel]             │                   │
│ ● studio  2 runs         │ └─────────────────────────────────────┴───────────────────┘
│ ○ spark   offline        │
└──────────────────────────┘
```

Take over replaces the log pane with the harness's own terminal, a banner "you are at the keyboard · rules active · Hand back", and the approval card stays in the side panel.

## 6. Phasing

1. **8a. The desk layout** over the existing API: sidebar, tabs, split panes, the diff side panel, the command palette. The office-wide stream (R7).
2. **8b. The office pages:** plans, routines, capacity, deliverables, the report, machines.
3. **8c. Instructing:** parked runs and `UserMessage` (shared with 7a), `POST /v1/runs/{run}/messages`, turn-boundary and **Now** delivery, follow-up continuation runs.
4. **8d. Takeover on the office machine:** PTY driver mode for Claude Code and Codex, the terminal WebSocket and its ADR, attach tickets, transcript import, hand back, restart recovery.
5. **8e. Takeover in containers and on nodes,** once the node transport can stream.
6. **Later:** the native app, as a client of the same API.

## 7. Acceptance

- On a phone the dashboard looks and behaves as today; on a wide screen the sidebar shows every repository's runs, and a new approval appears under **Needs you** within a second without a reload.
- Everything the desk does is reproducible with `curl` against `/v1` (checked by a test that drives the API the way the desk does).
- A message sent to a running Claude Code run is delivered when the current turn ends, appears in the log as the owner's, and the agent's next turn responds to it; **Now** interrupts and the resumed session still has the earlier context.
- Follow-up on a completed run starts a new run on the same branch whose harness remembers the earlier session, and whose spec names the original run.
- During a takeover, a tool call that `rules.toml` says `ask` produces an approval card in the desk and a Telegram message; a `deny` is denied in the terminal. After hand back, `shiioo log <run>` verifies and shows the takeover's messages and tool calls between `TakeoverStarted` and `TakeoverEnded`.
- Killing the office during a takeover leaves the run parked with `TakeoverEnded { reason: office_restarted }` after restart, and no headless worker picks it up on its own.
- A terminal WebSocket URL with a used or expired ticket is refused.

## 8. Decisions needed

| | Question | Proposed |
|---|---|---|
| D1 | How the sidebar stays live | Office-wide resumable SSE, `GET /v1/events` |
| D2 | When a message reaches a busy harness run | Next turn boundary by default; **Now** interrupts |
| D3 | Follow-up on a finished run | A continuation run on the same branch and session, linked by `origin` |
| D4 | Keep the raw terminal stream as well as the imported transcript | Yes, capped and redacted, as a blob |
| D5 | Does `ask` become `allow` while the owner is at the keyboard | No |
| D6 | How long a detached takeover waits before ending itself | 30 minutes, then hand back to parked |
| D7 | Takeover on remote nodes | After the office machine, once the node transport streams |
| D8 | Terminal transport | One WebSocket endpoint, recorded in an ADR amending 0004 |
| D9 | Auth for a takeover | Office token plus a single-use attach ticket |
| D10 | A build step for the UI | No; native ES modules, embedded per file |
| D11 | Order with phase 7 (conversation) | Build the shared parked-run kernel once, in whichever phase comes first; 8a and 8b do not depend on it |

## 9. Tasks

One line each, in build order within a phase. "Needs" names what must land first; a task with no "needs" can start now. Each task ends with tests in the crate it touches and the docs it changes (the design doc's *(planned)* marks become *(built)*, the crate's `docs/` describe the code).

### 8a. The desk layout

| | Task | Where | Needs | Done when |
|---|---|---|---|---|
| T1 | Split `app.js` into native ES modules (core: token, `api()`, routing, SSE client; one module per view) and serve each embedded file under `/ui/` | `shiioo-api/ui/`, `lib.rs` UI routes, `tests/dashboard.rs` | | Phone dashboard unchanged; every module served without a token; D10 settled |
| T2 | Office-wide event stream `GET /v1/events`: run created, status changed, approval requested and decided, plan proposed; resumable with `Last-Event-ID` | `shiioo-api` (store notifications), `docs/http.md`, `docs/design/api.md` | | A test opens the stream, creates a run and an approval, reconnects mid-way and sees each event once; D1 settled |
| T3 | Desk layout: breakpoint, sidebar (Needs you, repos, machines), unread marks kept in `localStorage` | `ui/` | T1, T2 | Wide screen shows grouped runs; a new approval appears under Needs you without reload |
| T4 | Tabs and split panes, one live log per pane; the diff and approval card in a side panel | `ui/` | T3 | Two runs stream side by side; Allow in the side panel answers the hook |
| T5 | Command palette and shortcuts (new task, jump to run, next "needs me", allow / deny focused, cancel) | `ui/` | T4 | Every action reachable without the mouse |
| T6 | API-parity test: drive the flows the desk uses (create, stream, approve, cancel, diff) through `/v1` only | `shiioo-api/tests/` | T2 | Test passes; R3 holds by construction |

### 8b. The office pages

| | Task | Where | Needs | Done when |
|---|---|---|---|---|
| T7 | Plans page: list, read, approve; add `POST /v1/plans/{id}/reject` | `ui/`, `shiioo-api/src/plans.rs` | T3 | A plan can be approved or rejected from the desk; rejection is recorded |
| T8 | Routines page: list, next fire time, run now; add pause and unpause | `ui/`, `routines.rs`, `schedule.rs` | T3 | A paused routine does not fire; the state survives a restart |
| T9 | Capacity, report and machines pages over the existing endpoints | `ui/` | T3 | Same numbers as `/capacity` and the morning report |
| T10 | Deliverables per run: list, inline preview (image, PDF, audio, video), download | `ui/` | T4 | A run's files show in its side panel; closes the backlog item "dashboard has no views for machines, files or plans" |

### 8c. Instructing an employee

| | Task | Where | Needs | Done when |
|---|---|---|---|---|
| T11 | Kernel: `RunParked { waiting_for }` and `RunWoken` payloads; the status fold learns `Parked` | `shiioo-core/src/event.rs`, `shiioo-runtime/src/status.rs` | | Fold and sim tests cover park → message → wake; shared with phase 7a (D11) |
| T12 | `POST /v1/runs/{run}/messages` appending `UserMessage { author: "owner" }` with `Idempotency-Key`; a parked run is driven again | `shiioo-api/src/lib.rs`, `docs/http.md` | T11 | A message to a parked run wakes it once, also when retried |
| T13 | Harness delivery at the turn boundary: when the harness process exits with unread messages, resume the session with them as the prompt; otherwise park instead of completing (D2) | `shiioo-runtime/src/harness/mod.rs`, `worker.rs` | T11 | Scripted-harness test: a message sent mid-turn is the next resume prompt |
| T14 | **Now** delivery: stop the harness process, resume with the interrupted-turn flag and the message | `harness/mod.rs` | T13 | Real-binary check with Claude Code and Codex: context survives the interrupt |
| T15 | Follow-up on a finished run: `POST /v1/runs/{run}/follow-up` creates a continuation run (same repo, worktree, branch, session; `origin: { run, seq }`) (D3) | `shiioo-api`, `shiioo-runtime/src/spec.rs`, `workspace.rs` | T12 | The new run's harness remembers the earlier session; its spec names the original |
| T16 | Message box in the run view (Send at next turn / Now) and Follow up on finished runs | `ui/` | T4, T12–T15 | Both work from the desk and the phone |

### 8d. Taking over, on the office machine

| | Task | Where | Needs | Done when |
|---|---|---|---|---|
| T17 | ADR-0013: a WebSocket for the takeover terminal, amending ADR-0004 for one endpoint (D8) | `docs/adr/` | | Accepted by the owner |
| T18 | Events `TakeoverStarted { by }` and `TakeoverEnded { reason, transcript_blob? }` | `shiioo-core/src/event.rs`, `docs/design/event-log.md` | T11 | Payload round-trip and hash tests |
| T19 | Interactive driver mode: the harness command without `--print` / `exec`, same hook flags and environment, under a PTY, resumed from the run's session; holds the run's lease while attached | `shiioo-runtime/src/harness/` (new `interactive.rs`), `claude_code.rs`, `codex.rs` | T18 | A test with a fake harness binary gets keystrokes in and output out; the hook env is set |
| T20 | `POST /v1/runs/{run}/takeover` returning a single-use, short-lived attach ticket; `GET /v1/runs/{run}/terminal?ticket=…` WebSocket with binary PTY frames and a resize control frame (D9) | `shiioo-api` | T17, T19 | Used or expired ticket refused; detach and re-attach keeps the session |
| T21 | Transcript import at the end: read the harness session file (Claude Code JSONL, Codex rollout) into the log as messages and tool calls; raw stream as a capped, redacted blob (D4) | `shiioo-runtime/src/harness/`, `redact.rs` | T19 | `shiioo log <run>` reads through a takeover; secrets in the stream are redacted |
| T22 | Hand back (park, or resume headless with a note), detached-takeover timeout (D6), and restart recovery (`TakeoverEnded { reason: office_restarted }`, run left parked) | `shiioo-runtime/src/worker.rs`, `shiioo-api` | T20, T21 | Kill the office mid-takeover: after restart the run is parked and nothing drives it |
| T23 | Terminal view in the desk: vendored xterm.js with its licence, Take over / Hand back, the "rules active" banner, approvals in the side panel (D5) | `ui/`, `ui/vendor/` | T4, T20 | Live check with Claude Code: an `ask` command shows a card in the desk and a Telegram message; first answer wins |
| T24 | Live verification with both real harnesses and the security note: the shell escape bypasses the hook and is recorded | `docs/design/security.md`, `README.md` status table | T22, T23 | Status table row "built, verified live" |

### 8e. Takeover in containers and on nodes

| | Task | Where | Needs | Done when |
|---|---|---|---|---|
| T25 | Takeover inside a `container` run: `exec` with a TTY into the run's container, same mounts and egress | `shiioo-runtime/src/sandbox/` | T22 | Egress allow list still holds during a takeover |
| T26 | A streaming path between node and office for the terminal (D7), then takeover on remote nodes | `shiioo-node`, ADR-0012 follow-up | T22 | `/run on:spark …` can be taken over from the desk |
