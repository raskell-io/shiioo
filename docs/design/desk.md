# The desk: the web UI, and working with an employee directly

Status: **partly built** (phase 8). Built: parked runs and messages (T11–T13), **Now** (T14), follow-ups (T15), the message box (T16), and the desk's layout and keyboard: tabs and split panes (T4), the command palette and shortcuts (T5), tokens, fonts and the `#kit` page with AA contrast checked (T28), the sidebar (T3), the structured log (T29) and the diff panel (T30), each marked in section 9. ADR-0013 (the takeover terminal, T17) is proposed. Decisions marked **D1**…**D16** are open unless marked settled; the rest follows from the picture.

Related: [api.md](api.md) (the endpoints the desk uses), [`shiioo-api/docs/dashboard.md`](../../crates/shiioo-api/docs/dashboard.md) (the dashboard as built), [conversation.md](conversation.md) (phase 7, which shares the parked-run work), [execution.md §1](execution.md#1-run-states-partly-built) (parked runs), [governance.md](governance.md) (rules and approvals), [runtimes-and-providers.md](runtimes-and-providers.md) (the harness drivers), [ADR-0004](../adr/0004-rest-and-sse-api.md) (REST and SSE).

## 1. The picture

The phone is how the owner runs the company while away. The desk is how they run it while sitting at a big screen: every repository and machine in a sidebar, what needs them floated to the top, several runs open side by side, and the diff next to the request that asks to push it. Most of the time the owner still delegates. Sometimes the boss walks over to an employee's desk and says "no, do it like this", or sits down at the keyboard for ten minutes. Both are part of the job, and both stay on the record.

Long term the desk is also a **Rust-native client app** (in the spirit of Herdr: a sidebar of spaces and tabs, terminals for agents, a core that keeps working when the app closes), drawn by a Rust UI toolkit rather than a web view. Shiioo already has that core: the office keeps the log, the leases and the workers, and survives restarts. So the web desk comes first (the phone needs it anyway), and both are built so that the native app is one more client of the same API, sharing Rust crates with the office instead of re-implementing it (section 10).

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
- The groundwork the Rust-native client will stand on: typed wire types, a client crate, one source for design tokens (section 10).

**Out (for this phase):**

- The native app itself (a phase 9 candidate, section 10). What this phase owes it is an API with nothing the web desk alone can do (R3), and the shared crates.
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
- **R7. Live without polling where it matters.** Run logs stream as today. **D1:** how the sidebar learns of new runs and state changes. Proposed: an office-wide SSE stream (`GET /v1/events/stream`) of run lifecycle and approval events, resumable like the per-run stream, replacing the 10 s refresh.

### Instructing an employee

- **R8. Message a run.** The owner types an instruction into a run ("use the existing retry helper, don't add a dependency"). It is appended as `UserMessage { author: owner }` through `POST /v1/runs/{run}/messages` (planned in [api.md](api.md)) and shown in the log at once. **D2:** when a harness run receives it. Proposed: at the next turn boundary by default: the worker lets the current harness turn finish, then resumes the same session with the message as the prompt; a **Now** option interrupts the turn (stops the harness process and resumes with the message, the interrupted-turn resume the drivers already use after a crash). Native-loop runs read it at their next model call.
- **R9. Follow up on a finished run.** A completed or failed run is closed for good (the log does not reopen). "Follow up" starts a continuation run: same repository, same worktree and branch, the harness resumed from the finished run's session, the owner's message as the prompt, and `origin: { run, seq }` linking back. **D3:** confirm that continuation runs are the shape, rather than reopening.

### Taking over

- **R10. Take the desk.** From a run, **Take over** opens a terminal in the desk running the harness interactively in the run's worktree, resumed from the run's session (`claude --resume <session>`, `codex resume <thread>`), started by Shiioo with the same hook configuration the headless driver uses, so every tool call the agent makes during the takeover still goes through the rules. The owner types to the agent as they would in their own terminal. A running run stops at its turn boundary first (as in R8); the takeover then holds the run's lease, so no worker drives it while the owner is at the keyboard.
- **R11. On the record.** The log shows `TakeoverStarted { by: owner }` and `TakeoverEnded`, every hook decision in between as for any harness run, and, at the end, the takeover's transcript imported from the harness's session file as messages and tool calls, so `shiioo log` reads through the takeover like any other stretch of the run. **D4:** whether the raw terminal stream is also kept, as a blob referenced from `TakeoverEnded`. Proposed: yes, capped in size, redacted like every other output, for the cases the transcript import misses. Commands the owner runs directly through the harness's own shell escape (Claude Code's `!`) bypass the hook, since they are the owner's own commands; the transcript records them and the docs say so plainly.
- **R12. Approvals during a takeover.** An `ask` verdict while the owner is at the keyboard appears in the desk next to the terminal and on the phone as always; the first answer wins. **D5:** whether `ask` should turn into `allow` during a takeover, since the owner is present. Proposed: no; the rules stay what they say, and a one-tap answer in the desk is cheap.
- **R13. Hand back.** **Hand back** ends the interactive process, records `TakeoverEnded`, and either leaves the run parked for the next instruction or resumes it headless with an optional note ("carry on from here"). Closing the browser tab does not end the takeover; the session waits (D6) and can be re-attached from any client.
- **R14. Isolation and machines.** In a `container` run the terminal attaches inside the run's container (`exec` with a TTY), with the same mounts and egress as the headless run. On a remote node the terminal runs on the node. **D7:** remote-node takeover needs a streaming path the long-poll protocol ([ADR-0012](../adr/0012-nodes-are-workers-over-the-offices-api.md)) does not have. Proposed: takeover on the office machine first; nodes after the transport (D8) exists.

