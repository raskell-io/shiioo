//! Mail with send-approval. An agent (or the owner from a terminal) asks
//! the office to send a mail; the office treats it as a tool call named
//! `mail.send`, judges it by the rules (the default asks), waits for the
//! owner's tap if needed, sends through the configured sender and records
//! `MailSent` in the run's log. The agent never holds the mail account.

use crate::{ApiError, ApiState, PermissionInput};
use serde::{Deserialize, Serialize};
use serde_json::json;
use shiioo_core::{EventStore, Payload, RunId};
use std::future::Future;
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailRequest {
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    pub subject: String,
    pub body: String,
}

/// Delivers mail. Implemented by the binary over `shiioo-mail`.
pub trait MailSender: Send + Sync {
    /// Returns the message id.
    fn send(&self, mail: MailRequest) -> BoxFuture<'_, Result<String, String>>;
    fn sender_address(&self) -> String;
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MailOutcome {
    pub message_id: String,
    pub from: String,
}

impl<S: EventStore + 'static> ApiState<S> {
    /// Sends `mail` for `run` (through the rules and, if they ask, the
    /// owner), or directly when no run is named (the owner's own request).
    pub async fn send_mail(
        self: &std::sync::Arc<Self>,
        run: Option<RunId>,
        mail: MailRequest,
    ) -> Result<MailOutcome, ApiError> {
        let sender = self.options.mail.as_ref().ok_or_else(|| {
            ApiError::Conflict("this office has no mail account ([mail] in config.toml)".into())
        })?;
        if mail.to.is_empty() {
            return Err(ApiError::BadRequest("no recipient".into()));
        }
        if let Some(run_id) = run {
            let outcome = self
                .permission(
                    run_id,
                    PermissionInput {
                        tool_name: "mail.send".into(),
                        tool_input: json!({
                            "to": mail.to,
                            "cc": mail.cc,
                            "subject": mail.subject,
                            "description": format!("to {}: {}", mail.to.join(", "), mail.subject),
                        }),
                        tool_use_id: None,
                        session_id: None,
                    },
                )
                .await?;
            if outcome.decision != "allow" {
                return Err(ApiError::Forbidden(outcome.reason));
            }
        }
        let message_id = sender.send(mail.clone()).await.map_err(ApiError::Failed)?;
        if let Some(run_id) = run {
            self.store
                .append_external(
                    run_id,
                    vec![Payload::MailSent {
                        to: mail.to.clone(),
                        subject: mail.subject.clone(),
                        message_id: message_id.clone(),
                    }],
                )
                .await
                .ok();
        }
        Ok(MailOutcome {
            message_id,
            from: sender.sender_address(),
        })
    }
}
