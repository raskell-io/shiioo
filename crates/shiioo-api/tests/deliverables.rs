//! Files a run produced, and mail with send-approval, over HTTP.

use serde_json::{Value, json};
use shiioo_api::{
    ApiOptions, ApiState, Artifact, BoxFuture, MailRequest, MailSender, Notifications, router,
};
use shiioo_core::{RunId, SystemClock};
use shiioo_runtime::{AgentSpec, LocalMachine, RunStatus, Toolbox, UnconfiguredProvider, Worker};
use shiioo_store::{MemoryStore, Notifying};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TOKEN: &str = "test-token";
const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../shiioo-runtime/tests/fixtures/claude-code-read.jsonl"
);

#[derive(Default)]
struct Recorder {
    sent: Mutex<Vec<MailRequest>>,
    ended: Mutex<Vec<(RunId, Vec<Artifact>)>>,
}

impl MailSender for Recorder {
    fn send(&self, mail: MailRequest) -> shiioo_api::mail::BoxFuture<'_, Result<String, String>> {
        lock(&self.sent).push(mail);
        Box::pin(async { Ok("msg-1@shiioo".into()) })
    }
    fn sender_address(&self) -> String {
        "office@example.com".into()
    }
}

impl Notifications for Recorder {
    fn approval_requested(&self, _: RunId, _: String, _: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn approval_resolved(&self, _: String, _: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn run_ended(
        &self,
        run: RunId,
        _: String,
        _: RunStatus,
        media: Vec<Artifact>,
    ) -> BoxFuture<'_, ()> {
        lock(&self.ended).push((run, media));
        Box::pin(async {})
    }
    fn message(&self, _: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

struct Office {
    base: String,
    recorder: Arc<Recorder>,
    repo: PathBuf,
    dir: tempfile::TempDir,
}

async fn git(dir: &Path, args: &[&str]) {
    let status = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(status.success(), "git {args:?}");
}

async fn serve() -> Office {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap_or_else(|e| panic!("{e}"));
    git(&repo, &["init", "-q", "-b", "main"]).await;
    git(&repo, &["config", "user.email", "t@example.com"]).await;
    git(&repo, &["config", "user.name", "t"]).await;
    std::fs::write(repo.join("README.md"), "hello\n").unwrap_or_else(|e| panic!("{e}"));
    git(&repo, &["add", "."]).await;
    git(&repo, &["commit", "-q", "-m", "init"]).await;

    let store = Arc::new(Notifying::new(MemoryStore::new(SystemClock)));
    let worktrees = dir.path().join("worktrees");
    let machine = Arc::new(
        LocalMachine::in_memory(dir.path().join("ws"), Toolbox::builtin())
            .with_worktrees(&worktrees),
    );
    let factory_store = Arc::clone(&store);
    let recorder = Arc::new(Recorder::default());
    let state = ApiState::with_options(
        store,
        Arc::new(move || {
            Worker::new(
                Arc::clone(&factory_store),
                Arc::new(UnconfiguredProvider),
                Arc::clone(&machine),
                "deliverables-test",
            )
        }),
        TOKEN,
        AgentSpec {
            model: String::new(),
            system: String::new(),
            tools: vec![],
            max_turns: 5,
        },
        ApiOptions {
            worktrees: Some(worktrees),
            mail: Some(Arc::clone(&recorder) as Arc<dyn MailSender>),
            notifications: Some(Arc::clone(&recorder) as Arc<dyn Notifications>),
            ..ApiOptions::default()
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|e| panic!("bind: {e}"));
    let addr = listener.local_addr().unwrap_or_else(|e| panic!("{e}"));
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.ok();
    });
    Office {
        base: format!("http://{addr}"),
        recorder,
        repo,
        dir,
    }
}

/// A fake harness: prints the fixture's init line, runs `body` in the
/// worktree, then a result line.
fn fake_harness(dir: &Path, body: &str) -> PathBuf {
    let script = dir.join("claude");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nhead -n 1 '{FIXTURE}'\n{body}\n\
             printf '%s\\n' '{{\"type\":\"result\",\"subtype\":\"success\",\"is_error\":false,\
             \"result\":\"made things\",\"session_id\":\"s1\",\"total_cost_usd\":0,\"usage\":{{}},\
             \"permission_denials\":[]}}'\n"
        ),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    script
}

async fn start_harness_run(
    http: &reqwest::Client,
    base: &str,
    repo: &Path,
    script: &Path,
) -> String {
    let created: Value = http
        .post(format!("{base}/v1/runs"))
        .bearer_auth(TOKEN)
        .json(&json!({
            "task": "make things",
            "runtime": { "kind": "harness", "harness": "claude-code", "repo": repo, "binary": "/bin/sh", "binary_args": [script] }
        }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    created["id"].as_str().unwrap_or_default().to_owned()
}

async fn wait_state(http: &reqwest::Client, base: &str, run: &str, want: &str) -> Value {
    let mut view = Value::Null;
    for _ in 0..200 {
        view = http
            .get(format!("{base}/v1/runs/{run}"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .json()
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        if view["status"]["state"] == want {
            return view;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("run {run} never reached {want}: {view}");
}

#[tokio::test]
async fn a_runs_files_are_listed_downloadable_and_sent_along() {
    let office = serve().await;
    let http = reqwest::Client::new();
    // The harness commits an image and leaves an untracked note.
    let script = fake_harness(
        office.dir.path(),
        "printf 'PNG' > cover.png && mkdir -p notes && printf 'draft' > notes/todo.txt && \
         git add cover.png && git -c user.email=a@b -c user.name=a commit -q -m 'cover'",
    );
    let run = start_harness_run(&http, &office.base, &office.repo, &script).await;
    wait_state(&http, &office.base, &run, "completed").await;

    let artifacts: Vec<Value> = http
        .get(format!("{}/v1/runs/{run}/artifacts", office.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let by_path = |p: &str| {
        artifacts
            .iter()
            .find(|a| a["path"] == p)
            .cloned()
            .unwrap_or(Value::Null)
    };
    assert_eq!(by_path("cover.png")["status"], "added", "{artifacts:?}");
    assert_eq!(by_path("cover.png")["kind"], "image");
    assert_eq!(by_path("cover.png")["size"], 3);
    assert_eq!(by_path("notes/todo.txt")["status"], "untracked");
    assert_eq!(by_path("notes/todo.txt")["kind"], "text");

    let resp = http
        .get(format!("{}/v1/runs/{run}/artifacts/cover.png", office.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(resp.status(), 200);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("image/png")
    );
    assert_eq!(resp.bytes().await.unwrap_or_default().as_ref(), b"PNG");
    let resp = http
        .get(format!(
            "{}/v1/runs/{run}/artifacts/notes/todo.txt",
            office.base
        ))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(resp.text().await.unwrap_or_default(), "draft");
    let status = http
        .get(format!("{}/v1/runs/{run}/artifacts/../secret", office.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .status();
    assert!(status == 400 || status == 404, "{status}");

    // The ending notification carried the image, not the note.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let ended = lock(&office.recorder.ended).clone();
    assert_eq!(ended.len(), 1, "{ended:?}");
    let media: Vec<&str> = ended[0].1.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(media, vec!["cover.png"]);
}

#[tokio::test]
async fn mail_goes_through_the_rules_and_the_owner() {
    let office = serve().await;
    let http = reqwest::Client::new();

    // The owner's own request needs no run and no approval.
    let sent: Value = http
        .post(format!("{}/v1/mail/send", office.base))
        .bearer_auth(TOKEN)
        .json(&json!({ "to": ["ada@example.com"], "subject": "Hi", "body": "Hello." }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(sent["message_id"], "msg-1@shiioo");
    assert_eq!(sent["from"], "office@example.com");
    assert_eq!(lock(&office.recorder.sent).len(), 1);

    // A run's request waits for the owner (the default rule asks).
    let script = fake_harness(office.dir.path(), "sleep 20");
    let run = start_harness_run(&http, &office.base, &office.repo, &script).await;
    wait_state(&http, &office.base, &run, "open").await;
    let base = office.base.clone();
    let run_for_mail = run.clone();
    let mail = tokio::spawn(async move {
        reqwest::Client::new()
            .post(format!("{base}/v1/mail/send"))
            .bearer_auth(TOKEN)
            .json(&json!({ "run": run_for_mail, "to": ["ceo@example.com"], "subject": "Report", "body": "Done." }))
            .send()
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    });
    let view = wait_state(&http, &office.base, &run, "waiting_approval").await;
    let summary = view["status"]["summary"].as_str().unwrap_or_default();
    assert!(
        summary.starts_with("mail.send: to ceo@example.com: Report"),
        "{summary}"
    );
    let approval = view["status"]["approval_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    // Deny first: nothing is sent.
    http.post(format!("{}/v1/approvals/{approval}", office.base))
        .bearer_auth(TOKEN)
        .json(&json!({ "decision": "deny" }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let resp = mail.await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(resp.status(), 403);
    assert_eq!(lock(&office.recorder.sent).len(), 1);

    // Allowed: sent, and on the record.
    let base = office.base.clone();
    let run_for_mail = run.clone();
    let mail = tokio::spawn(async move {
        reqwest::Client::new()
            .post(format!("{base}/v1/mail/send"))
            .bearer_auth(TOKEN)
            .json(&json!({ "run": run_for_mail, "to": ["ceo@example.com"], "subject": "Report", "body": "Done." }))
            .send()
            .await
            .unwrap_or_else(|e| panic!("{e}"))
    });
    let view = wait_state(&http, &office.base, &run, "waiting_approval").await;
    let approval = view["status"]["approval_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    http.post(format!("{}/v1/approvals/{approval}", office.base))
        .bearer_auth(TOKEN)
        .json(&json!({ "decision": "allow" }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let resp = mail.await.unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(resp.status(), 200);
    let sent = lock(&office.recorder.sent).clone();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].to, vec!["ceo@example.com"]);
    let events: Vec<Value> = http
        .get(format!("{}/v1/runs/{run}/events?from=0", office.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let mail_sent = events
        .iter()
        .find(|e| e["payload"]["type"] == "mail_sent")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(mail_sent["payload"]["subject"], "Report", "{events:?}");
    assert_eq!(mail_sent["payload"]["message_id"], "msg-1@shiioo");
    http.post(format!("{}/v1/runs/{run}/cancel", office.base))
        .bearer_auth(TOKEN)
        .json(&json!({}))
        .send()
        .await
        .ok();
}
