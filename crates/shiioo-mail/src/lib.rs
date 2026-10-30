//! Outgoing mail (`docs/design/tools-and-machines.md` §7: "work goes out as
//! mail drafts"). One sender, two transports: SMTP with the owner's
//! account, and a directory of `.eml` files, which tests and dry runs use.
//! Who may send what is the office's business (rules and approvals in
//! `shiioo-api::mail`); this crate only delivers.

use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Security {
    /// STARTTLS on the submission port (587). The default.
    StartTls,
    /// Implicit TLS (465).
    Tls,
    /// Plain text, for a relay on localhost only.
    None,
}

impl Security {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "" | "starttls" => Some(Security::StartTls),
            "tls" | "ssl" => Some(Security::Tls),
            "none" | "plain" => Some(Security::None),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub security: Security,
    /// `Name <address>` or `address`.
    pub from: String,
    pub user: String,
    pub password: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mail {
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: String,
    /// Plain text. Sent as text/plain; a Markdown body stays Markdown.
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MailError {
    #[error("bad address {0:?}: {1}")]
    Address(String, String),
    #[error("building the message: {0}")]
    Build(#[from] lettre::error::Error),
    #[error("smtp: {0}")]
    Smtp(#[from] lettre::transport::smtp::Error),
    #[error("writing the message file: {0}")]
    File(#[from] lettre::transport::file::Error),
    #[error("{0}")]
    Config(String),
}

pub enum Mailer {
    Smtp {
        transport: AsyncSmtpTransport<Tokio1Executor>,
        from: Mailbox,
    },
    /// Writes `<message id>.eml` into a directory instead of sending.
    File { dir: PathBuf, from: Mailbox },
}

impl Mailer {
    pub fn smtp(config: &SmtpConfig) -> Result<Self, MailError> {
        let from = mailbox(&config.from)?;
        let builder = match config.security {
            Security::StartTls => {
                AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)?
            }
            Security::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)?,
            Security::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host),
        }
        .port(config.port);
        let builder = if config.user.is_empty() && config.password.is_empty() {
            builder
        } else {
            let user = if config.user.is_empty() {
                from.email.to_string()
            } else {
                config.user.clone()
            };
            builder.credentials(Credentials::new(user, config.password.clone()))
        };
        Ok(Mailer::Smtp {
            transport: builder.build(),
            from,
        })
    }

    pub fn file(dir: impl Into<PathBuf>, from: &str) -> Result<Self, MailError> {
        Ok(Mailer::File {
            dir: dir.into(),
            from: mailbox(from)?,
        })
    }

    pub fn from_address(&self) -> String {
        match self {
            Mailer::Smtp { from, .. } | Mailer::File { from, .. } => from.email.to_string(),
        }
    }

    /// Sends, returning the message id (without the angle brackets).
    pub async fn send(&self, mail: &Mail) -> Result<String, MailError> {
        let from = match self {
            Mailer::Smtp { from, .. } | Mailer::File { from, .. } => from.clone(),
        };
        if mail.to.is_empty() {
            return Err(MailError::Config("no recipient".into()));
        }
        let id = format!("{}@shiioo", uuid::Uuid::new_v4().simple());
        let mut message = Message::builder()
            .from(from)
            .subject(&mail.subject)
            .message_id(Some(format!("<{id}>")));
        for to in &mail.to {
            message = message.to(mailbox(to)?);
        }
        for cc in &mail.cc {
            message = message.cc(mailbox(cc)?);
        }
        let message = message.multipart(MultiPart::alternative_plain_html(
            mail.body.clone(),
            html_escape(&mail.body),
        ))?;
        match self {
            Mailer::Smtp { transport, .. } => {
                transport.send(message).await?;
            }
            Mailer::File { dir, .. } => {
                std::fs::create_dir_all(dir)
                    .map_err(|e| MailError::Config(format!("{}: {e}", dir.display())))?;
                let transport = lettre::AsyncFileTransport::<Tokio1Executor>::new(dir);
                transport.send(message).await?;
            }
        }
        Ok(id)
    }
}

fn mailbox(s: &str) -> Result<Mailbox, MailError> {
    s.trim()
        .parse::<Mailbox>()
        .map_err(|e| MailError::Address(s.to_owned(), e.to_string()))
}

/// The plain body as minimal HTML: escaped, paragraphs on blank lines.
fn html_escape(body: &str) -> String {
    let escaped = body
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let paragraphs: Vec<String> = escaped
        .split("\n\n")
        .map(|p| format!("<p>{}</p>", p.trim().replace('\n', "<br>")))
        .collect();
    format!("<html><body>{}</body></html>", paragraphs.join("\n"))
}

/// The `.eml` files in a directory, newest last: what a `File` mailer sent.
pub fn sent_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "eml"))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_a_message_file_with_everything_in_it() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let mailer = Mailer::file(dir.path(), "Shiioo <office@example.com>")
            .unwrap_or_else(|e| panic!("{e}"));
        let id = mailer
            .send(&Mail {
                to: vec!["Ada <ada@example.com>".into()],
                cc: vec!["bob@example.com".into()],
                subject: "Weekly report".into(),
                body: "Hello,\n\nall good & well.\n".into(),
            })
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!id.is_empty());
        let files = sent_files(dir.path());
        assert_eq!(files.len(), 1);
        let text = std::fs::read_to_string(&files[0]).unwrap_or_default();
        for needle in [
            "From: Shiioo <office@example.com>",
            "To: Ada <ada@example.com>",
            "Cc: bob@example.com",
            "Subject: Weekly report",
            "all good & well.",
            "all good &amp; well.",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in {text}");
        }
        assert_eq!(mailer.from_address(), "office@example.com");
        assert!(
            mailer
                .send(&Mail {
                    to: vec![],
                    cc: vec![],
                    subject: "x".into(),
                    body: String::new()
                })
                .await
                .is_err()
        );
        assert!(
            mailer
                .send(&Mail {
                    to: vec!["not an address".into()],
                    cc: vec![],
                    subject: "x".into(),
                    body: String::new()
                })
                .await
                .is_err()
        );
    }

    #[test]
    fn security_names() {
        assert_eq!(Security::parse(""), Some(Security::StartTls));
        assert_eq!(Security::parse("TLS"), Some(Security::Tls));
        assert_eq!(Security::parse("none"), Some(Security::None));
        assert_eq!(Security::parse("quantum"), None);
    }
}