### The bar for polish

The desk is held to the standard of a good native tool (Herdr is the reference the owner named), not of an admin page. "Works" is not done; these are requirements with the same weight as the rest.

- **U1. Quiet until it needs you.** Herdr's best idea: agents report working, done or waiting, the one that needs you shows in the sidebar and on its tab, and everything else stays quiet so the owner can stop checking. The desk follows it: one attention colour, used only for "needs you" (approval, plan, needs attention); working runs get a calm activity mark, finished runs fade; no badges, counters or toasts for routine progress.
- **U2. A visual system, not ad-hoc CSS.** Design tokens for colour, spacing (4 px scale), radius, elevation and motion; a type scale with Inter for the interface and Bricolage Grotesque for the brand only (both already in `assets/brand/fonts`, served by the binary, no third-party font hosts); one monospace face for logs, diffs and the terminal. Light, dark and follow-the-system, each designed, not inverted. The brand teal stays the accent.
- **U3. The log reads like a story, not a dump.** Events are grouped into turns; a tool call and its result are one collapsible row with the command, its exit status and duration; long output folds to its last lines; edits show as small inline diffs; model text is rendered markdown; timestamps are relative with the exact time on hover. The log follows the tail while the owner is at the bottom and stops following when they scroll up, with a "jump to latest" pill.
- **U4. Diffs worth reviewing.** Per-file sections with a file list, unified or split view, syntax highlighting for the common languages, word-level changes, collapsed unchanged context.
- **U5. Fast, and feels fast.** Tab switches are instant (panes stay mounted); Allow / Deny, cancel and send update the screen at once and reconcile with the server, rolling back with a clear message if the server refuses; a 10 000-event log scrolls at 60 fps (virtualised); no layout shift while events stream in; first paint under 1 s.
- **U6. Motion with restraint.** Transitions of 120–180 ms for panes, the palette and cards appearing; none on streaming text; everything off under `prefers-reduced-motion`.
- **U7. The keyboard is complete and discoverable.** Every action has a shortcut; the palette lists actions with their keys and fuzzy-matches runs, repos and pages; `?` shows the cheat sheet; focus is always visible and never lost after an action.
- **U8. Every state is designed.** Empty states that teach the next step (no runs yet: how to start one, with the `/run` and `POST /v1/runs` equivalents); skeletons while loading; errors that say what happened and what to do; a reconnecting indicator when a stream drops, which clears itself when it catches up.
- **U9. It tells you, even in another window.** Optional browser notifications and a soft sound for "needs you" only; the tab title and favicon carry the count of things waiting; clicking a notification opens the right run with the card focused.
- **U10. Installable.** The desk is a PWA (manifest, icons from `assets/brand`, standalone window), so on a Mac it opens in its own window from the Dock, which is most of the native feel before the native app exists.
- **U11. A terminal that feels like a terminal.** xterm.js with its WebGL renderer, the desk's monospace font and theme colours, correct resize, copy and paste, clickable links, scrollback; the "at the keyboard" banner is part of the frame, not a pop-up.
- **U12. Personal, safely.** Herdr makes every pixel a file the agent may change; Shiioo keeps the chrome built in (the approval card must not be something an agent can restyle), but the owner's **themes** and **key bindings** are files under `.shiioo/desk/` (`theme.toml`, `keys.toml`), read by the desk through the API. The default rules deny agent writes under `.shiioo/`.
- **U13. Accessible.** WCAG AA contrast in every theme, labels for screen readers, full operation without a pointer, hit targets of at least 24 px on the desk and 44 px on the phone.
- **U14. Reviewed like code.** Each sub-phase ends with a UX review: screenshots of every view in light and dark at phone and desk widths, a short papercut list, and the papercuts fixed or consciously kept before the phase is called built.

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

