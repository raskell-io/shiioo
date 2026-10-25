//! What the owner can say to the bot.

use crate::api::{InlineButton, InlineKeyboard};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// `/run [@role] [<repo>] <task…>`, or plain text (optionally starting
    /// with `@role`).
    Run {
        repo: Option<String>,
        role: Option<String>,
        /// `on:<machine>`: an enrolled node's name or `label=value`.
        machine: Option<String>,
        task: String,
    },
    /// `/files <run>`: the files a run produced.
    Files {
        run: String,
    },
    /// `/get <run> <path>`: send one of them.
    Get {
        run: String,
        path: String,
    },
    /// `/machines`: the enrolled nodes and whether they are online.
    Machines,
    /// `/enroll <name> [label=value…]`: a one-time token for `shiioo node enroll`.
    Enroll {
        name: String,
        labels: Vec<String>,
    },
    /// `/status`
    Status,
    /// `/roles`: the configured roles.
    Roles,
    /// `/routines`: the configured routines and when they fire next.
    Routines,
    /// `/routine <name>`: fire a routine now.
    Routine {
        name: String,
    },
    /// `/report`: the morning report, now.
    Report,
    /// `/plan [<repo>] <goal…>`: ask the chief of staff for a plan.
    Plan {
        repo: Option<String>,
        goal: String,
    },
    /// `/plan approve <id>`
    PlanApprove {
        plan: String,
    },
    /// `/plan show <id>`
    PlanShow {
        plan: String,
    },
    /// `/plans`
    Plans,
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
        return parse_run(text, known_repos);
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
        ("roles", _) => Command::Roles,
        ("machines", _) => Command::Machines,
        ("files", "") | ("get", "") => Command::Unknown(text.to_owned()),
        ("files", run) => Command::Files {
            run: run.to_owned(),
        },
        ("get", rest) => match rest.split_once(char::is_whitespace) {
            Some((run, path)) if !path.trim().is_empty() => Command::Get {
                run: run.to_owned(),
                path: path.trim().to_owned(),
            },
            _ => Command::Unknown(text.to_owned()),
        },
        ("enroll", "") => Command::Unknown(text.to_owned()),
        ("enroll", rest) => {
            let mut words = rest.split_whitespace();
            Command::Enroll {
                name: words.next().unwrap_or_default().to_owned(),
                labels: words.map(str::to_owned).collect(),
            }
        }
        ("routines", _) => Command::Routines,
        ("routine", "") => Command::Unknown(text.to_owned()),
        ("routine", name) => Command::Routine {
            name: name.to_owned(),
        },
        ("report", _) => Command::Report,
        ("plans", _) => Command::Plans,
        ("plan", "") => Command::Unknown(text.to_owned()),
        ("plan", rest) => parse_plan(rest, known_repos),
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

/// `[@role] [on:<machine>] [<repo>] <task>`, in any order of the first
/// two: a leading `@word` names a role, `on:word` a machine; a next word
/// that is a known repo label names the repo; the rest is the task.
fn parse_run(rest: &str, known_repos: &[String]) -> Command {
    let mut role = None;
    let mut machine = None;
    let mut rest = rest.trim();
    for _ in 0..2 {
        let (r, after) = take_role(rest);
        if r.is_some() {
            role = r;
            rest = after;
            continue;
        }
        let (m, after) = take_machine(rest);
        if m.is_some() {
            machine = m;
            rest = after;
            continue;
        }
        break;
    }
    let (repo, task) = take_repo(rest, known_repos);
    Command::Run {
        repo,
        role,
        machine,
        task: task.to_owned(),
    }
}

fn take_machine(text: &str) -> (Option<String>, &str) {
    let text = text.trim();
    if let Some(stripped) = text.strip_prefix("on:") {
        let (machine, rest) = match stripped.split_once(char::is_whitespace) {
            Some((m, rest)) => (m, rest.trim()),
            None => (stripped, ""),
        };
        if !machine.is_empty() {
            return (Some(machine.to_owned()), rest);
        }
    }
    (None, text)
}

