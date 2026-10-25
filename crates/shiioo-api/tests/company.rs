//! Phase 5 over HTTP: roles, the chief of staff's plans, routines and the
//! report, with a scripted model and a fake issue creator.

use serde_json::{Value, json};
use shiioo_api::{
    ApiOptions, ApiState, BoxFuture, Employee, EmployeeKind, IssueCreator, Notifications, Role,
    Routine, router,
};
use shiioo_core::{RunId, SystemClock};
use shiioo_runtime::{AgentSpec, LocalMachine, ModelProvider, RunStatus, ScriptedProvider, Toolbox, Worker};
use shiioo_store::{MemoryStore, Notifying};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const TOKEN: &str = "test-token";

#[derive(Default)]
struct Recorder {
    issues: Mutex<Vec<(PathBuf, String, String, String)>>,
    messages: Mutex<Vec<String>>,
    ended: Mutex<Vec<RunId>>,
}

impl IssueCreator for Recorder {
    fn create_issue(
        &self,
        repo: &Path,
        title: &str,
        body: &str,
        label: &str,
    ) -> shiioo_api::plans::BoxFuture<'_, Result<String, String>> {
        let n = lock(&self.issues).len() + 1;
        lock(&self.issues).push((repo.to_path_buf(), title.into(), body.into(), label.into()));
        Box::pin(async move { Ok(format!("https://github.com/me/demo/issues/{n}")) })
    }
}