0. **Design first:** mockups the owner signs off, then the token and component kit. The polish bar (U1–U14) applies to every phase below and each one ends with a UX review.
1. **8a. The desk layout** over the existing API: sidebar, tabs, split panes, the diff side panel, the command palette. The office-wide stream (R7).
2. **8b. The office pages:** plans, routines, capacity, deliverables, the report, machines.
3. **8c. Instructing:** parked runs and `UserMessage` (shared with 7a), `POST /v1/runs/{run}/messages`, turn-boundary and **Now** delivery, follow-up continuation runs.
4. **8d. Takeover on the office machine:** PTY driver mode for Claude Code and Codex, the terminal WebSocket and its ADR, attach tickets, transcript import, hand back, restart recovery.
5. **8e. Takeover in containers and on nodes,** once the node transport can stream.
6. **Alongside 8a–8d:** the groundwork for the native client (T37–T41): wire types, `shiioo-client`, the CLI and node moved onto it, tokens from one source.
7. **Phase 9 candidate:** the Rust-native client app (section 10).

## 7. Acceptance

- On a phone the dashboard looks and behaves as today; on a wide screen the sidebar shows every repository's runs, and a new approval appears under **Needs you** within a second without a reload.
- Everything the desk does is reproducible with `curl` against `/v1` (checked by a test that drives the API the way the desk does).
- A message sent to a running Claude Code run is delivered when the current turn ends, appears in the log as the owner's, and the agent's next turn responds to it; **Now** interrupts and the resumed session still has the earlier context.
- Follow-up on a completed run starts a new run on the same branch whose harness remembers the earlier session, and whose spec names the original run.
- During a takeover, a tool call that `rules.toml` says `ask` produces an approval card in the desk and a Telegram message; a `deny` is denied in the terminal. After hand back, `shiioo log <run>` verifies and shows the takeover's messages and tool calls between `TakeoverStarted` and `TakeoverEnded`.
- Killing the office during a takeover leaves the run parked with `TakeoverEnded { reason: office_restarted }` after restart, and no headless worker picks it up on its own.
- A terminal WebSocket URL with a used or expired ticket is refused.
- **Polish gates (U1–U14):** with ten runs open and one waiting, the only attention colour on screen is on the waiting one; a 10 000-event log scrolls without dropped frames on the Mac Studio; every view has a designed empty, loading and error state; every action in the palette works from the keyboard; light and dark pass WCAG AA; the desk installs as a standalone window; each sub-phase's UX review is in the repository with its screenshots and papercut list.

