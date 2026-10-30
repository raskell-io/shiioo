//! The dashboard files and the diff endpoint.

use serde_json::{Value, json};
use shiioo_api::{ApiState, UI_FILES, router};
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

    for (name, body) in UI_FILES {
        let resp = http
            .get(format!("{base}/ui/{name}"))
            .send()
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(resp.status(), 200, "{name}");
        assert_eq!(resp.text().await.unwrap_or_default(), *body, "{name}");
    }
    let missing = http
        .get(format!("{base}/ui/nope.js"))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(missing.status(), 404);
}

/// Every file under `ui/` (but the page itself) is in `UI_FILES`, and every
/// relative module import resolves to one of them.
#[test]
fn ui_files_match_the_directory_and_imports_resolve() {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{e}")) {
            let path = entry.unwrap_or_else(|e| panic!("{e}")).path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut on_disk = Vec::new();
    walk(&root, &root, &mut on_disk);
    on_disk.retain(|f| f != "index.html");
    on_disk.sort();
    let mut listed: Vec<String> = UI_FILES.iter().map(|(n, _)| (*n).to_owned()).collect();
    listed.sort();
    assert_eq!(listed, on_disk, "UI_FILES and ui/ disagree");

    for (name, body) in UI_FILES {
        let dir = Path::new(name).parent().unwrap_or(Path::new(""));
        for line in body.lines() {
            let line = line.trim();
            if !line.starts_with("import ") {
                continue;
            }
            let Some(spec) = line.split('"').nth(1) else {
                continue;
            };
            let mut parts: Vec<&str> = dir
                .to_str()
                .unwrap_or("")
                .split('/')
                .filter(|p| !p.is_empty())
                .collect();
            for seg in spec.split('/') {
                match seg {
                    "." => {}
                    ".." => {
                        parts.pop();
                    }
                    s => parts.push(s),
                }
            }
            let target = parts.join("/");
            assert!(
                UI_FILES.iter().any(|(n, _)| *n == target),
                "{name} imports {spec}, which is not served"
            );
        }
    }
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
            "runtime": { "kind": "harness", "harness": "claude-code", "repo": repo, "binary": "/bin/sh", "binary_args": [fake] }
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