impl Notifications for Recorder {
    fn approval_requested(&self, _: RunId, _: String, _: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn approval_resolved(&self, _: String, _: String) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
    fn run_ended(&self, run: RunId, _: String, _: RunStatus) -> BoxFuture<'_, ()> {
        lock(&self.ended).push(run);
        Box::pin(async {})
    }
    fn message(&self, text: String) -> BoxFuture<'_, ()> {
        lock(&self.messages).push(text);
        Box::pin(async {})
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

struct Company {
    base: String,
    recorder: Arc<Recorder>,
    _dir: tempfile::TempDir,
}

/// An office with one scripted native employee, a role over it, a repo,
/// a routine and a report schedule.
async fn serve(script: Vec<shiioo_runtime::ModelResponse>) -> Company {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let repo = dir.path().join("demo");
    std::fs::create_dir_all(&repo).unwrap_or_else(|e| panic!("{e}"));
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec!["config", "user.email", "t@example.com"],
        vec!["config", "user.name", "t"],
    ] {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(&args)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        assert!(ok, "git {args:?}");
    }
    std::fs::write(repo.join("README.md"), "demo\n").unwrap_or_else(|e| panic!("{e}"));
    let ok = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["add", "."])
        .status()
        .and_then(|_| {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(["commit", "-q", "-m", "init"])
                .status()
        })
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(ok, "git commit");

    let store = Arc::new(Notifying::new(MemoryStore::new(SystemClock)));
    let machine = Arc::new(LocalMachine::in_memory(dir.path().join("ws"), Toolbox::builtin()));
    std::fs::create_dir_all(dir.path().join("ws")).unwrap_or_else(|e| panic!("{e}"));
    let provider: Arc<dyn ModelProvider> = Arc::new(ScriptedProvider::new(script));
    let factory_store = Arc::clone(&store);
    let recorder = Arc::new(Recorder::default());
    let mut repos = BTreeMap::new();
    repos.insert("demo".to_owned(), repo);
    let state = ApiState::with_options(
        store,
        Arc::new(move || {
            Worker::new(
                Arc::clone(&factory_store),
                Arc::clone(&provider),
                Arc::clone(&machine),
                "company-test",
            )
        }),
        TOKEN,
        AgentSpec {
            model: "scripted".into(),
            system: String::new(),
            tools: vec!["write_file".into()],
            max_turns: 5,
        },
        ApiOptions {
            employees: vec![Employee {
                name: "scripted".into(),
                kind: EmployeeKind::Native {
                    model: "scripted".into(),
                },
                max_five_hour: 0.95,
                max_seven_day: 0.9,
            }],
            repos,
            roles: vec![Role {
                name: "writer".into(),
                employee: "scripted".into(),
                system: "Be terse.".into(),
                tools: vec!["write_file".into()],
                model: None,
                isolation: None,
                max_minutes: Some(7),
                max_turns: Some(3),
            }],
            routines: vec![Routine {
                name: "nightly".into(),
                schedule: "daily 02:00".into(),
                repo: Some("demo".into()),
                role: Some("writer".into()),
                employee: "auto".into(),
                task: "Write the nightly note.".into(),
                enabled: true,
            }],
            report: Some("weekdays 07:00".into()),
            data_dir: Some(dir.path().join("data")),
            issues: Some(Arc::clone(&recorder) as Arc<dyn IssueCreator>),
            notifications: Some(Arc::clone(&recorder) as Arc<dyn Notifications>),
            intake_label: "shiioo".into(),
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
    Company {
        base: format!("http://{addr}"),
        recorder,
        _dir: dir,
    }
}

async fn wait_terminal(http: &reqwest::Client, base: &str, run: &str) -> Value {
    for _ in 0..100 {
        let view: Value = http
            .get(format!("{base}/v1/runs/{run}"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .json()
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let state = view["status"]["state"].as_str().unwrap_or_default();
        if !matches!(state, "open" | "waiting_approval") {
            return view;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("run {run} never finished");
}

#[tokio::test]
async fn a_role_lays_its_configuration_over_the_employee() {
    let company = serve(vec![ScriptedProvider::answer("Done, tersely.")]).await;
    let http = reqwest::Client::new();
    let created: Value = http
        .post(format!("{}/v1/runs", company.base))
        .bearer_auth(TOKEN)
        .json(&json!({ "task": "Write a note.", "role": "writer", "repo": "demo" }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let run = created["id"].as_str().unwrap_or_default().to_owned();
    let view = wait_terminal(&http, &company.base, &run).await;
    assert_eq!(view["status"]["state"], "completed", "{view}");
    let events: Value = http
        .get(format!("{}/v1/runs/{run}/events?from=0", company.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let spec = &events[0]["payload"]["spec"];
    assert_eq!(spec["agent"]["system"], "Be terse.");
    assert_eq!(spec["agent"]["max_turns"], 3);
    assert_eq!(spec["agent"]["tools"], json!(["write_file"]));
    // An unknown role is a clear 400.
    let status = http
        .post(format!("{}/v1/runs", company.base))
        .bearer_auth(TOKEN)
        .json(&json!({ "task": "x", "role": "ceo" }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .status();
    assert_eq!(status, 400);
    let roles: Value = http
        .get(format!("{}/v1/roles", company.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(roles[0]["name"], "writer");
}

#[tokio::test]
async fn a_plan_is_proposed_from_the_planners_answer_and_approved_into_issues() {
    let plan = "Looked at the repo.\n```json\n{\"notes\": \"Two small steps.\", \"items\": [\
                {\"title\": \"Write the changelog\", \"body\": \"From the git log.\"}, \
                {\"title\": \"Tag the release\", \"body\": \"After the changelog.\"}]}\n```";
    let company = serve(vec![ScriptedProvider::answer(plan)]).await;
    let http = reqwest::Client::new();
    let created: Value = http
        .post(format!("{}/v1/plans", company.base))
        .bearer_auth(TOKEN)
        .json(&json!({ "repo": "demo", "goal": "Ship v0.3" }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let run = created["plan"].as_str().unwrap_or_default().to_owned();
    wait_terminal(&http, &company.base, &run).await;
    // The planner's spec carries the planning task, not the goal verbatim.
    let events: Value = http
        .get(format!("{}/v1/runs/{run}/events?from=0", company.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let task = events[0]["payload"]["spec"]["task"].as_str().unwrap_or_default();
    assert!(task.contains("chief of staff") && task.contains("Ship v0.3"), "{task}");

    // The plan is proposed once the run ends; the owner got the plan, not
    // a run notification.
    let mut plan_view = Value::Null;
    for _ in 0..100 {
        plan_view = http
            .get(format!("{}/v1/plans/{}", company.base, &run[..8]))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .json()
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        if plan_view["status"]["status"] == "proposed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(plan_view["status"]["status"], "proposed", "{plan_view}");
    assert_eq!(plan_view["items"].as_array().map(Vec::len), Some(2));
    assert_eq!(plan_view["notes"], "Two small steps.");
    let messages = lock(&company.recorder.messages).clone();
    assert!(
        messages.iter().any(|m| m.contains("1. Write the changelog") && m.contains("/plan approve")),
        "{messages:?}"
    );
    assert!(lock(&company.recorder.ended).is_empty());

    // Approving creates one labelled issue per item, in order.
    let approved: Value = http
        .post(format!("{}/v1/plans/{}/approve", company.base, &run[..8]))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(approved["plan"]["status"]["status"], "approved", "{approved}");
    let issues = lock(&company.recorder.issues).clone();
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].1, "Write the changelog");
    assert_eq!(issues[0].3, "shiioo");
    assert!(issues[0].2.contains("From the git log.") && issues[0].2.contains("Part 1 of 2"));
    assert!(issues[1].2.contains("2. **Tag the release** (this issue)"));
    assert!(approved["message"].as_str().unwrap_or_default().contains("issues/2"));
    // A second approval is refused.
    let status = http
        .post(format!("{}/v1/plans/{}/approve", company.base, &run[..8]))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .status();
    assert_eq!(status, 409);
    // The plan survived on disk.
    let on_disk = std::fs::read_to_string(company._dir.path().join("data/plans.json"))
        .unwrap_or_default();
    assert!(on_disk.contains("Write the changelog"));
}

#[tokio::test]
async fn an_unreadable_plan_fails_with_the_planners_words() {
    let company = serve(vec![ScriptedProvider::answer("I could not decide.")]).await;
    let http = reqwest::Client::new();
    let created: Value = http
        .post(format!("{}/v1/plans", company.base))
        .bearer_auth(TOKEN)
        .json(&json!({ "repo": "demo", "goal": "Do it" }))
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let run = created["plan"].as_str().unwrap_or_default().to_owned();
    wait_terminal(&http, &company.base, &run).await;
    let mut plan_view = Value::Null;
    for _ in 0..100 {
        plan_view = http
            .get(format!("{}/v1/plans/{run}", company.base))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .json()
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        if plan_view["status"]["status"] == "failed" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let reason = plan_view["status"]["reason"].as_str().unwrap_or_default();
    assert!(reason.contains("not JSON") && reason.contains("I could not decide."), "{plan_view}");
}

#[tokio::test]
async fn routines_fire_on_demand_and_the_report_counts_them() {
    let company = serve(vec![ScriptedProvider::answer("Nightly note written.")]).await;
    let http = reqwest::Client::new();
    let listed: Value = http
        .get(format!("{}/v1/routines", company.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let summary = listed["summary"].as_str().unwrap_or_default();
    assert!(summary.contains("nightly: daily 02:00, next") && summary.contains("on demo"), "{summary}");
    assert!(summary.contains("report: weekdays 07:00, next"), "{summary}");

    let fired: Value = http
        .post(format!("{}/v1/routines/nightly/run", company.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .json()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let message = fired["message"].as_str().unwrap_or_default();
    assert!(message.starts_with("⏰ routine nightly started run"), "{message}");
    assert!(message.contains("scripted as @writer"), "{message}");
    let run = fired["run"].as_str().unwrap_or_default().to_owned();
    let view = wait_terminal(&http, &company.base, &run).await;
    assert_eq!(view["status"]["state"], "completed", "{view}");
    assert_eq!(view["task"], "Write the nightly note.");

    // Firing an unknown routine is a 404; the state file knows the firing.
    let status = http
        .post(format!("{}/v1/routines/nope/run", company.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .status();
    assert_eq!(status, 404);
    let state = std::fs::read_to_string(company._dir.path().join("data/routines-state.json"))
        .unwrap_or_default();
    assert!(state.contains("nightly"), "{state}");

    let report = http
        .get(format!("{}/v1/report", company.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .text()
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(report.starts_with("📋 Report, "), "{report}");
    assert!(report.contains("✅ 1 finished:") && report.contains("Write the nightly note."), "{report}");
    assert!(report.contains("Capacity:") && report.contains("Next routines:"), "{report}");
}
