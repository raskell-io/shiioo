//! The owner's phone (`docs/design/telegram.md`).
//!
//! A deliberately small Telegram Bot API client over plain HTTP, the
//! commands the owner can send, the long-poll loop that dispatches them to
//! the office, and the notifier the server uses to speak first. Long
//! polling means the office needs no public address: it dials out to
//! Telegram, like everything else in Shiioo.

mod api;
mod bot;
mod command;

pub use api::{Bot, BotInfo, InlineButton, InlineKeyboard, TelegramError, Update, UpdateKind};
pub use bot::{BoxFuture, Notifier, Office, chunks, help_text, run_bot};
pub use command::{Command, approval_keyboard, parse_callback, parse_command};
