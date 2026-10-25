# Telegram: the owner's phone

Telegram is the owner's primary surface ([positioning.md](../positioning.md)): delegate work, get told when something finished or needs a decision, and answer with one tap. The web dashboard is for details; Telegram is for the day.

Status: the client, commands, long-poll loop and notifier are built in `crates/shiioo-telegram`. The office side (what `/status` shows, how approvals are created and expire) is built by the server and is *(built)* where marked.

## Why Telegram, and why long polling

- Push notifications on the phone come free, abroad too.
- The bot **long-polls** Telegram's API (`getUpdates`, 30-second waits). Shiioo dials out; nothing listens on a public address. The office can sit behind NAT and Tailscale and still reach the owner.
- Plain text only. Replies and notifications contain tool output and file names, which Markdown would mangle; no parse mode is set, so nothing is interpreted.

## Setup

1. Create a bot with [@BotFather](https://t.me/BotFather); it gives you a token.
2. Find your own numeric user id (for example by messaging `@userinfobot`).
3. On the office machine: `SHIIOO_TELEGRAM_TOKEN=<token>` and `SHIIOO_TELEGRAM_OWNER=<your id>` in the server's environment. The server refuses to start the bot with one but not the other.
4. Send `/start` to the bot. Only messages from the owner id get an answer.

## Commands

| You send | What happens |
|---|---|
| plain text, e.g. `fix the flaky test in net.rs` | Starts a run with that task on the default repo (or workspace). Reply: the run id, the branch, and how to follow it. |
| `/run <repo> <task…>` | Same, on a named repo. `<repo>` is one of the labels from `/repos`; if the first word isn't a label, the whole text is the task. |
| `/status` | One line per open run: id prefix, repo, what it's doing right now, how long it has been running; then runs waiting on you. *(office side planned)* |
| `/log <run>` | The run's steps, compacted: one line per model message and tool call, the last ones first. Accepts an id prefix. *(office side planned)* |
| `/cancel <run>` | Asks the run to stop at its next step boundary. *(planned, needs `CancelRequested`)* |
| `/repos` | The repo labels you can use in `/run`. |
| `/approve <id>`, `/deny <id>` | Answer an approval without the buttons (for example from a client that doesn't show them). |
| `/help`, `/start` | The command list. |

Commands are case-insensitive and accept a bot suffix (`/status@shiioo_bot`), which Telegram adds in group chats. Anything unrecognised gets the help text back.

Replies longer than Telegram's limit are split at line boundaries into several messages.

## Approval flow

When a harness or a Shiioo tool wants to do something the rules mark **ask**, the server:

1. records `ApprovalRequested` in the run's log and parks the run;
2. sends the owner a message with two inline buttons:

```
🟡 sango · run 7f3a21c9 wants to run

    git push origin shiioo/7f3a21c9

Reason: the agent finished the fix and wants to open a PR.
Expires in 30 min.

[ ✅ Allow ]  [ ⛔ Deny ]
```

3. on a button press (`approve:<id>` / `deny:<id>` as callback data), calls the office's `decide`, which records `ApprovalDecided` and wakes the run;
4. acknowledges the press (a short toast), and edits the original message: the buttons disappear and a footer is appended, `✅ allowed by you` or `⛔ denied by you`. A second press on a stale message gets `⚠️ <reason>` (already decided, or expired).

Approvals bind to the exact call (tool, input digest, run), so a later, different `git push` asks again. If nobody answers before the expiry *(planned: 30 minutes by default, per rule)*, the server records the expiry, denies the call, edits the message to `⏰ expired`, and the run continues with a denial the agent can see.

The office tracks the message id returned by `ask_approval`, so it can edit the request when it expires or is answered another way (`/approve`, the dashboard).

## Notifications

The server speaks first in these cases (`Notifier::notify`), each as one short message:

| Event | Message |
|---|---|
| run finished | `✅ sango · 7f3a21c9 done in 12 min` + the agent's final summary (first lines) + the branch or PR link |
| run failed | `❌ sango · 7f3a21c9 failed: <reason>` + `/log 7f3a21c9` |
| run needs attention | `🟠 sango · 7f3a21c9 needs you: <reason>` (for example a side-effecting call with an unknown outcome) |
| approval requested | the approval message above |
| capacity | *(planned)* a warning when a subscription window passes a threshold (`CapacityObserved`) |
| morning report | *(planned, phase 5)* |

Notifications never contain secrets: tool output is redacted by the node before it is recorded, and the bot only ever sends recorded text.

## Security

- **Owner gate.** Every update carries the sender's user id. Messages and button presses from anyone but `SHIIOO_TELEGRAM_OWNER` are ignored (and logged once per sender). Only the owner's private chat receives notifications. Group chats are not supported on purpose.
- **Token.** `SHIIOO_TELEGRAM_TOKEN` is read from the environment, never from config files or the log. It is sent to Telegram in the request URL, which is Telegram's design; the connection is HTTPS.
- **No inbound traffic.** Long polling means no webhook, no public URL, no TLS certificate to manage. Nothing in Shiioo accepts connections because of Telegram.
- **What a stolen token can do:** read the owner's messages to the bot, send messages as the bot, and, because it can read updates, see approval requests and press buttons in the office's name. **What it cannot do:** reach the office machine, read the event log, or start runs without Shiioo accepting the command from the owner id (the attacker would need to spoof Telegram's own `from.id`, which Telegram sets). Rotate the token with @BotFather if in doubt; Shiioo picks the new one up on restart.
- **What the bot cannot grant.** Approvals only answer questions the rules asked. The bot has no command to change rules, add repos or disable isolation; those live in the server's configuration on the office machine.
- **Audit.** Every command that changes anything (a started run, a decision) is an event in a run's log, attributed to the owner.

## Crate map

`crates/shiioo-telegram`:

- `api.rs`: `Bot` (`get_me`, `get_updates`, `send_message`, `edit_message_text`, `answer_callback_query`), `Update`/`UpdateKind`, `InlineKeyboard`.
- `command.rs`: `Command`, `parse_command`, `parse_callback`, `approval_keyboard`.
- `bot.rs`: the `Office` trait the server implements, `run_bot` (the loop), `Notifier`, `chunks`.

Tests: a parser table; a fake Telegram server (axum) that feeds an owner message, a stranger's message and a button press, and records what the bot sends back; the notifier's keyboard and splitting.

## Open questions

- Voice messages: Telegram delivers them as files; transcribing them locally (Whisper on the office machine) would make delegation from a walk natural. *(later)*
- A second owner, or a read-only viewer, means a list of allowed ids and per-id rights. Not before it is needed.
- Replies with photos or files (a rendered diff, a generated image): `sendDocument`/`sendPhoto` are easy to add when a run produces them.
