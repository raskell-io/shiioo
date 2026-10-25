# Telegram protocol

What goes over the wire between the office and Telegram, which updates the bot acts on, the command grammar, and the approval round trip.

## Transport

Every call is `POST {base}/bot{token}/{method}` with a JSON body. Telegram answers `{"ok":true,"result":…}` or `{"ok":false,"error_code":…,"description":…}`; the latter becomes `TelegramError::Api`. `base` is `https://api.telegram.org` unless `with_base_url` changed it. The HTTP client times out after 30 s, except for the long poll, which gets `timeout + 10` s.

| Method | Body | Used for |
|---|---|---|
| `getMe` | `{}` | Startup check; returns the bot's id and username. |
| `getUpdates` | `offset`, `timeout` (≤ 50), `allowed_updates: ["message", "callback_query"]` | The long poll. The HTTP timeout is `timeout + 10` s. |
| `sendMessage` | `chat_id`, `text`, optional `reply_markup` | Replies and notifications. Plain text, no `parse_mode`. |
| `editMessageText` | `chat_id`, `message_id`, `text`, `reply_markup` | Settling an approval message; a missing keyboard removes the buttons. |
| `answerCallbackQuery` | `callback_query_id`, optional `text` | Acknowledging a button press; `text` shows as a toast. |

## Updates

`get_updates` returns `Update { update_id, kind }`:

| `UpdateKind` | Fields | Source |
|---|---|---|
| `Message` | `chat_id`, `from_id: Option<i64>`, `text` | A text message. |
| `Callback` | `id`, `chat_id`, `message_id`, `from_id`, `data`, `text` | An inline button press; `text` is the message the button belongs to, so it can be edited in place. |
| `Other` | | Photos, edits, joins, anything else. Ignored. |

The loop keeps `offset = max(offset, update_id + 1)` and sends it with the next poll, which acknowledges everything seen so far. There is no persistence of the offset: after a restart Telegram re-delivers updates that were not acknowledged, which the owner gate and the office's own idempotency have to tolerate.

## The owner gate

- A `Message` is handled only when `from_id == owner` and `chat_id == owner`. Group chats therefore never work, even with the owner in them.
- A `Callback` is handled only when `from_id == owner`. Others get the toast `Not yours.`.
- Each unknown sender id is logged once (`ignoring messages from … in chat …`) and then silently dropped.

## Commands

`parse_command(text, known_repos)`; a `@botname` suffix on the command word is stripped, so `/status@shiioo_bot` works in a group-style client.

| Input | `Command` | Office call |
|---|---|---|
| plain text without a leading `/` | `Run { repo: None, task }` | `start_run(None, task)` |
| `/run <repo> <task…>` where `<repo>` is a known label | `Run { repo: Some(label), task }` | `start_run(Some(label), task)` |
| `/run <task…>` | `Run { repo: None, task }` | `start_run(None, task)` |
| `/status` | `Status` | `status()` |
| `/capacity` | `Capacity` | `capacity()` |
| `/log <run-id-or-prefix>` | `Log { run }` | `log(run)` |
| `/cancel <run-id-or-prefix>` | `Cancel { run }` | `cancel(run)` |
| `/repos` | `Repos` | `repos()` |
| `/help`, `/start` | `Help` | none; `help_text()` |
| `/approve <id>`, `/deny <id>` | `Approve`, `Deny` | `decide(id, true/false)` |
| anything else, or `/run`, `/log`, `/cancel` without an argument | `Unknown(text)` | none; `I didn't understand "…"` plus the help text |

`start_run`, `cancel` and `decide` return `Result<String, String>`; the `Err` text is shown as `Couldn't start that: …`, `Couldn't cancel: …`, `Couldn't record that: …`. Every reply is sent in chunks of at most 4 000 characters.

## Approvals

1. The server calls `Notifier::ask_approval(approval_id, text)`. The message carries one keyboard row: `✅ Allow` with data `approve:<id>` and `⛔ Deny` with data `deny:<id>`. The returned `message_id` is the server's handle.
2. The owner presses a button (or types `/approve <id>` / `/deny <id>`).
3. The bot calls `office.decide(id, allow)`, then:
   - toast `Allowed` or `Denied`, and the message is edited to `<original text>\n\n✅ allowed by you` or `⛔ denied by you`, keyboard removed;
   - on `Err(e)`: toast `Couldn't record that`, footer `⚠️ <e>`, keyboard removed as well.
4. If the request is settled some other way (it expired, the run was cancelled), the server calls `resolve_approval(message_id, original_text, outcome)` to append the outcome and remove the buttons.

Unknown callback data gets the toast `Unknown button.` and no edit. Because the decision is recorded by the office before the message is edited, a second press of a stale button is the office's problem to reject; the bot simply shows whatever `decide` answers.

## Message size

`chunks(text, 4000)` splits long replies, cutting at the last line break that fits and only mid-line when a single line is longer than the limit. Telegram's hard limit is 4 096 characters.

## Not covered today

- Webhooks; the bot only long-polls.
- Group chats, multiple owners, or a per-command allow list.
- Markdown or HTML formatting, photos, files, voice.
- Persisting the update offset across restarts.

Related: [Telegram design](../../../docs/design/telegram.md), [governance design](../../../docs/design/governance.md) for what gets asked in the first place.
