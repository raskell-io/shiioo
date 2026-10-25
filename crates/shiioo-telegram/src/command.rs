//! What the owner can say to the bot.

use crate::api::{InlineButton, InlineKeyboard};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// `/run <repo> <task…>`, `/run <task…>`, or plain text.
    Run {
        repo: Option<String>,
        task: String,
    },
    /// `/status`
    Status,
    /// `/capacity`: how much of the subscriptions' windows is used.
    Capacity,
    /// `/log <run-id-or-prefix>`
    Log {
        run: String,
    },
    /// `/cancel <run-id-or-prefix>`
    Cancel {
        run: String,
    },
    /// `/repos`
    Repos,
    /// `/help`, `/start`
    Help,
    /// `/approve <id>`, or the Allow button
    Approve {
        approval: String,
    },
    /// `/deny <id>`, or the Deny button
    Deny {
        approval: String,
    },
    Unknown(String),
}

/// Parses a message. Plain text without a leading slash is a task for the
/// default repo. In `/run`, a first word that matches a known repo label
/// selects that repo; otherwise the whole rest is the task.
pub fn parse_command(text: &str, known_repos: &[String]) -> Command {
    let text = text.trim();
    if text.is_empty() {
        return Command::Unknown(String::new());
    }
    if !text.starts_with('/') {
        return Command::Run {
            repo: None,
            task: text.to_owned(),
        };
    }
    let (head, rest) = match text.split_once(char::is_whitespace) {
        Some((h, r)) => (h, r.trim()),
        None => (text, ""),
    };
    // "/status@shiioo_bot" is how Telegram disambiguates bots in groups.
    let name = head
        .trim_start_matches('/')
        .split('@')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    match (name.as_str(), rest) {
        ("run", "") => Command::Unknown(text.to_owned()),
        ("run", rest) => parse_run(rest, known_repos),
        ("status", _) => Command::Status,
        ("capacity", _) => Command::Capacity,
        ("log", "") | ("cancel", "") | ("approve", "") | ("deny", "") => {
            Command::Unknown(text.to_owned())
        }
        ("log", run) => Command::Log {
            run: run.to_owned(),
        },
        ("cancel", run) => Command::Cancel {
            run: run.to_owned(),
        },
        ("repos", _) => Command::Repos,
        ("help" | "start", _) => Command::Help,
        ("approve", id) => Command::Approve {
            approval: id.to_owned(),
        },
        ("deny", id) => Command::Deny {
            approval: id.to_owned(),
        },
        _ => Command::Unknown(text.to_owned()),
    }
}

fn parse_run(rest: &str, known_repos: &[String]) -> Command {
    let (first, task) = match rest.split_once(char::is_whitespace) {
        Some((f, t)) => (f, t.trim()),
        None => (rest, ""),
    };
    if known_repos.iter().any(|r| r == first) && !task.is_empty() {
        return Command::Run {
            repo: Some(first.to_owned()),
            task: task.to_owned(),
        };
    }
    Command::Run {
        repo: None,
        task: rest.to_owned(),
    }
}

/// Decodes a button press: `approve:<id>` or `deny:<id>`.
pub fn parse_callback(data: &str) -> Option<Command> {
    let (action, id) = data.split_once(':')?;
    if id.is_empty() {
        return None;
    }
    match action {
        "approve" => Some(Command::Approve {
            approval: id.to_owned(),
        }),
        "deny" => Some(Command::Deny {
            approval: id.to_owned(),
        }),
        _ => None,
    }
}

/// The Allow / Deny buttons for an approval request. Callback data is
/// limited to 64 bytes; a UUID fits.
pub fn approval_keyboard(approval_id: &str) -> InlineKeyboard {
    InlineKeyboard(vec![vec![
        InlineButton {
            text: "✅ Allow".into(),
            callback_data: format!("approve:{approval_id}"),
        },
        InlineButton {
            text: "⛔ Deny".into(),
            callback_data: format!("deny:{approval_id}"),
        },
    ]])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(repo: Option<&str>, task: &str) -> Command {
        Command::Run {
            repo: repo.map(str::to_owned),
            task: task.to_owned(),
        }
    }

    #[test]
    fn command_table() {
        let repos = vec!["shiioo".to_owned()];
        let cases: Vec<(&str, Command)> = vec![
            (
                "/run shiioo fix the test",
                run(Some("shiioo"), "fix the test"),
            ),
            ("/run fix the test", run(None, "fix the test")),
            ("/run shiioo", run(None, "shiioo")),
            ("fix it", run(None, "fix it")),
            ("  fix it  ", run(None, "fix it")),
            ("/status", Command::Status),
            ("/status@shiioo_bot", Command::Status),
            ("/STATUS", Command::Status),
            ("/log abc", Command::Log { run: "abc".into() }),
            ("/cancel abc", Command::Cancel { run: "abc".into() }),
            ("/repos", Command::Repos),
            ("/help", Command::Help),
            ("/start", Command::Help),
            (
                "/approve x",
                Command::Approve {
                    approval: "x".into(),
                },
            ),
            (
                "/deny x",
                Command::Deny {
                    approval: "x".into(),
                },
            ),
            ("/bogus", Command::Unknown("/bogus".into())),
            ("/run", Command::Unknown("/run".into())),
            ("/log", Command::Unknown("/log".into())),
            ("", Command::Unknown(String::new())),
        ];
        for (text, expected) in cases {
            assert_eq!(parse_command(text, &repos), expected, "{text:?}");
        }
    }

    #[test]
    fn callbacks() {
        assert_eq!(
            parse_callback("approve:abc"),
            Some(Command::Approve {
                approval: "abc".into()
            })
        );
        assert_eq!(
            parse_callback("deny:abc"),
            Some(Command::Deny {
                approval: "abc".into()
            })
        );
        assert_eq!(parse_callback("approve:"), None);
        assert_eq!(parse_callback("nope"), None);
        assert_eq!(parse_callback("other:abc"), None);
    }

    #[test]
    fn keyboard_has_allow_and_deny() {
        let kb = approval_keyboard("abc");
        assert_eq!(kb.0.len(), 1);
        assert_eq!(kb.0[0][0].callback_data, "approve:abc");
        assert_eq!(kb.0[0][1].callback_data, "deny:abc");
        assert!(kb.0[0].iter().all(|b| b.callback_data.len() <= 64));
    }
}
