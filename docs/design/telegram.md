# Telegram: the owner's phone

Telegram is the owner's primary surface ([positioning.md](../positioning.md)): delegate work, get told when something finished or needs a decision, and answer with one tap. The web dashboard is for details; Telegram is for the day.

Status: *(built)*. The client, commands, long-poll loop and notifier are in `crates/shiioo-telegram`; the office side (what the commands do, how approvals are created and expire, what is notified) is the `Office` and `Notifications` implementations in the `shiioo` binary and `shiioo-api`. Items marked *(planned)* are not there yet.

## Why Telegram, and why long polling

- Push notifications on the phone come free, abroad too.
- The bot **long-polls** Telegram's API (`getUpdates`, 30-second waits). Shiioo dials out; nothing listens on a public address. The office can sit behind NAT and Tailscale and still reach the owner.
- Plain text only. Replies and notifications contain tool output and file names, which Markdown would mangle; no parse mode is set, so nothing is interpreted.

## Setup

1. Create a bot with [@BotFather](https://t.me/BotFather); it gives you a token.
2. Find your own numeric user id (for example by messaging `@userinfobot`).
3. On the office machine: the token in `SHIIOO_TELEGRAM_TOKEN` or in `.shiioo/telegram-token` (mode 0600; `shiioo install --telegram-token …` writes it for the service), and your id as `owner_chat_id` under `[telegram]` in `.shiioo/config.toml` (or `SHIIOO_TELEGRAM_OWNER`). With a token but no owner the bot stays off and the server says so at start.
4. Send `/start` to the bot. Only messages from the owner id get an answer.

## Commands

| You send | What happens |
|---|---|
| plain text, e.g. `fix the flaky test in net.rs` | Starts a run with that task. With one repo under `[repos]` it is used; with several the bot asks you to use `/run <repo> …`; with none the run has no repository. The employee is chosen by capacity (`auto`). Reply: `▶️ <id> started on <repo> (<employee>): <task>` and the branch. |
| `/run <repo> <task…>` | Same, on a named repo. `<repo>` is one of the labels from `/repos`; if the first word isn't a label, the whole text is the task. |
| `/status` | The 15 most recently updated runs, one line each: state glyph, id prefix, task, and what a waiting run waits for; then the capacity line. *(built)* |
| `/capacity` | How much of each subscription's five-hour and seven-day window is used, and when it resets. *(built)* |
| `/log <run>` | The run's last 25 steps, one line each. Accepts an id prefix (4+ characters). *(built)* |
| `/cancel <run>` | Asks the run to stop at its next step boundary (`CancelRequested`). *(built)* |
| `/repos` | The repo labels you can use in `/run`. *(built)* |
| `/run @<role> [<repo>] <task…>`, or `@<role> <task…>` | The same, as a configured role (`[[roles]]`): its instructions, tools, model, level and limits over the employee's. `/roles` lists them. *(built)* |
| `/plan [<repo>] <goal…>` | Asks the chief of staff for a plan: a read-only run on the repo that answers with work items. The plan arrives as a message when the run ends. *(built)* |
| `/plan approve <id>`, `/plan show <id>`, `/plans` | Creates one labelled issue per item (the intake works them), shows a plan with every item's details, lists plans. *(built)* |
| `/routines`, `/routine <name>` | The scheduled routines and the report with their next firing; fire one now. *(built)* |
| `/report` | The morning report, now. *(built)* |
| `/approve <id>`, `/deny <id>` | Answer an approval without the buttons (for example from a client that doesn't show them). *(built)* |
| `/help`, `/start` | The command list. *(built)* |

Commands are case-insensitive and accept a bot suffix (`/status@shiioo_bot`), which Telegram adds in group chats. Anything unrecognised gets the help text back.

Replies longer than Telegram's limit are split at line boundaries into several messages.

## Approval flow

When a harness wants to do something the rules mark **ask** (Shiioo's own tools *(planned)*), the server:

1. records `ApprovalRequested` in the run's log and holds the harness's hook request open; the run waits in place ([governance.md §3.0](governance.md#30-today-the-owner-is-the-board-built-shiioo-api));
2. sends the owner a message with two inline buttons:

```
🔐 7f3a21c9 wants to run
Bash: git push origin shiioo/7f3a21c9
https://studio.tail1234.ts.net/#run=7f3a21c9-…

[ ✅ Allow ]  [ ⛔ Deny ]
```

   The second line is the rule set's `summarize` of the call; the link appears when `[dashboard] url` is set. *(planned)* The rule's reason and the time left.

3. on a button press (`approve:<id>` / `deny:<id>` as callback data), calls the office's `decide`, which records `ApprovalDecided { by: "telegram" }` and answers the hook;
4. acknowledges the press (a short toast), and edits the original message: the buttons disappear and a footer is appended, `✅ allowed by you` or `⛔ denied by you`. A second press on a stale message gets `⚠️ <reason>` (already decided, or expired).

Approvals bind to the exact request (one `ApprovalRequested`, answered once), so a later `git push`, even the same command, asks again. If nobody answers before the expiry (just under six hours, the harness hook's own timeout minus a margin; per-rule expiries are planned), the server records `ApprovalDecided { decision: deny, by: timeout }`, the hook denies the call, the message is edited to show it expired, and the run continues with a denial the agent can see.

The office tracks the message id returned by `ask_approval`, so it can edit the request when it expires or is answered another way (`/approve`, the dashboard).

## Notifications

The server speaks first in these cases (`Notifier::notify`), each as one short message:

| Event | Message |
|---|---|
| run finished | `✅ 7f3a21c9 finished: <task>` + the agent's final summary (first 600 characters) + the dashboard link |
| run failed | `❌ 7f3a21c9 failed: <task>` + the reason |
| run cancelled | `🛑 7f3a21c9 <reason>: <task>` |
| run needs attention | `⚠️ 7f3a21c9 needs you: <task>` + the reason (for example a side-effecting call with an unknown outcome) |
| run stopped without finishing | `⏸ 7f3a21c9 stopped without finishing: <task>` (the worker gave up, for example after too many harness restarts) |
| approval requested | the approval message above |
| capacity | *(planned)* a warning when a subscription window passes a threshold (`CapacityObserved`) |
| morning report | `[report] schedule` (for example `weekdays 07:00`): what finished, failed or needs you since the last report, approvals waiting, branches awaiting review, capacity, next routines *(built)* |
| routine started | `⏰ routine <name> started run <id> (<employee>): <task>` *(built)* |
| plan ready | `📋 plan <id> for <repo>: <goal>`, the items, and the `/plan approve` line; a plan that could not be read says so *(built)* |

The task is cut to 120 characters, the summary to 600; the branch or PR link is in the summary if the agent put it there *(a dedicated link is planned)*. Notifications are recorded text, nothing else; the office's secrets and its API token are redacted before recording ([security.md §3.2](security.md#32-secrets-never-reach-the-record-built-never-reach-the-model-holds-for-the-native-loop-planned)), so a leaked token shows as `[secret:<name>]` on the phone too. Anything else the agent printed arrives as is.

## Security

- **Owner gate.** Every update carries the sender's user id. Messages and button presses from anyone but the configured owner are ignored (and logged once per sender). Only the owner's private chat receives notifications. Group chats are not supported on purpose.
- **Token.** The token is read from `SHIIOO_TELEGRAM_TOKEN` or the 0600 file `.shiioo/telegram-token`, never from `config.toml` or the log. It is sent to Telegram in the request URL, which is Telegram's design; the connection is HTTPS.
- **No inbound traffic.** Long polling means no webhook, no public URL, no TLS certificate to manage. Nothing in Shiioo accepts connections because of Telegram.
- **What a stolen token can do:** read the owner's messages to the bot, send messages as the bot, and, because it can read updates, see approval requests and press buttons in the office's name. **What it cannot do:** reach the office machine, read the event log, or start runs without Shiioo accepting the command from the owner id (the attacker would need to spoof Telegram's own `from.id`, which Telegram sets). Rotate the token with @BotFather if in doubt; Shiioo picks the new one up on restart.
- **What the bot cannot grant.** Approvals only answer questions the rules asked. The bot has no command to change rules, add repos or disable isolation; those live in the server's configuration on the office machine.
- **Audit.** Every command that changes anything is an event in a run's log: a decision as `ApprovalDecided { by: "telegram" }`, a cancel as `CancelRequested { by: "telegram" }`, a started run as its `RunCreated`.

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