## 8. Decisions needed

| | Question | Proposed |
|---|---|---|
| D1 | How the sidebar stays live | *Settled, built:* office-wide resumable SSE, `GET /v1/events/stream`, one `RunView` snapshot per change ([api.md §3.2](api.md#32-office-stream-built-for-runs-machines-planned)) |
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
| D12 | The monospace face for logs, diffs and the terminal | Bundle one OFL face (JetBrains Mono or Iosevka) next to Inter, so every machine renders the same |
| D13 | Syntax highlighting without a build step | A small vendored highlighter (Prism core plus a handful of languages) or a server-side highlighter in the diff endpoint |
| D14 | The native toolkit | Decide after a spike (T42). Lead candidate GPUI (GPU-drawn, Zed's toolkit, Zed already embeds `alacritty_terminal` in it); Iced as the more conventional alternative; a web view shell (Tauri) is the fallback, not the goal |
| D15 | How web and native share the sidebar logic | The Rust office model in `shiioo-client` is the reference; the JS desk is tested against the same event fixtures and expected states (no WASM, so no build step) |
| D16 | Shared design tokens | `ui/tokens.toml` as the source; `tokens.css` generated and checked in, with a test that fails when they drift; the native app reads the TOML |

## 9. Tasks

One line each, in build order within a phase. "Needs" names what must land first; a task with no "needs" can start now. Each task ends with tests in the crate it touches and the docs it changes (the design doc's *(planned)* marks become *(built)*, the crate's `docs/` describe the code).

### Design first (before 8a)

| | Task | Where | Needs | Done when |
|---|---|---|---|---|
| T27 | Interactive mockups of the desk and phone layouts, light and dark: sidebar, run view with log, diff and approval card, palette, empty states, takeover frame; reviewed by the owner before T3 | an artifact or `docs/design/desk-mockups/` | | Owner signs off; open questions folded into this doc |
| T28 *(built: tokens, components, fonts, light and dark, `#kit` with a live contrast table, AA checked by `tests/dashboard.rs`; tokens are still in `app.css`, not split into files)* | Design tokens and base components in CSS: colour (three themes), type scale, spacing, radius, elevation, motion; buttons, cards, rows, pills, the attention mark; bundled fonts (D12) | `ui/tokens.css`, `ui/components.css`, `assets/brand/fonts` | T1, T27 | A reference page `/ui/#kit` renders every component in every theme; AA contrast checked |

### 8a. The desk layout

| | Task | Where | Needs | Done when |
|---|---|---|---|---|
| T1 *(built)* | Split `app.js` into native ES modules (core: token, `api()`, routing, SSE client; one module per view) and serve each embedded file under `/ui/` | `shiioo-api/ui/`, `lib.rs` UI routes, `tests/dashboard.rs` | | Phone dashboard unchanged; every module served without a token; D10 settled |
| T2 *(built; machine frames planned)* | Office-wide event stream `GET /v1/events/stream`: a `RunView` per run created or changed (approvals and plans included, as run states), a `ready` frame after the first pass; resumable with `Last-Event-ID` | `shiioo-api` (store notifications), `docs/http.md`, `docs/design/api.md` | | `tests/events.rs`: snapshot, a live run arriving without polling, resume from the last id skipping unchanged runs; D1 settled |
| T3 *(part built: sidebar with Needs you, runs by repository, unread marks; no machines group)* | Desk layout: breakpoint, sidebar (Needs you, repos, machines), unread marks kept in `localStorage`, attention rules of U1 | `ui/` | T1, T2, T28 | Wide screen shows grouped runs; a new approval appears under Needs you without reload |
| T4 *(built: tabs with drag to reorder, up to three resizable panes, views kept mounted and paused when covered, the side panel per pane by container query, the layout kept across reloads)* | Tabs and split panes, one live log per pane; the diff and approval card in a side panel | `ui/` | T3 | Two runs stream side by side; Allow in the side panel answers the hook |
| T5 *(built: one command list behind the keys, the ⌘K palette with fuzzy matching and the `?` cheat sheet)* | Command palette and shortcuts (new task, jump to run, next "needs me", allow / deny focused, cancel) | `ui/` | T4 | Every action reachable without the mouse |
| T6 | API-parity test: drive the flows the desk uses (create, stream, approve, cancel, diff) through `/v1` only | `shiioo-api/tests/` | T2 | Test passes; R3 holds by construction |

### Polish (alongside 8a–8d)

| | Task | Where | Needs | Done when |
|---|---|---|---|---|
| T29 *(part built: turns, tool-call rows, folded output, markdown, tail-follow; no virtualised list or inline edit diffs)* | The structured log: turns, collapsible tool-call rows with status and duration, folded output, inline edit diffs, rendered markdown, relative times, tail-follow with "jump to latest", virtualised list (U3, U5) | `ui/log.js` | T4, T28 | 10 000 events scroll at 60 fps; a test fixture log renders without layout shift while streaming |
| T30 *(part built: file list and unified hunks in the side panel)* | The diff viewer: file list, unified and split, word-level changes, collapsed context, highlighting (U4, D13) | `ui/diff.js`, possibly `shiioo-api` diff endpoint | T4, T28 | A real multi-file PR diff reads comfortably at desk width and on the phone |
| T31 | Optimistic actions with reconciliation, reconnecting indicator, designed empty / loading / error states for every view (U5, U8) | `ui/` | T3 | Each view checked with the server stopped, slow and empty |
| T32 *(part built: tab title count and favicon dot)* | Notifications, sound, tab title and favicon count for "needs you"; click opens the run with the card focused (U9) | `ui/` | T2, T3 | A pending approval in a background window notifies once, and the click lands on the card |
| T33 | PWA: manifest, icons, standalone window, service worker for the shell only (never caching API responses) (U10) | `ui/`, `lib.rs` routes, `assets/brand/build.py` | T3 | Installs from Safari and Chrome on macOS; opens in its own window |
| T34 | Owner themes and key bindings from `.shiioo/desk/theme.toml` and `keys.toml` via `GET /v1/desk/settings`; default rules deny agent writes under `.shiioo/` (U12) | `shiioo-api`, `shiioo-rules` defaults, `ui/` | T5, T28 | Changing `theme.toml` restyles the desk on reload; an agent's write there is denied |
| T35 *(part built: AA contrast in both themes, checked by a test)* | Accessibility pass: labels, focus order, keyboard-only walkthrough, contrast in all themes (U13) | `ui/` | T5, T28 | Keyboard-only walkthrough of every flow passes; automated contrast check in `tests/dashboard.rs` or a script |
| T36 | UX review at the end of each sub-phase: screenshots of every view (light, dark, phone, desk) driven by a browser script, papercut list, fixes (U14) | `docs/design/desk-reviews/`, `scripts/` | each sub-phase | Review committed; papercuts fixed or listed in the backlog |

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
| T11 *(built)* | Kernel: `RunParked { waiting_for }` and `RunWoken` payloads, parking ends the lease, the status fold learns `Parked`; the native loop shows messages to the model, parks when `park_on_answer`, answers a message that arrived mid-turn before settling (`context_seq`), and wakes on input | `shiioo-core/src/event.rs`, `shiioo-runtime/src/status.rs` | | Fold and sim tests cover park → message → wake; shared with phase 7a (D11) |
| T12 *(built)* | `POST /v1/runs/{run}/messages` appending `UserMessage { author: "owner" }` with `Idempotency-Key`; a parked run is driven again | `shiioo-api/src/lib.rs`, `docs/http.md` | T11 | A message to a parked run wakes it once, also when retried |
| T13 *(built; verified live with Claude Code 2.1.316 and Codex 0.163.0)* | Harness delivery at the turn boundary: when the harness process exits with unread messages, resume the session with them as the prompt; otherwise park instead of completing (D2) | `shiioo-runtime/src/harness/mod.rs`, `worker.rs` | T11 | Scripted-harness test: a message sent mid-turn is the next resume prompt |
| T14 *(built; verified live with Claude Code 2.1 and Codex 0.163: interrupted mid-command, the resumed session still knew what it said before)* | **Now** delivery: stop the harness process, resume with the interrupted-turn flag and the message | `harness/mod.rs` | T13 | Real-binary check with Claude Code and Codex: context survives the interrupt |
| T15 *(built; verified live with both harnesses, including a follow-up of a follow-up)* | Follow-up on a finished run: `POST /v1/runs/{run}/follow-up` creates a continuation run (same repo, worktree, branch, session; `origin: { run, seq }`) (D3) | `shiioo-api`, `shiioo-runtime/src/spec.rs`, `workspace.rs` | T12 | The new run's harness remembers the earlier session; its spec names the original |
| T16 *(built: Send at next turn, Now, Follow up)* | Message box in the run view (Send at next turn / Now) and Follow up on finished runs | `ui/` | T4, T12–T15 | Both work from the desk and the phone |

### 8d. Taking over, on the office machine

| | Task | Where | Needs | Done when |
|---|---|---|---|---|
| T17 *(proposed: [ADR-0013](../adr/0013-a-websocket-for-the-takeover-terminal.md), waiting for the owner)* | ADR-0013: a WebSocket for the takeover terminal, amending ADR-0004 for one endpoint (D8) | `docs/adr/` | | Accepted by the owner |
| T18 | Events `TakeoverStarted { by }` and `TakeoverEnded { reason, transcript_blob? }` | `shiioo-core/src/event.rs`, `docs/design/event-log.md` | T11 | Payload round-trip and hash tests |
| T19 | Interactive driver mode: the harness command without `--print` / `exec`, same hook flags and environment, under a PTY, resumed from the run's session; holds the run's lease while attached | `shiioo-runtime/src/harness/` (new `interactive.rs`), `claude_code.rs`, `codex.rs` | T18 | A test with a fake harness binary gets keystrokes in and output out; the hook env is set |
| T20 | `POST /v1/runs/{run}/takeover` returning a single-use, short-lived attach ticket; `GET /v1/runs/{run}/terminal?ticket=…` WebSocket with binary PTY frames and a resize control frame (D9) | `shiioo-api` | T17, T19 | Used or expired ticket refused; detach and re-attach keeps the session |
| T21 | Transcript import at the end: read the harness session file (Claude Code JSONL, Codex rollout) into the log as messages and tool calls; raw stream as a capped, redacted blob (D4) | `shiioo-runtime/src/harness/`, `redact.rs` | T19 | `shiioo log <run>` reads through a takeover; secrets in the stream are redacted |
| T22 | Hand back (park, or resume headless with a note), detached-takeover timeout (D6), and restart recovery (`TakeoverEnded { reason: office_restarted }`, run left parked) | `shiioo-runtime/src/worker.rs`, `shiioo-api` | T20, T21 | Kill the office mid-takeover: after restart the run is parked and nothing drives it |
| T23 | Terminal view in the desk (U11): vendored xterm.js with its WebGL renderer and licence, the desk font and theme, Take over / Hand back, the "rules active" banner, approvals in the side panel (D5) | `ui/`, `ui/vendor/` | T4, T20 | Live check with Claude Code: an `ask` command shows a card in the desk and a Telegram message; first answer wins |
| T24 | Live verification with both real harnesses and the security note: the shell escape bypasses the hook and is recorded | `docs/design/security.md`, `README.md` status table | T22, T23 | Status table row "built, verified live" |

### 8e. Takeover in containers and on nodes

| | Task | Where | Needs | Done when |
|---|---|---|---|---|
| T25 | Takeover inside a `container` run: `exec` with a TTY into the run's container, same mounts and egress | `shiioo-runtime/src/sandbox/` | T22 | Egress allow list still holds during a takeover |
| T26 | A streaming path between node and office for the terminal (D7), then takeover on remote nodes | `shiioo-node`, ADR-0012 follow-up | T22 | `/run on:spark …` can be taken over from the desk |

### Groundwork for the Rust-native client (alongside 8a–8d)

| | Task | Where | Needs | Done when |
|---|---|---|---|---|
| T37 | Wire types: request and response structs for every `/v1` endpoint and SSE payload, used by the handlers instead of ad-hoc `json!`; the OpenAPI document generated from them (api.md section 7, today deferred) | new `crates/shiioo-wire`, `shiioo-api` | | Handlers compile against the types; `/v1/openapi.json` served; a test fails on an unreviewed change |
| T38 | `shiioo-client`: a typed async client over the wire types: REST with `Idempotency-Key`, the per-run and office-wide SSE streams with `Last-Event-ID` resume and backoff, the terminal WebSocket, and the office model (the fold from events to sidebar state, D15) | new `crates/shiioo-client` | T37, T2 | Tests against an in-process office: resume after a dropped stream sees every event once; the model matches the fixture states |
| T39 | Move the CLI and the node onto `shiioo-client`, removing their hand-rolled `reqwest` calls | `crates/shiioo/src/main.rs`, `crates/shiioo-node` | T38 | No direct `reqwest` use outside `shiioo-client` and the providers; existing tests pass |
| T40 | Design tokens from one source (D16) and owner theme / key files with a schema owned by a Rust type | `ui/tokens.toml`, `shiioo-wire`, `tests/dashboard.rs` | T28, T34 | Editing a token without regenerating fails a test; the schema rejects an unknown key |
| T41 | Terminal frame protocol written down in ADR-0013 with a Rust reference client in `shiioo-client` and a test that attaches, types, resizes and detaches | `docs/adr/`, `shiioo-client` | T17, T20, T38 | The web desk and the Rust test speak the same frames |

## 10. The Rust-native client *(phase 9 candidate, not designed in detail)*

What the owner asked for: long term, a desktop app written in Rust and drawn natively, at least as polished as Herdr. This section records what it is and what the web desk must not get in the way of; it becomes its own design document when phase 9 starts.

- **N1. A client, never a second office.** The app holds no state of its own beyond window layout and preferences. Everything comes from `/v1` through `shiioo-client`; closing it stops nothing.
- **N2. Same crates as the office.** Wire types (`shiioo-wire`), the client and office model (`shiioo-client`), the theme and key file schemas. A change to the API breaks the app at compile time, not at run time.
- **N3. Real terminals, natively.** Takeover sessions rendered with a Rust terminal emulator core (`alacritty_terminal`, as Zed does) fed by the same WebSocket frames as the web desk (T41).
- **N4. The same polish bar.** U1–U14 apply unchanged, with native equivalents: system notifications, a menu-bar item with the "needs you" count, global shortcut to the palette, native window tabs and splits.
- **N5. Several offices.** The app can hold more than one office connection (home studio, a second machine), each with its own token in the OS keychain.
- **N6. Same personalisation.** It reads the owner's `.shiioo/desk/theme.toml` and `keys.toml` through the API, so the web and native desks look and behave alike.

| | Task | Where | Needs | Done when |
|---|---|---|---|---|
| T42 | Toolkit spike (D14): a sidebar fed by the office-wide stream, one live log, and one terminal over the takeover WebSocket, in GPUI and in Iced; measure frame time, memory, text quality and terminal fidelity | `spikes/native-*` (not in the workspace) | T38, T41 | A written comparison and a recommendation in an ADR |
