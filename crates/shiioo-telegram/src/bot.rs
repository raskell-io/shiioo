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
    /// Starts a run, as `role` if one was named. `Ok` is the reply, `Err`
    /// a user-facing refusal.
    fn start_run(
        &self,
        repo: Option<String>,
        role: Option<String>,
        machine: Option<String>,
        task: String,
    ) -> BoxFuture<'_, Result<String, String>>;
    fn status(&self) -> BoxFuture<'_, String>;
    /// The files a run produced, one line each.
    fn files(&self, run: String) -> BoxFuture<'_, Result<String, String>> {
        Box::pin(async move { Err(format!("no files for {run}")) })
    }
    /// One produced file: its name and bytes, for `sendDocument`.
    fn get_file(&self, run: String, path: String) -> BoxFuture<'_, Result<(String, Vec<u8>), String>> {
        Box::pin(async move { Err(format!("no file {path} for {run}")) })
    }
    /// The enrolled machines and whether they are online.
    fn machines(&self) -> BoxFuture<'_, String> {
        Box::pin(async { "No machines.".to_owned() })
    }
    /// A one-time enrollment token for a new node, and the command to run
    /// on it.
    fn enroll(&self, name: String, labels: Vec<String>) -> BoxFuture<'_, Result<String, String>> {
        Box::pin(async move { Err(format!("enrollment is not available ({name}, {labels:?})")) })
    }
    /// The configured roles, one line each.
    fn roles(&self) -> BoxFuture<'_, String> {
        Box::pin(async { "No roles configured.".to_owned() })
    }
    /// The configured routines and when they fire next.
    fn routines(&self) -> BoxFuture<'_, String> {
        Box::pin(async { "No routines configured.".to_owned() })
    }
    /// Fires a routine now.
    fn fire_routine(&self, name: String) -> BoxFuture<'_, Result<String, String>> {
        Box::pin(async move { Err(format!("no routine named {name}")) })
    }
    /// The morning report, now.
    fn report(&self) -> BoxFuture<'_, String> {
        Box::pin(async { "No report available.".to_owned() })
    }
    /// Asks the chief of staff for a plan.
    fn plan(&self, _repo: Option<String>, goal: String) -> BoxFuture<'_, Result<String, String>> {
        Box::pin(async move { Err(format!("planning is not available ({goal})")) })
    }
    fn plan_approve(&self, plan: String) -> BoxFuture<'_, Result<String, String>> {
        Box::pin(async move { Err(format!("no plan {plan}")) })
    }
    fn plan_show(&self, plan: String) -> BoxFuture<'_, Result<String, String>> {
        Box::pin(async move { Err(format!("no plan {plan}")) })
    }
    fn plans(&self) -> BoxFuture<'_, String> {
        Box::pin(async { "No plans.".to_owned() })
    }
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
     /run [@role] [on:machine] [<repo>] <task> – run a task, as a role, on a machine\n\
     /machines – enrolled machines; /enroll <name> [label=value] – add one\n\
     /plan [<repo>] <goal> – the chief of staff breaks a goal into issues\n\
     /plan approve <id> – create the issues; /plan show <id>, /plans\n\
     /status – what everyone is doing\n\
     /report – the morning report, now\n\
     /routines – scheduled work; /routine <name> fires one now\n\
     /roles – roles you can name with @\n\
     /capacity – how much of this week's subscription windows is left\n\
     /log <run> – a run's steps (id or prefix)\n\
     /files <run> – files the run produced; /get <run> <path> sends one\n\
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
            let command = parse_command(&text, &office.repos());
            if let Command::Get { run, path } = command {
                let reply = match office.get_file(run, path.clone()).await {
                    Ok((name, bytes)) => match bot.send_document(owner, &name, bytes, Some(&path)).await {
                        Ok(_) => return,
                        Err(e) => format!("Couldn't send {path}: {e}"),
                    },
                    Err(e) => format!("Couldn't get that: {e}"),
                };
                bot.send_message(owner, &reply, None).await.ok();
                return;
            }
            let reply = reply_to(office, command).await;
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
        Command::Run {
            repo,
            role,
            machine,
            task,
        } => match office.start_run(repo, role, machine, task).await {
            Ok(reply) => reply,
            Err(e) => format!("Couldn't start that: {e}"),
        },
        Command::Machines => office.machines().await,
        Command::Files { run } => match office.files(run).await {
            Ok(reply) => reply,
            Err(e) => format!("Couldn't list files: {e}"),
        },
        // Handled before reply_to: it sends a document, not text.
        Command::Get { path, .. } => format!("Couldn't send {path}."),
        Command::Enroll { name, labels } => match office.enroll(name, labels).await {
            Ok(reply) => reply,
            Err(e) => format!("Couldn't enroll: {e}"),
        },
        Command::Status => office.status().await,
        Command::Roles => office.roles().await,
        Command::Routines => office.routines().await,
        Command::Routine { name } => match office.fire_routine(name).await {
            Ok(reply) => reply,
            Err(e) => format!("Couldn't fire that: {e}"),
        },
        Command::Report => office.report().await,
        Command::Plan { repo, goal } => match office.plan(repo, goal).await {
            Ok(reply) => reply,
            Err(e) => format!("Couldn't plan that: {e}"),
        },
        Command::PlanApprove { plan } => match office.plan_approve(plan).await {
            Ok(reply) => reply,
            Err(e) => format!("Couldn't approve that: {e}"),
        },
        Command::PlanShow { plan } => match office.plan_show(plan).await {
            Ok(reply) => reply,
            Err(e) => format!("Couldn't show that: {e}"),
        },
        Command::Plans => office.plans().await,
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

    /// Sends a file to the owner with a caption.
    pub async fn send_file(
        &self,
        filename: &str,
        bytes: Vec<u8>,
        caption: &str,
    ) -> Result<(), TelegramError> {
        self.bot
            .send_document(self.owner_chat_id, filename, bytes, Some(caption))
            .await
            .map(drop)
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
