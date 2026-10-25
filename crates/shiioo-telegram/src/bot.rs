//! The long-poll loop that turns the owner's messages into office actions,
//! and the notifier the office uses to speak first.

use crate::api::{Bot, TelegramError, Update, UpdateKind};
use crate::command::{Command, approval_keyboard, parse_callback, parse_command};
use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Telegram rejects messages over 4096 characters; stay under it.
const MESSAGE_LIMIT: usize = 4000;
const POLL_SECONDS: u32 = 30;
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// What the bot can ask the office to do. Implemented by the server.
/// Replies are plain text for the owner.
pub trait Office: Send + Sync {
    /// Labels of the repositories the owner can name in `/run`.
    fn repos(&self) -> Vec<String>;
    /// Starts a run. `Ok` is the reply, `Err` a user-facing refusal.
    fn start_run(
        &self,
        repo: Option<String>,
        task: String,
    ) -> BoxFuture<'_, Result<String, String>>;
    fn status(&self) -> BoxFuture<'_, String>;
    /// Remaining capacity per provider (subscription windows), as text.
    fn capacity(&self) -> BoxFuture<'_, String>;
    fn log(&self, run: String) -> BoxFuture<'_, String>;
    fn cancel(&self, run: String) -> BoxFuture<'_, Result<String, String>>;
    /// Records the owner's decision on an approval request.
    fn decide(&self, approval: String, allow: bool) -> BoxFuture<'_, Result<String, String>>;
}

pub fn help_text() -> &'static str {
    "Shiioo, your virtual company.\n\n\
     Send a task as plain text to start a run, or:\n\
     /run <repo> <task> – run a task on a named repo\n\
     /status – what everyone is doing\n\
     /capacity – how much of this week's subscription windows is left\n\
     /log <run> – a run's steps (id or prefix)\n\
     /cancel <run> – stop a run\n\
     /repos – repos you can name in /run\n\
     /approve <id>, /deny <id> – answer a request without the buttons\n\
     /help – this text"
}

