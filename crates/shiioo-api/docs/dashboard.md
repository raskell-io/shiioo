# The dashboard (the desk)

A web UI with no build step: plain files under `ui/`, the scripts as native
ES modules, embedded into the binary and served by the API at `/`, `/ui`
and `/ui/{path}`. It speaks only to the HTTP API described in
[http.md](http.md), so anything it shows can also be fetched with `curl`.
The design follows `docs/design/desk.md` (requirements U1–U14) and its
mockups: a quiet desk on a wide screen (sidebar · tabs · up to three panes
of runs side by side, each with its own side panel), one column on a
phone, and only "needs you" in colour. Everything is reachable from the
keyboard, through single keys and the ⌘K palette.

## Files

| File | Lines | Role |
| --- | --- | --- |
| `ui/index.html` | ~30 | Shell: viewport and iOS web-app meta tags, the favicon, the sidebar `<nav id="side">` and `<main id="app">`, the toast area; resolves the theme (stored, or the system's) onto `<html data-theme>` before the first paint. |
| `ui/app.js` | ~40 | Entry module: hash routing onto the desk; starts the office feed, the sidebar and the keys once. |
| `ui/core.js` | ~165 | Token handling, `api()`, DOM helpers, times (`age`, `ago`, `span`), run states (`kind`, `mark`, `stateLabel`, `needsYou`), `who()`, the theme, toasts, the token form. |
| `ui/office.js` | ~65 | Every run's view, kept current by `GET /v1/events/stream`, plus `/v1/capacity` every minute; views `watch()` it instead of polling. |
| `ui/desk.js` | ~430 | The workspace: tabs, up to three panes with draggable dividers, the run-view cache, the layout in `localStorage["shiioo_desk"]`. |
| `ui/commands.js` | ~75 | Every keyboard action in one list (id, title, key, when it applies, what it does), read by the keys, the palette and the cheat sheet. |
| `ui/keys.js` | ~35 | Turns key presses into commands; ⌘K / Ctrl-K and Esc. |
| `ui/palette.js` | ~225 | The command palette with its fuzzy matcher, and the keyboard cheat sheet (`?`). |
| `ui/stream.js` | ~95 | The SSE client (`streamEvents`, `parseFrame`) and `liveStream`, which pauses with the browser tab or the pane. |
| `ui/describe.js` | ~40 | One line per event, mirroring the server's `describe()`; the run view's **Raw** mode and the fallback for unknown events. |
| `ui/views/side.js` | ~150 | The sidebar: Needs you, runs grouped by repository, unread marks, capacity meters, connection state, theme switch; the page title and favicon. |
| `ui/views/runs.js` | ~120 | Home (an overview on a desk, the run list on a phone) and the new-task sheet. |
| `ui/views/run.js` | ~500 | One run as an instance a pane mounts: header, actions, the log or the takeover terminal, the message box, the side panel (approval card and diff). |
| `ui/views/log.js` | ~250 | The structured log: turns, tool-call rows, inline approvals, how the run ended; a small markdown renderer. |
| `ui/views/diff.js` | ~90 | Parses the branch diff into files and renders the file list and hunks. |
| `ui/views/kit.js` | ~180 | `#kit`: every component in both themes, drawn by the real code, with a live contrast table. |
| `ui/app.css` | ~540 | Tokens (two colour sets, spacing, radius, elevation, motion, type) and every component. |
| `ui/views/terminal.js` | ~170 | The takeover terminal: loads xterm.js on first use, draws with the desk's font and theme colours, attaches over the ticketed WebSocket and rejoins after a drop. |
| `ui/vendor/xterm*.mjs`, `xterm.css` | | xterm.js 6.0 with its fit and WebGL add-ons, vendored unchanged from npm with their MIT licence in a header. |
| `ui/fonts/*.woff2` | | Latin subsets of Inter and Bricolage Grotesque, and JetBrains Mono with box drawing and the symbols terminal UIs use, from `assets/brand/fonts`; served from `UI_FONTS`. |

## Token

`shiioo install` prints `dashboard on this machine:
http://<addr>/#token=<api token>` (and the Tailscale variant); `shiioo serve`
prints the same URL with the path of `.shiioo/api-token` in place of the
token. Opening the link once is enough:

1. `app.js` reads `#token=` from the URL fragment, stores it under
   `localStorage["shiioo_token"]`, and strips the fragment with
   `history.replaceState` so the token is not left in the address bar or
   history.
2. Every request goes through `api()`, which adds `Authorization: Bearer`.
3. A `401` clears the stored token and shows a small form to paste a new
   one.

The fragment is never sent to the server, which is why the token can travel
in a link over Tailscale without appearing in access logs.

## Routes

Hash routing, so the single page works from the home screen:

- `#` (empty): home. On a desk, an overview ("1 needs you · 2 working")
  with **Next that needs you** and **New task**; on a phone (≤ 760 px), the
  run list.
- `#run=<id>`: that run, opened as a tab in the focused pane. The address
  follows the focused pane as you move around (without adding history).
- `#kit`: the component kit, in place of the panes; "Back to the desk"
  returns to the same layout.

Telegram notifications link straight to `#run=<id>` using the `[dashboard]
url` from `config.toml` (or the listen address).

## Tabs and panes

On a desk the main area is a tab bar over one to three panes:

- **Tabs.** Opening a run adds a tab after the current one. A tab shows the
  run's state mark and task; one that needs you carries the amber mark and
  stays bold, a finished one you have not opened carries a dot. Click to
  show it in the focused pane, ⇧-click to show it beside, middle-click or
  × to close; drag to reorder, or drop on a pane to show it there. Tabs
  for runs that no longer exist close themselves.
- **Panes.** The split button (or `\`) adds a pane to the right with the
  next tab not on screen; three at most, and none narrower than 360 px. A
  tab is in one pane at a time: opening one that is already on screen
  focuses its pane. The focused pane has a teal line on top and takes the
  keys. Dividers drag to resize and double-click to even out. A window
  made narrower hides the panes that don't fit (keeping the focused one)
  and shows them again when it is wide enough.
- **Views stay mounted.** Each open tab's run view (the newest eight beyond
  the ones on screen) keeps its log, scroll position, panel and draft
  message, so switching is instant. Only views in a pane stream; a covered
  one lets go of its connection and resumes from its last event when shown,
  which keeps the browser under its six connections per host.
- **Layout.** Tabs, panes, sizes and focus are kept in
  `localStorage["shiioo_desk"]`, so a reload comes back to the same desk.

On a phone there are no tabs: one pane, home is the run list, and a run's
"‹ Runs" goes back to it.

## Keyboard and palette

Every action is a command in `commands.js`; the keys, the palette and the
cheat sheet all read that list. Single keys act when no text box has the
cursor and no sheet is open:

| Key | Action |
| --- | --- |
| ⌘K / Ctrl-K | Command palette |
| ? | Cheat sheet |
| N | New task |
| J | Next run that needs you |
| H | Home in the focused pane |
| A | Go to the approval (opens the panel, focuses Allow; Enter answers) |
| T | Take over: sit at the agent's own terminal |
| R | Write to the run, reply, or follow up |
| D | Approval and diff panel |
| L | Raw log |
| ] / [ | Next / previous tab |
| 1–8, 9 | Tab by number; 9 is the last |
| W, ⇧T | Close the tab, reopen the last closed one |
| \\, ⇧\\, O | Split, close the pane, focus the next pane |
| Esc | Leave the text box; close the panel |

The **palette** fuzzy-matches runs (by task, repository, branch and id),
repositories ("New task in sango") and commands, and shows each command's
key. Empty, it starts with what needs you, what you can do to the run in
the focused pane (Allow and Deny name the request), the open tabs, then
recent runs and the rest. ↑/↓ move, Enter opens or runs, ⇧Enter opens a
run beside the focused pane. Cancel asks for a second Enter. A query that
matches nothing offers to start it as a new task. The matcher favours
substrings, word starts and runs of consecutive letters, and refuses
letters strewn across a long text.

## Sidebar

Shown on a desk, kept across routes, fed by `office.js`:

- **Needs you**: runs waiting for an approval, needing attention, or parked
  for your reply, with what they wait for (the approval's summary) and where
  (`repo · Claude Code`). The page title counts them, and the favicon
  gets an amber dot.
- **Runs**, grouped by repository (runs without one under "Office"), the
  most recently active group first, six per group with "Show N more". A
  finished run you have not opened since it changed carries a dot; what
  you have seen is kept in `localStorage["shiioo_seen"]`.
- Footer: capacity meters per provider window (the bar turns amber at
  80 %), whether the office stream is live, and System / Light / Dark
  (`localStorage["shiioo_theme"]`).

State marks are shapes first: a ring for working (with a slow ping), a
filled amber dot for needs you, a check for done, a cross for failed, a
square for cancelled.

## Run view

- **Header**: the task, a state pill, then repository · branch · who works
  on it · machine · isolation (when not a plain worktree) · how long ·
  short id · "follows up …" for a follow-up.
- **Actions**: **Cancel run** needs two clicks (the first arms it for four
  seconds); **Resume** when open; **Raw** switches the log to one line per
  event; **Approval & diff** (D) opens the side panel.
- **Log** (`log.js`): your task and messages as "You" turns (a message
  reads "Not read yet" until the agent's next turn or, for a harness, its
  next process); the agent's text as markdown; each tool call as one row
  with the tool, its main argument, ok / error / stopped (its process
  ended first) and duration, the input and
  the output (folded to its last 12 lines) inside; rules that did not just
  allow; approvals inline with Allow / Deny while they are the one the run
  waits on; how the run ended (Done · time · cost, Failed, Cancelled).
  Leases, capacity readings and the sandbox marker stay out. The log
  follows the tail while you are at the bottom; scrolled up, a "Jump to
  latest" pill appears.
- **Side panel**: the approval card (Allow / Deny) and the branch diff:
  file list with +/− counts, then each file's hunks with line numbers;
  long lines scroll or wrap. It sits beside the log when the run's pane is
  at least 1000 px wide and slides over the run inside its pane otherwise
  (a container query, so it follows the pane, not the window). It opens
  by itself for an approval when there is room beside the log, unless you
  closed it.
- **Message box** pinned at the bottom while the run can still read a
  message (open, waiting approval, needs attention, parked): posts to
  `/v1/runs/{id}/messages` with an `Idempotency-Key` made per message and
  kept until it is accepted, so resending after a dropped connection can't
  deliver it twice. ⌘/Ctrl-Enter sends. The hint says when it will be read.
  While a Claude Code or Codex run is working, **Now** (⌘/Ctrl-Shift-Enter)
  sends with `"now": true`: the current step is stopped and the session
  resumed with the message; the log shows "You stopped the current step
  so this is read now".
  On a finished Claude Code or Codex run the same box is **Follow up**: it
  posts to `/v1/runs/{id}/follow-up` and opens the new run.

## Taking over

**Take over** (T) on a Claude Code or Codex run that is still open, on
this machine and in a plain worktree, puts you at the agent's own terminal
in place of the log ([http.md](http.md), `POST …/takeover`). A working
agent first stops its current step; the terminal then shows the harness's
own UI resumed from the run's session, so its history is on screen.

- A teal bar above it says you're at the keyboard and that the rules still
  judge every command. An `ask` command waits for you exactly as in a
  headless run: the card appears in the side panel (and on the phone).
- **Hand back** ends it and leaves the run waiting for your next message;
  **Hand back, carry on** takes an optional note and lets it continue
  headless in the same session. The log then shows what happened at the
  keyboard, imported from the harness's session file, between "You took
  over" and "You handed back".
- Keys typed in the terminal are the agent's: Esc interrupts it, single-key
  shortcuts are off; ⌘K still opens the palette, which has Hand back.
- Closing the tab or the pane detaches; the agent keeps its terminal for
  30 minutes. Another window or the phone shows "Taken over" with **Join**,
  which attaches to the same terminal (each attach redeems a fresh ticket,
  and a dropped connection rejoins by itself).
- xterm.js draws with WebGL; `localStorage.shiioo_term_renderer = "dom"`
  switches to its DOM renderer on a GPU that draws it badly. The colours
  follow the desk's theme.
- Claude Code asks once whether to trust a new worktree; its preselected
  answer is "No, exit", so choose "Yes".

## Live log

The log is a hand-rolled SSE client built on `fetch` rather than
`EventSource`, because `EventSource` cannot send an `Authorization` header:

- reads the response body as a stream, splits it into frames on blank lines,
  normalises `\r\n`, ignores `:` comment lines, and parses `id:`, `event:`
  and `data:`;
- remembers the last `id` and reconnects with `Last-Event-ID`, backing off
  from 1 s to 10 s;
- for a run, stops reconnecting after a terminal payload (`run_completed`,
  `run_failed`, `run_cancelled`); the office stream never stops, and tells
  the sidebar when it drops ("Reconnecting…");
- the run's header follows the office stream; a run event also schedules
  one refetch of the run, in case that stream is down.

## Styling

Two colour sets: light on `:root` and `.t-light`, dark on
`:root[data-theme="dark"]` and `.t-dark`. `index.html` resolves the stored
theme, or the system's, onto `data-theme` before the first paint, and
`core.js` follows the system while the choice is "System". The `.t-*`
classes scope a set to a subtree (the kit shows both at once). Every text
colour meets WCAG AA (4.5:1) on the surfaces it sits on, in both sets:
`tests/dashboard.rs` reads the tokens from `app.css` and checks the same
pairs the kit shows. One attention colour is used only for "needs you":
`--attn` for marks and borders, `--attn-ink` for text. The brand teal
(`--accent`) is for selection, primary actions and working. Spacing is a
4 px scale (`--s1`…`--s10`), radii `--r1`…`--r3`, elevation `--e1`/`--e2`,
motion `--t-fast`/`--t-med` (zero under `prefers-reduced-motion`). Inter
for the interface, Bricolage Grotesque for the brand, JetBrains Mono for
logs, diffs and the terminal (a subset with box drawing and the symbols
terminal UIs use).
Below 760 px the sidebar goes and the layout is one column with phone-sized
targets.

## Changing it

Edit the files under `ui/` and rebuild; they are included with
`include_str!`, so the binary always ships the version it was built with.
A new file goes into `UI_FILES` (or `UI_FONTS` for a font) in `src/lib.rs`: `tests/dashboard.rs`
fails when that list and the directory disagree, or when a module imports
a file that is not served. It also checks the files are served without a
token and that the diff endpoint behaves; there is no browser test.

See also: [telegram design](../../../docs/design/telegram.md) for the other
phone surface, [deployment design](../../../docs/design/deployment.md) for
reaching the dashboard over Tailscale.
