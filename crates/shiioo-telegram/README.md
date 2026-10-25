# shiioo-telegram

The owner's phone. A deliberately small Telegram Bot API client over plain HTTP, the commands the owner can send, the long-poll loop that dispatches them to the office, and the notifier the server uses to speak first: results, failures and approval requests with Allow / Deny buttons. Long polling means the office needs no public address; it dials out to Telegram like everything else in Shiioo.

## Where it sits

- Depends on: `serde`, `serde_json`, `thiserror`, `reqwest`, `tokio` (`time`, `sync`). Dev: `axum` for a fake Telegram server.
- Used by: the `shiioo` binary. The server implements `Office` and spawns `run_bot`; it holds a `Notifier` for outgoing messages. Nothing here knows about runs or events.

## Public API

| Item | What it is |
|---|---|
| `Bot::new(token)`, `.with_base_url(url)` | The HTTP client. `with_base_url` points it at a fake for tests. |
| `Bot::get_me()`, `get_updates(offset, timeout_s)`, `send_message(chat_id, text, keyboard)`, `edit_message_text(chat_id, message_id, text, keyboard)`, `answer_callback_query(id, text)` | The five Bot API methods used. `send_message` returns the message id. |
| `BotInfo`, `Update`, `UpdateKind::{Message, Callback, Other}`, `InlineKeyboard`, `InlineButton`, `TelegramError` | Wire types. `TelegramError` is `Http`, `Api { code, description }` or `Decode`. |
| `Command` | `Run { repo, task }`, `Status`, `Capacity`, `Log { run }`, `Cancel { run }`, `Repos`, `Help`, `Approve { approval }`, `Deny { approval }`, `Unknown(text)`. |
| `parse_command(text, known_repos)`, `parse_callback(data)`, `approval_keyboard(approval_id)` | Text to command; button data to command; the Allow / Deny keyboard. |
| `Office` | What the bot asks the server for: `repos`, `start_run`, `status`, `capacity`, `log`, `cancel`, `decide`. Replies are plain text. |
| `run_bot(bot, owner_chat_id, office)` | The poll loop. Runs until dropped, never panics, retries with backoff. |
| `Notifier::new(bot, owner_chat_id)` | `notify(text)`, `ask_approval(approval_id, text) -> message_id`, `resolve_approval(message_id, original_text, outcome)`. |
| `chunks(text, max)`, `help_text()`, `BoxFuture` | Message splitting at line boundaries; the `/help` reply; the future type `Office` returns. |

## Using it

```rust
use shiioo_telegram::{Bot, Notifier, run_bot};
use std::sync::Arc;

let bot = Arc::new(Bot::new(token));
let me = bot.get_me().await?;                     // fails fast on a bad token
tokio::spawn(run_bot(Arc::clone(&bot), owner_chat_id, office));

let notifier = Notifier::new(bot, owner_chat_id);
let msg = notifier.ask_approval("ap-1", "Bash: git push origin shiioo/b619ff8d\nrule 11: pushes leave the machine").await?;
// later, if the owner never answers:
notifier.resolve_approval(msg, "…", "⌛ expired").await?;
```

`tests/bot.rs` drives the same API against an `axum` fake.

## Testing

```sh
cargo test -p shiioo-telegram
```

Integration tests start a fake that serves `POST /bot{token}/{method}`, feed one batch of updates (an owner `/status`, a stranger's `/status`, an owner button press), and assert the office saw exactly the owner's command and decision, the stranger was ignored, and the approval message was edited with `allowed by you`. A second test checks `ask_approval` sends the two buttons. Unit tests cover the command table, callback parsing, the keyboard, chunking and update parsing.

## Invariants

- Only the owner is served: a message must come from `owner_chat_id` in that chat; a button press must come from the owner. Strangers are logged once per id and ignored; their button presses get a `Not yours.` toast.
- Messages are sent without a parse mode, so arbitrary tool output cannot break formatting. Text is split into chunks of at most 4 000 characters, preferring line breaks.
- Long polls last 30 s (Telegram's cap is 50). On a polling error the loop sleeps 1 s, doubling up to 30 s, and resets on success.
- Callback data is `approve:<id>` or `deny:<id>` and must fit Telegram's 64 bytes; a UUID does.

Wire details and the command grammar: [`docs/protocol.md`](docs/protocol.md).

## Further reading

- [Telegram design](../../docs/design/telegram.md), [architecture overview](../../docs/architecture.md), [security design](../../docs/design/security.md) for the owner gate.
- [ADR-0006 Daemon dials out](../../docs/adr/0006-daemon-dials-out-over-websocket.md): the same no-inbound-port rule applied to nodes.