/// Polls Telegram forever (until the future is dropped) and dispatches the
/// owner's messages and button presses to the office. Everyone else is
/// ignored. Network errors are logged and retried with backoff. Never
/// panics.
pub async fn run_bot(bot: Arc<Bot>, owner_chat_id: i64, office: Arc<dyn Office>) {
    let mut offset = 0i64;
    let mut backoff = Duration::from_secs(1);
    let mut strangers: HashSet<i64> = HashSet::new();
    loop {
        let updates = match bot.get_updates(offset, POLL_SECONDS).await {
            Ok(updates) => {
                backoff = Duration::from_secs(1);
                updates
            }
            Err(e) => {
                eprintln!("telegram: polling failed: {e}; retrying in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(MAX_BACKOFF);
                continue;
            }
        };
        for update in updates {
            offset = offset.max(update.update_id + 1);
            handle(&bot, owner_chat_id, office.as_ref(), update, &mut strangers).await;
        }
    }
}

async fn handle(
    bot: &Bot,
    owner: i64,
    office: &dyn Office,
    update: Update,
    strangers: &mut HashSet<i64>,
) {
    match update.kind {
        UpdateKind::Message {
            chat_id,
            from_id,
            text,
        } => {
            if from_id != Some(owner) || chat_id != owner {
                if strangers.insert(from_id.unwrap_or(chat_id)) {
                    eprintln!("telegram: ignoring messages from {from_id:?} in chat {chat_id}");
                }
                return;
            }
            let reply = reply_to(office, parse_command(&text, &office.repos())).await;
            for chunk in chunks(&reply, MESSAGE_LIMIT) {
                if let Err(e) = bot.send_message(owner, &chunk, None).await {
                    eprintln!("telegram: sending reply failed: {e}");
                    return;
                }
            }
        }
        UpdateKind::Callback {
            id,
            chat_id,
            message_id,
            from_id,
            data,
            text,
        } => {
            if from_id != owner {
                if strangers.insert(from_id) {
                    eprintln!("telegram: ignoring button presses from {from_id}");
                }
                bot.answer_callback_query(&id, Some("Not yours."))
                    .await
                    .ok();
                return;
            }
            let Some(command) = parse_callback(&data) else {
                bot.answer_callback_query(&id, Some("Unknown button."))
                    .await
                    .ok();
                return;
            };
            let (approval, allow) = match command {
                Command::Approve { approval } => (approval, true),
                Command::Deny { approval } => (approval, false),
                _ => return,
            };
            let (toast, footer) = match office.decide(approval, allow).await {
                Ok(_) if allow => ("Allowed", "✅ allowed by you".to_owned()),
                Ok(_) => ("Denied", "⛔ denied by you".to_owned()),
                Err(e) => ("Couldn't record that", format!("⚠️ {e}")),
            };
            bot.answer_callback_query(&id, Some(toast)).await.ok();
            let edited = format!("{text}\n\n{footer}");
            if let Err(e) = bot
                .edit_message_text(chat_id, message_id, &edited, None)
                .await
            {
                eprintln!("telegram: editing approval message failed: {e}");
            }
        }
        UpdateKind::Other => {}
    }
}

async fn reply_to(office: &dyn Office, command: Command) -> String {
    match command {
        Command::Run { repo, task } => match office.start_run(repo, task).await {
            Ok(reply) => reply,
            Err(e) => format!("Couldn't start that: {e}"),
        },
        Command::Status => office.status().await,
        Command::Capacity => office.capacity().await,
        Command::Log { run } => office.log(run).await,
        Command::Cancel { run } => match office.cancel(run).await {
            Ok(reply) => reply,
            Err(e) => format!("Couldn't cancel: {e}"),
        },
        Command::Repos => {
            let repos = office.repos();
            if repos.is_empty() {
                "No repos configured. Plain-text tasks run in the default workspace.".to_owned()
            } else {
                format!("Repos you can name in /run:\n{}", repos.join("\n"))
            }
        }
        Command::Help => help_text().to_owned(),
        Command::Approve { approval } => decision_reply(office, approval, true).await,
        Command::Deny { approval } => decision_reply(office, approval, false).await,
        Command::Unknown(text) => {
            format!("I didn't understand {text:?}.\n\n{}", help_text())
        }
    }
}

async fn decision_reply(office: &dyn Office, approval: String, allow: bool) -> String {
    match office.decide(approval, allow).await {
        Ok(reply) => reply,
        Err(e) => format!("Couldn't record that: {e}"),
    }
}

/// Splits text into pieces of at most `max` characters, preferring line
/// boundaries.
pub fn chunks(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in text.split_inclusive('\n') {
        if current.chars().count() + line.chars().count() > max && !current.is_empty() {
            out.push(std::mem::take(&mut current));
        }
        if line.chars().count() > max {
            let mut piece = String::new();
            for ch in line.chars() {
                if piece.chars().count() == max {
                    out.push(std::mem::take(&mut piece));
                }
                piece.push(ch);
            }
            current.push_str(&piece);
        } else {
            current.push_str(line);
        }
    }
    if !current.is_empty() || out.is_empty() {
        out.push(current);
    }
    out
}

/// Messages the office sends on its own: results, failures, requests.
pub struct Notifier {
    bot: Arc<Bot>,
    owner_chat_id: i64,
}

impl Notifier {
    pub fn new(bot: Arc<Bot>, owner_chat_id: i64) -> Self {
        Self { bot, owner_chat_id }
    }

    pub async fn notify(&self, text: &str) -> Result<(), TelegramError> {
        for chunk in chunks(text, MESSAGE_LIMIT) {
            self.bot
                .send_message(self.owner_chat_id, &chunk, None)
                .await?;
        }
        Ok(())
    }

    /// Asks the owner to allow or deny something. Returns the message id,
    /// so the request can be edited later (for example when it expires).
    pub async fn ask_approval(&self, approval_id: &str, text: &str) -> Result<i64, TelegramError> {
        self.bot
            .send_message(
                self.owner_chat_id,
                text,
                Some(&approval_keyboard(approval_id)),
            )
            .await
    }

    /// Marks an approval message as settled: appends `outcome` and removes
    /// the buttons.
    pub async fn resolve_approval(
        &self,
        message_id: i64,
        original_text: &str,
        outcome: &str,
    ) -> Result<(), TelegramError> {
        self.bot
            .edit_message_text(
                self.owner_chat_id,
                message_id,
                &format!("{original_text}\n\n{outcome}"),
                None,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunking_prefers_line_breaks_and_handles_long_lines() {
        assert_eq!(chunks("", 10), vec![String::new()]);
        assert_eq!(chunks("short", 10), vec!["short".to_owned()]);
        let text = "aaaa\nbbbb\ncccc\n";
        assert_eq!(
            chunks(text, 10),
            vec!["aaaa\nbbbb\n".to_owned(), "cccc\n".to_owned()]
        );
        let long = "x".repeat(25);
        let parts = chunks(&long, 10);
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.chars().count() <= 10));
        assert_eq!(parts.concat(), long);
    }
}