/// `approve <id>`, `show <id>`, or `[<repo>] <goal>`.
fn parse_plan(rest: &str, known_repos: &[String]) -> Command {
    let (first, after) = match rest.split_once(char::is_whitespace) {
        Some((f, a)) => (f, a.trim()),
        None => (rest, ""),
    };
    match (first.to_ascii_lowercase().as_str(), after) {
        ("approve", "") | ("show", "") => Command::Unknown(format!("/plan {rest}")),
        ("approve", id) => Command::PlanApprove {
            plan: id.to_owned(),
        },
        ("show", id) => Command::PlanShow {
            plan: id.to_owned(),
        },
        _ => {
            let (repo, goal) = take_repo(rest, known_repos);
            Command::Plan {
                repo,
                goal: goal.to_owned(),
            }
        }
    }
}

fn take_role(text: &str) -> (Option<String>, &str) {
    let text = text.trim();
    if let Some(stripped) = text.strip_prefix('@') {
        let (role, rest) = match stripped.split_once(char::is_whitespace) {
            Some((r, rest)) => (r, rest.trim()),
            None => (stripped, ""),
        };
        if !role.is_empty() {
            return (Some(role.to_owned()), rest);
        }
    }
    (None, text)
}

fn take_repo<'a>(text: &'a str, known_repos: &[String]) -> (Option<String>, &'a str) {
    let (first, rest) = match text.split_once(char::is_whitespace) {
        Some((f, r)) => (f, r.trim()),
        None => (text, ""),
    };
    if known_repos.iter().any(|r| r == first) && !rest.is_empty() {
        return (Some(first.to_owned()), rest);
    }
    (None, text)
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
            role: None,
            machine: None,
            task: task.to_owned(),
        }
    }

    fn run_as(role: &str, repo: Option<&str>, task: &str) -> Command {
        Command::Run {
            repo: repo.map(str::to_owned),
            role: Some(role.to_owned()),
            machine: None,
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
            (
                "/run @reviewer shiioo check PR 12",
                run_as("reviewer", Some("shiioo"), "check PR 12"),
            ),
            ("/run @reviewer check PR 12", run_as("reviewer", None, "check PR 12")),
            ("@reviewer check PR 12", run_as("reviewer", None, "check PR 12")),
            ("@ check", run(None, "@ check")),
            (
                "/run on:spark shiioo train the model",
                Command::Run {
                    repo: Some("shiioo".into()),
                    role: None,
                    machine: Some("spark".into()),
                    task: "train the model".into(),
                },
            ),
            (
                "@trainer on:gpu=true run it",
                Command::Run {
                    repo: None,
                    role: Some("trainer".into()),
                    machine: Some("gpu=true".into()),
                    task: "run it".into(),
                },
            ),
            ("/machines", Command::Machines),
            ("/files abcd", Command::Files { run: "abcd".into() }),
            (
                "/get abcd out/cover.png",
                Command::Get {
                    run: "abcd".into(),
                    path: "out/cover.png".into(),
                },
            ),
            ("/get abcd", Command::Unknown("/get abcd".into())),
            (
                "/enroll spark gpu=true os=linux",
                Command::Enroll {
                    name: "spark".into(),
                    labels: vec!["gpu=true".into(), "os=linux".into()],
                },
            ),
            ("/enroll", Command::Unknown("/enroll".into())),
            ("/roles", Command::Roles),
            ("/routines", Command::Routines),
            ("/routine nightly", Command::Routine { name: "nightly".into() }),
            ("/routine", Command::Unknown("/routine".into())),
            ("/report", Command::Report),
            ("/plans", Command::Plans),
            (
                "/plan shiioo ship v0.3 this week",
                Command::Plan {
                    repo: Some("shiioo".into()),
                    goal: "ship v0.3 this week".into(),
                },
            ),
            (
                "/plan ship v0.3",
                Command::Plan {
                    repo: None,
                    goal: "ship v0.3".into(),
                },
            ),
            ("/plan approve abcd", Command::PlanApprove { plan: "abcd".into() }),
            ("/plan show abcd", Command::PlanShow { plan: "abcd".into() }),
            ("/plan approve", Command::Unknown("/plan approve".into())),
            ("/plan", Command::Unknown("/plan".into())),
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
