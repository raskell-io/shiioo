//! The dashboard files and the diff endpoint.

use serde_json::{Value, json};
use shiioo_api::{ApiState, router};
use shiioo_core::SystemClock;
use shiioo_runtime::{AgentSpec, LocalMachine, ScriptedProvider, Toolbox, Worker, branch_for};
use shiioo_store::{MemoryStore, Notifying};
use std::path::Path;
use std::sync::Arc;

const TOKEN: &str = "test-token";

async fn serve() -> String {
    let store = Arc::new(Notifying::new(MemoryStore::new(SystemClock)));
    let dir = std::env::temp_dir().join(format!("shiioo-dash-{}", uuid::Uuid::new_v4()));
    // No worktree directory: repo runs stop before touching the repository,
    // so the test owns the branch.
    let machine = Arc::new(LocalMachine::in_memory(dir, Toolbox::builtin()));
    let provider = Arc::new(ScriptedProvider::new(vec![ScriptedProvider::answer("ok")]));
    let factory_store = Arc::clone(&store);
    let state = ApiState::new(
        store,
        Arc::new(move || {
            Worker::new(
                Arc::clone(&factory_store),
                Arc::clone(&provider) as _,
                Arc::clone(&machine),
                "dash-test",
            )
        }),
        TOKEN,
        AgentSpec {
            model: "scripted".into(),
            system: String::new(),
            tools: vec![],
            max_turns: 3,
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .unwrap_or_else(|e| panic!("bind: {e}"));
    let addr = listener.local_addr().unwrap_or_else(|e| panic!("{e}"));
    tokio::spawn(async move {
        axum::serve(listener, router(state)).await.ok();
    });
    format!("http://{addr}")
}

async fn git(repo: &Path, args: &[&str]) -> String {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
        .args(args)
        .output()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[tokio::test]
async fn serves_the_dashboard_files_without_a_token() {
    let base = serve().await;
    let http = reqwest::Client::new();

    let page = http
        .get(format!("{base}/"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(page.status(), 200);
    assert!(
        page.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/html"))
    );
    let body = page.text().await.unwrap_or_default();
    assert!(body.contains("<title>Shiioo"), "{body}");
    assert!(body.contains("/ui/app.js"));

    let js = http
        .get(format!("{base}/ui/app.js"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(js.status(), 200);
    assert!(
        js.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.contains("javascript"))
    );
    let css = http
        .get(format!("{base}/ui/app.css"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(css.status(), 200);
}

#[tokio::test]
async fn diff_of_a_run_without_a_repository_is_not_found() {
    let base = serve().await;
    let http = reqwest::Client::new();
    let created: Value = http
        .post(format!("{base}/v1/runs"))
        .bearer_auth(TOKEN)
        .json(&json!({ "task": "say ok" }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let id = created["id"].as_str().unwrap_or_default().to_owned();

    let resp = http
        .get(format!("{base}/v1/runs/{id}/diff"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(resp.status(), 404);
    assert_eq!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/problem+json")
    );
    let problem: Value = resp.json().await.unwrap_or_default();
    assert_eq!(problem["detail"], "this run has no repository");
}

#[tokio::test]
async fn diff_shows_the_branch_changes() {
    let base = serve().await;
    let http = reqwest::Client::new();
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap_or_else(|e| panic!("{e}"));
    git(&repo, &["init", "-q", "-b", "main"]).await;
    std::fs::write(repo.join("README.md"), "hello\n").unwrap_or_else(|e| panic!("{e}"));
    git(&repo, &["add", "."]).await;
    git(&repo, &["commit", "-q", "-m", "init"]).await;

    // A harness that does nothing, so the run never reaches the repository.
    let fake = dir.path().join("claude");
    std::fs::write(&fake, "#!/bin/sh\nexit 0\n").unwrap_or_else(|e| panic!("{e}"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    let created: Value = http
        .post(format!("{base}/v1/runs"))
        .bearer_auth(TOKEN)
        .json(&json!({
            "task": "add a feature",
            "runtime": { "kind": "harness", "harness": "claude-code", "repo": repo, "binary": fake }
        }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let id = created["id"].as_str().unwrap_or_default().to_owned();
    let run_id = shiioo_core::RunId(uuid::Uuid::parse_str(&id).unwrap_or_default());

    // The branch the run would work on, with a commit.
    let wt = dir.path().join("wt");
    let branch = branch_for(run_id);
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            &branch,
            &wt.to_string_lossy(),
        ],
    )
    .await;
    std::fs::write(wt.join("feature.rs"), "fn feature() {}\n").unwrap_or_else(|e| panic!("{e}"));
    git(&wt, &["add", "."]).await;
    git(&wt, &["commit", "-q", "-m", "feature"]).await;
    // The owner moves on; the diff must still show only the branch's work.
    std::fs::write(repo.join("OTHER.md"), "owner\n").unwrap_or_else(|e| panic!("{e}"));
    git(&repo, &["add", "."]).await;
    git(&repo, &["commit", "-q", "-m", "owner commit"]).await;

    let resp = http
        .get(format!("{base}/v1/runs/{id}/diff"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(resp.status(), 200);
    assert!(
        resp.headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|t| t.starts_with("text/plain"))
    );
    let diff = resp.text().await.unwrap_or_default();
    assert!(diff.contains("feature.rs"), "{diff}");
    assert!(diff.contains("+fn feature() {}"), "{diff}");
    assert!(
        !diff.contains("OTHER.md"),
        "owner's commit leaked in: {diff}"
    );
}
