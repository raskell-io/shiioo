# The dashboard (the desk)

A web UI with no build step: plain files under `ui/`, the scripts as native
ES modules, embedded into the binary and served by the API at `/`, `/ui`
and `/ui/{path}`. It speaks only to the HTTP API described in
[http.md](http.md), so anything it shows can also be fetched with `curl`.
The design follows `docs/design/desk.md` (requirements U1–U14) and its
mockups: a quiet three-column desk (sidebar · run · side panel) on a wide
screen, one column on a phone, and only "needs you" in colour.

## Files

| File | Lines | Role |
| --- | --- | --- |
| `ui/index.html` | ~25 | Shell: viewport and iOS web-app meta tags, the sidebar `<nav id="side">` and `<main id="app">`, the toast area; applies a stored theme before the first paint. |
| `ui/app.js` | ~35 | Entry module: hash routing; starts the office feed, the sidebar and the keys once. |
| `ui/core.js` | ~165 | Token handling, `api()`, DOM helpers, times (`age`, `ago`, `span`), run states (`kind`, `mark`, `stateLabel`, `needsYou`), `who()`, the theme, toasts, the token form, the current view's teardown (`onLeave` / `leave`). |
| `ui/office.js` | ~60 | Every run's view, kept current by `GET /v1/events/stream`, plus `/v1/capacity` every minute; views `watch()` it instead of polling. |
| `ui/keys.js` | ~30 | Desk-wide keys: N new task, J next run that needs you, R the message box, D the side panel. |
| `ui/stream.js` | ~70 | The SSE client (`streamEvents`, `parseFrame`). |
| `ui/describe.js` | ~40 | One line per event, mirroring the server's `describe()`; the run view's **Raw** mode and the fallback for unknown events. |
| `ui/views/side.js` | ~135 | The sidebar: Needs you, runs grouped by repository, unread marks, capacity meters, connection state, theme switch. |
| `ui/views/runs.js` | ~115 | Home (an overview on a desk, the run list on a phone) and the new-task sheet. |
| `ui/views/run.js` | ~300 | One run: header, actions, the log, the message box, the side panel (approval card and diff). |
| `ui/views/log.js` | ~235 | The structured log: turns, tool-call rows, inline approvals, how the run ended; a small markdown renderer. |
| `ui/views/diff.js` | ~90 | Parses the branch diff into files and renders the file list and hunks. |
| `ui/app.css` | ~400 | Tokens (colour for light and dark, spacing, radius, elevation, motion, type) and every component. |
| `ui/fonts/*.woff2` | | Latin subsets of Inter and Bricolage Grotesque from `assets/brand/fonts`, served from `UI_FONTS`. |

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
- `#run=<id>`: one run.

Telegram notifications link straight to `#run=<id>` using the `[dashboard]
url` from `config.toml` (or the listen address).

## Sidebar

Shown on a desk, kept across routes, fed by `office.js`:

- **Needs you**: runs waiting for an approval, needing attention, or parked
  for your reply, with what they wait for (the approval's summary) and where
  (`repo · Claude Code`). The page title counts them.
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
  with the tool, its main argument, ok / error and duration, the input and
  the output (folded to its last 12 lines) inside; rules that did not just
  allow; approvals inline with Allow / Deny while they are the one the run
  waits on; how the run ended (Done · time · cost, Failed, Cancelled).
  Leases, capacity readings and the sandbox marker stay out. The log
  follows the tail while you are at the bottom; scrolled up, a "Jump to
  latest" pill appears.
- **Side panel**: the approval card (Allow / Deny) and the branch diff:
  file list with +/− counts, then each file's hunks with line numbers;
  long lines scroll or wrap. It opens by itself for an approval on a wide
  screen unless you closed it; below 1180 px it slides over the run.
- **Message box** pinned at the bottom while the run can still read a
  message (open, waiting approval, needs attention, parked): posts to
  `/v1/runs/{id}/messages` with an `Idempotency-Key` made per message and
  kept until it is accepted, so resending after a dropped connection can't
  deliver it twice. ⌘/Ctrl-Enter sends. The hint says when it will be read.
  On a finished Claude Code or Codex run the same box is **Follow up**: it
  posts to `/v1/runs/{id}/follow-up` and opens the new run.

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

Tokens live on `:root` (light) and are redefined for dark under
`prefers-color-scheme: dark` unless `data-theme="light"`, and under
`data-theme="dark"`. One attention colour (`--attn`) is used only for
"needs you"; the brand teal (`--accent`) for selection, primary actions
and working. Spacing is a 4 px scale (`--s1`…`--s10`), radii `--r1`…`--r3`,
elevation `--e1`/`--e2`, motion `--t-fast`/`--t-med` (zero under
`prefers-reduced-motion`). Inter for the interface, Bricolage Grotesque for
the brand, the system monospace for code (D12 picks a bundled one).
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
