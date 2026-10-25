//! Telegram Bot API over HTTP: `POST {base}/bot{token}/{method}` with a
//! JSON body; the reply is `{"ok":true,"result":…}` or
//! `{"ok":false,"error_code":…,"description":…}`.

use serde::Serialize;
use serde_json::{Value, json};
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://api.telegram.org";
/// Telegram caps a long poll at 50 seconds.
const MAX_POLL_SECONDS: u32 = 50;

#[derive(Debug, thiserror::Error)]
pub enum TelegramError {
    #[error("telegram request failed: {0}")]
    Http(String),
    #[error("telegram api error {code}: {description}")]
    Api { code: i64, description: String },
    #[error("unexpected telegram response: {0}")]
    Decode(String),
}

pub struct Bot {
    token: String,
    http: reqwest::Client,
    base_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BotInfo {
    pub id: i64,
    pub username: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub update_id: i64,
    pub kind: UpdateKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateKind {
    /// A text message in a chat.
    Message {
        chat_id: i64,
        from_id: Option<i64>,
        text: String,
    },
    /// The owner pressed an inline button. `text` is the text of the
    /// message the button belongs to, so it can be edited in place.
    Callback {
        id: String,
        chat_id: i64,
        message_id: i64,
        from_id: i64,
        data: String,
        text: String,
    },
    /// Anything else (photos, edits, joins). Ignored.
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InlineKeyboard(pub Vec<Vec<InlineButton>>);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct InlineButton {
    pub text: String,
    pub callback_data: String,
}

impl InlineKeyboard {
    fn markup(keyboard: Option<&InlineKeyboard>) -> Value {
        // An empty keyboard removes the buttons from an edited message.
        let rows = keyboard.map_or_else(Vec::new, |k| k.0.clone());
        json!({ "inline_keyboard": rows })
    }
}

impl Bot {
    pub fn new(token: impl Into<String>) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self {
            token: token.into(),
            http,
            base_url: DEFAULT_BASE_URL.into(),
        }
    }

    /// Points the client at another server, for tests.
    pub fn with_base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into().trim_end_matches('/').to_owned();
        self
    }

    pub async fn get_me(&self) -> Result<BotInfo, TelegramError> {
        let v = self.call("getMe", &json!({}), None).await?;
        Ok(BotInfo {
            id: v["id"]
                .as_i64()
                .ok_or_else(|| TelegramError::Decode("getMe without id".into()))?,
            username: v["username"].as_str().unwrap_or_default().to_owned(),
        })
    }

    /// Long poll for updates with `update_id >= offset`, waiting up to
    /// `timeout_s` (at most 50) for the first one. Pass the last seen
    /// `update_id + 1` as `offset` to acknowledge earlier updates.
    pub async fn get_updates(
        &self,
        offset: i64,
        timeout_s: u32,
    ) -> Result<Vec<Update>, TelegramError> {
        let timeout_s = timeout_s.min(MAX_POLL_SECONDS);
        let body = json!({
            "offset": offset,
            "timeout": timeout_s,
            "allowed_updates": ["message", "callback_query"],
        });
        let wait = Duration::from_secs(u64::from(timeout_s) + 10);
        let v = self.call("getUpdates", &body, Some(wait)).await?;
        let updates = v
            .as_array()
            .ok_or_else(|| TelegramError::Decode("getUpdates result is not an array".into()))?;
        updates.iter().map(parse_update).collect()
    }

    /// Sends plain text (no parse mode, so arbitrary tool output is safe).
    /// Returns the new message's id.
    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        keyboard: Option<&InlineKeyboard>,
    ) -> Result<i64, TelegramError> {
        let mut body = json!({ "chat_id": chat_id, "text": text });
        if keyboard.is_some() {
            body["reply_markup"] = InlineKeyboard::markup(keyboard);
        }
        let v = self.call("sendMessage", &body, None).await?;
        v["message_id"]
            .as_i64()
            .ok_or_else(|| TelegramError::Decode("sendMessage without message_id".into()))
    }

