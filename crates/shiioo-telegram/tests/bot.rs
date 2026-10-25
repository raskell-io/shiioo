//! `run_bot` and `Notifier` against a fake Telegram server.

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{Value, json};
use shiioo_telegram::{Bot, BoxFuture, Notifier, Office, run_bot};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const OWNER: i64 = 42;
const STRANGER: i64 = 99;

#[derive(Default)]
struct Fake {
    polls: AtomicUsize,
    sent: Mutex<Vec<(String, Value)>>,
}

impl Fake {
    fn calls(&self, method: &str) -> Vec<Value> {
        lock(&self.sent)
            .iter()
            .filter(|(m, _)| m == method)
            .map(|(_, b)| b.clone())
            .collect()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

async fn telegram(
    State(fake): State<Arc<Fake>>,
    Path((_token, method)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Json<Value> {
    let result = match method.as_str() {
        "getUpdates" => match fake.polls.fetch_add(1, Ordering::SeqCst) {
            0 => json!([
                { "update_id": 1, "message": { "message_id": 10, "from": { "id": OWNER }, "chat": { "id": OWNER }, "text": "/status" } },
                { "update_id": 2, "message": { "message_id": 11, "from": { "id": STRANGER }, "chat": { "id": STRANGER }, "text": "/status" } },
            ]),
            1 => json!([
                { "update_id": 3, "callback_query": {
                    "id": "cq1", "from": { "id": OWNER }, "data": "approve:abc",
                    "message": { "message_id": 12, "chat": { "id": OWNER }, "text": "Allow git push?" }
                } },
            ]),
            _ => {
                tokio::time::sleep(Duration::from_millis(200)).await;
                json!([])
            }
        },
        "sendMessage" => {
            let n = lock(&fake.sent).len() as i64;
            lock(&fake.sent).push((method.clone(), body));
            return Json(json!({ "ok": true, "result": { "message_id": 100 + n } }));
        }
        "getMe" => json!({ "id": 1, "is_bot": true, "username": "shiioo_bot" }),
        _ => {
            lock(&fake.sent).push((method.clone(), body));
            json!(true)
        }
    };
    Json(json!({ "ok": true, "result": result }))
}

async fn serve() -> (Arc<Fake>, Bot) {
    let fake = Arc::new(Fake::default());
    let app = Router::new()
        .route("/bot{token}/{method}", post(telegram))
        .with_state(Arc::clone(&fake));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|e| panic!("bind: {e}"));
    let addr = listener.local_addr().unwrap_or_else(|e| panic!("{e}"));
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    (
        fake,
        Bot::new("test-token").with_base_url(format!("http://{addr}")),
    )
}

#[derive(Default)]
struct RecordingOffice {
    calls: Mutex<Vec<String>>,
}

impl Office for RecordingOffice {
    fn repos(&self) -> Vec<String> {
        vec!["shiioo".into()]
    }

    fn start_run(
        &self,
        repo: Option<String>,
        task: String,
    ) -> BoxFuture<'_, Result<String, String>> {
        lock(&self.calls).push(format!("start_run {repo:?} {task}"));
        Box::pin(async { Ok("started".into()) })
    }

    fn status(&self) -> BoxFuture<'_, String> {
        lock(&self.calls).push("status".into());
        Box::pin(async { "all quiet".into() })
    }

    fn log(&self, run: String) -> BoxFuture<'_, String> {
        lock(&self.calls).push(format!("log {run}"));
        Box::pin(async { "log".into() })
    }

    fn cancel(&self, run: String) -> BoxFuture<'_, Result<String, String>> {
        lock(&self.calls).push(format!("cancel {run}"));
        Box::pin(async { Ok("cancelled".into()) })
    }

    fn decide(&self, approval: String, allow: bool) -> BoxFuture<'_, Result<String, String>> {
        lock(&self.calls).push(format!("decide {approval} {allow}"));
        Box::pin(async { Ok("recorded".into()) })
    }
}

#[tokio::test]
async fn dispatches_owner_commands_and_button_presses() {
    let (fake, bot) = serve().await;
    let office = Arc::new(RecordingOffice::default());
    let bot = Arc::new(bot);

    assert_eq!(
        bot.get_me()
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .username,
        "shiioo_bot"
    );

    let ran = tokio::time::timeout(
        Duration::from_secs(5),
        run_bot(
            Arc::clone(&bot),
            OWNER,
            Arc::clone(&office) as Arc<dyn Office>,
        ),
    )
    .await;
    assert!(ran.is_err(), "run_bot must keep polling until dropped");

    let calls = lock(&office.calls).clone();
    assert_eq!(
        calls.iter().filter(|c| *c == "status").count(),
        1,
        "the stranger's /status is ignored: {calls:?}"
    );
    assert!(calls.contains(&"decide abc true".to_string()), "{calls:?}");

    let sent = fake.calls("sendMessage");
    assert!(
        sent.iter()
            .any(|b| b["chat_id"] == OWNER && b["text"] == "all quiet"),
        "{sent:?}"
    );
    assert!(sent.iter().all(|b| b["chat_id"] == OWNER));

    let answered = fake.calls("answerCallbackQuery");
    assert_eq!(answered.len(), 1);
    assert_eq!(answered[0]["callback_query_id"], "cq1");

    let edited = fake.calls("editMessageText");
    assert_eq!(edited.len(), 1);
    assert_eq!(edited[0]["message_id"], 12);
    let text = edited[0]["text"].as_str().unwrap_or_default();
    assert!(text.starts_with("Allow git push?"), "{text}");
    assert!(text.contains("allowed by you"), "{text}");
    assert_eq!(edited[0]["reply_markup"]["inline_keyboard"], json!([]));

    assert!(fake.polls.load(Ordering::SeqCst) >= 3);
}

#[tokio::test]
async fn notifier_sends_approval_requests_with_buttons() {
    let (fake, bot) = serve().await;
    let notifier = Notifier::new(Arc::new(bot), OWNER);

    let id = notifier
        .ask_approval("abc", "Allow git push to main?")
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(id, 100);

    let sent = fake.calls("sendMessage");
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["text"], "Allow git push to main?");
    let row = &sent[0]["reply_markup"]["inline_keyboard"][0];
    assert_eq!(row[0]["callback_data"], "approve:abc");
    assert_eq!(row[1]["callback_data"], "deny:abc");

    notifier
        .resolve_approval(id, "Allow git push to main?", "⏰ expired")
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let edited = fake.calls("editMessageText");
    assert_eq!(edited[0]["message_id"], 100);
    assert!(
        edited[0]["text"]
            .as_str()
            .unwrap_or_default()
            .ends_with("⏰ expired")
    );

    notifier
        .notify(&"line\n".repeat(1000))
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        fake.calls("sendMessage").len() > 2,
        "long notifications are split"
    );
}
