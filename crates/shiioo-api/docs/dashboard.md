# The dashboard

A phone-sized web UI with no build step: plain files under `ui/`, the
scripts as native ES modules, embedded into the binary and served by the
API at `/`, `/ui` and `/ui/{path}`. It speaks only to the HTTP API described in
[http.md](http.md), so anything it shows can also be fetched with `curl`.

## Files

| File | Lines | Role |
| --- | --- | --- |
| `ui/index.html` | ~25 | Shell: viewport and iOS web-app meta tags, theme colour `#2f7d73`, header with brand link, **New task** and refresh buttons, `<main id="app">`, `<script type="module" src="/ui/app.js">`. |
| `ui/app.js` | ~25 | Entry module: hash routing between the views. |
| `ui/core.js` | ~100 | Token handling, `api()`, DOM helpers, `showError`, the token form, the current view's teardown (`onLeave` / `leave`). |
| `ui/stream.js` | ~70 | The SSE client (`streamEvents`, `parseFrame`). |
| `ui/describe.js` | ~40 | One line per event, mirroring the server's `describe()`. |
| `ui/views/runs.js` | ~70 | The run list and the new-task form. |
| `ui/views/run.js` | ~95 | One run: header, approval card, actions, diff, live log. |
| `ui/app.css` | ~100 | Colour tokens on `:root` with a `prefers-color-scheme: dark` override, buttons, status colours, the approval card, the log. |

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

- `#` (empty): the run list.
- `#run=<id>`: one run.

Telegram notifications link straight to `#run=<id>` using the `[dashboard]
url` from `config.toml` (or the listen address).

## Run list

- A capacity line from `GET /v1/capacity`, rendered as `provider · 5h N% ·
  7d N%` per provider.
- One row per run from `GET /v1/runs`: a glyph for the state (▶ open, 🔐
  waiting approval, ⚠️ needs attention, ✅ completed, ❌ failed, 🛑
  cancelled), the task, how long ago the last event was, and the short run id.
- Refreshes every 10 s while the tab is visible.
- **New task** opens a form with a task field and an optional repository
  field; submit posts `{"task", "employee": "auto", "repo"?}` to `POST
  /v1/runs`, so the capacity broker picks the employee.

## Run view

- Header with the task and state.
- An **approval card** when the status is `waiting_approval`: the summary
  and **Allow** / **Deny** buttons that post to `POST /v1/approvals/{id}`.
- **Cancel** needs two taps: the first arms it for four seconds, the second
  posts `{"reason": "from dashboard"}` to `/cancel`.
- **Resume** when the run is open, posting to `/resume`.
- **Diff** fetches `/diff` and shows it in a `<pre class="diff">`.
- The **log**: every event rendered with the same one-line `describe()` the
  server uses, newest at the bottom; the owner's messages read "you: …" in
  the accent colour.
- The **message box**, pinned under the log while the run can still read a
  message (open, waiting approval, needs attention, parked): posts to
  `/v1/runs/{id}/messages` with an `Idempotency-Key` made per message and
  kept until it is accepted, so resending after a dropped connection can't
  deliver it twice. ⌘/Ctrl-Enter sends. The hint says when it will be read:
  at the next turn (native), when Claude Code or Codex finishes its current
  step (read from the run's spec in the first log event), or "it is waiting
  for your reply" for a parked run. The message appears in the log through
  the live stream, not optimistically.

## Live log

The log is a hand-rolled SSE client built on `fetch` rather than
`EventSource`, because `EventSource` cannot send an `Authorization` header:

- reads the response body as a stream, splits it into frames on blank lines,
  normalises `\r\n`, ignores `:` comment lines, and parses `id:`, `event:`
  and `data:`;
- remembers the last `id` and reconnects with `Last-Event-ID`, backing off
  from 1 s to 10 s;
- stops reconnecting after a terminal payload (`run_completed`,
  `run_failed`, `run_cancelled`);
- re-renders the run header at most once per 200 ms while events arrive.

## Styling

Colour tokens live on `:root` and are redefined under `@media
(prefers-color-scheme: dark)`. Classes: `.btn` with `.primary`, `.danger`,
`.allow` and `.deny` variants, `.status-<state>` for the glyph colours
(`open`, `completed`, `failed`, `cancelled` have their own colour),
`.card.approval` for the pending question, `.log` and `.line` for the event
list, `.sheet` for the forms. Touch targets are
sized for a phone; there is no desktop-specific layout.

## Changing it

Edit the files under `ui/` and rebuild; they are included with
`include_str!`, so the binary always ships the version it was built with.
A new file goes into `UI_FILES` in `src/lib.rs`: `tests/dashboard.rs`
fails when that list and the directory disagree, or when a module imports
a file that is not served. It also checks the files are served without a
token and that the diff endpoint behaves; there is no browser test.

See also: [telegram design](../../../docs/design/telegram.md) for the other
phone surface, [deployment design](../../../docs/design/deployment.md) for
reaching the dashboard over Tailscale.