    /// Replaces a message's text and keyboard. `None` removes the keyboard.
    pub async fn edit_message_text(
        &self,
        chat_id: i64,
        message_id: i64,
        text: &str,
        keyboard: Option<&InlineKeyboard>,
    ) -> Result<(), TelegramError> {
        let body = json!({
            "chat_id": chat_id,
            "message_id": message_id,
            "text": text,
            "reply_markup": InlineKeyboard::markup(keyboard),
        });
        self.call("editMessageText", &body, None).await?;
        Ok(())
    }

    /// Acknowledges a button press; `text` shows as a short toast.
    pub async fn answer_callback_query(
        &self,
        callback_query_id: &str,
        text: Option<&str>,
    ) -> Result<(), TelegramError> {
        let mut body = json!({ "callback_query_id": callback_query_id });
        if let Some(text) = text {
            body["text"] = json!(text);
        }
        self.call("answerCallbackQuery", &body, None).await?;
        Ok(())
    }

    async fn call(
        &self,
        method: &str,
        body: &Value,
        timeout: Option<Duration>,
    ) -> Result<Value, TelegramError> {
        let url = format!("{}/bot{}/{method}", self.base_url, self.token);
        let mut req = self.http.post(url).json(body);
        if let Some(timeout) = timeout {
            req = req.timeout(timeout);
        }
        let text = req
            .send()
            .await
            .map_err(|e| TelegramError::Http(e.to_string()))?
            .text()
            .await
            .map_err(|e| TelegramError::Http(e.to_string()))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| {
            TelegramError::Decode(format!(
                "{e}: {}",
                text.chars().take(200).collect::<String>()
            ))
        })?;
        if v["ok"] == true {
            return Ok(v["result"].clone());
        }
        Err(TelegramError::Api {
            code: v["error_code"].as_i64().unwrap_or(0),
            description: v["description"]
                .as_str()
                .unwrap_or("unknown error")
                .to_owned(),
        })
    }
}

fn parse_update(v: &Value) -> Result<Update, TelegramError> {
    let update_id = v["update_id"]
        .as_i64()
        .ok_or_else(|| TelegramError::Decode("update without update_id".into()))?;
    let kind = if let Some(text) = v["message"]["text"].as_str() {
        UpdateKind::Message {
            chat_id: v["message"]["chat"]["id"].as_i64().unwrap_or(0),
            from_id: v["message"]["from"]["id"].as_i64(),
            text: text.to_owned(),
        }
    } else if let Some(cb) = v.get("callback_query") {
        UpdateKind::Callback {
            id: cb["id"].as_str().unwrap_or_default().to_owned(),
            chat_id: cb["message"]["chat"]["id"].as_i64().unwrap_or(0),
            message_id: cb["message"]["message_id"].as_i64().unwrap_or(0),
            from_id: cb["from"]["id"].as_i64().unwrap_or(0),
            data: cb["data"].as_str().unwrap_or_default().to_owned(),
            text: cb["message"]["text"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        }
    } else {
        UpdateKind::Other
    };
    Ok(Update { update_id, kind })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_message_and_callback_updates() {
        let msg = json!({
            "update_id": 7,
            "message": { "message_id": 3, "from": { "id": 42 }, "chat": { "id": 42 }, "text": "/status" }
        });
        let Update { update_id, kind } = parse_update(&msg).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(update_id, 7);
        assert_eq!(
            kind,
            UpdateKind::Message {
                chat_id: 42,
                from_id: Some(42),
                text: "/status".into()
            }
        );

        let cb = json!({
            "update_id": 8,
            "callback_query": {
                "id": "q1", "from": { "id": 42 }, "data": "approve:abc",
                "message": { "message_id": 9, "chat": { "id": 42 }, "text": "Allow?" }
            }
        });
        let Update { kind, .. } = parse_update(&cb).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            kind,
            UpdateKind::Callback {
                id: "q1".into(),
                chat_id: 42,
                message_id: 9,
                from_id: 42,
                data: "approve:abc".into(),
                text: "Allow?".into(),
            }
        );

        let photo = json!({ "update_id": 9, "message": { "chat": { "id": 1 }, "photo": [] } });
        assert_eq!(
            parse_update(&photo).unwrap_or_else(|e| panic!("{e}")).kind,
            UpdateKind::Other
        );
    }
}
