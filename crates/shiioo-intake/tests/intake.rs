//! The intake loop against a fake `gh` and a recording sink.

use serde_json::json;
use shiioo_intake::{
    BoxFuture, IntakeConfig, IntakeSink, IssueRecord, IssueStatus, RepoIntake, Started, State,
    run_intake,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Writes a `gh` stand-in that logs every invocation (one line, fields
/// separated by `\x1f`) and answers `issue list` with two open issues.
fn fake_gh(dir: &Path) -> PathBuf {
    let log = dir.join("gh-args.log");
    let issues = json!([
        { "number": 1, "title": "Old one", "body": "already handled", "url": "https://github.com/me/demo/issues/1", "labels": [{ "name": "shiioo" }] },
        { "number": 2, "title": "Fix the flaky test", "body": "It fails every third run.", "url": "https://github.com/me/demo/issues/2", "labels": [{ "name": "shiioo" }] }
    ]);
    let issues_file = dir.join("issues.json");
    std::fs::write(&issues_file, issues.to_string()).unwrap_or_else(|e| panic!("{e}"));
    let script = format!(
        "#!/bin/sh\nprintf '%s\\037' \"$@\" >> '{log}'\nprintf '\\n' >> '{log}'\n\
         case \"$1 $2\" in \"issue list\") cat '{issues}';; esac\nexit 0\n",
        log = log.display(),
        issues = issues_file.display(),
    );
    let bin = dir.join("gh");
    std::fs::write(&bin, script).unwrap_or_else(|e| panic!("{e}"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755))
            .unwrap_or_else(|e| panic!("{e}"));
    }
    bin
}

fn gh_calls(dir: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(dir.join("gh-args.log"))
        .unwrap_or_default()
        .lines()
        .map(|l| {
            l.split('\u{1f}')
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .collect()
}

struct RecordingSink {
    starts: Mutex<Vec<(String, String)>>,
    outcome_polls: AtomicUsize,
}

impl IntakeSink for RecordingSink {
    fn start_run(&self, repo: &str, task: String) -> BoxFuture<'_, Result<Started, String>> {
        let repo = repo.to_owned();
        Box::pin(async move {
            self.starts
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .push((repo, task));
            Ok(Started {
                run_id: "run-1".into(),
                branch: "shiioo/run1".into(),
            })
        })
    }

    fn run_outcome(&self, run_id: &str) -> BoxFuture<'_, Option<Result<String, String>>> {
        assert_eq!(run_id, "run-1");
        Box::pin(async move {
            // Still running on the first poll, finished on the second.
            if self.outcome_polls.fetch_add(1, Ordering::SeqCst) == 0 {
                None
            } else {
                Some(Ok("merged the fix on shiioo/run1, PR #3 opened".into()))
            }
        })
    }
}

#[tokio::test]
async fn new_issues_become_runs_and_outcomes_are_reported() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap_or_else(|e| panic!("{e}"));
    let gh = fake_gh(dir.path());
    let state_file = dir.path().join("intake-state.json");

    // Issue #1 was handled in an earlier life.
    let mut seen = State::default();
    seen.issues.insert(
        State::key("demo", 1),
        IssueRecord {
            run_id: "run-0".into(),
            status: IssueStatus::Done,
            started_at: 1,
        },
    );
    seen.save(&state_file)
        .await
        .unwrap_or_else(|e| panic!("{e}"));

    let mut config = IntakeConfig::new(
        vec![RepoIntake {
            label: "demo".into(),
            path: repo.clone(),
        }],
        &state_file,
    );
    config.interval = Duration::from_millis(100);
    config.gh_bin = Some(PathBuf::from("/bin/sh"));
    config.gh_prefix_args = vec![gh.to_string_lossy().into_owned()];
    let sink = Arc::new(RecordingSink {
        starts: Mutex::new(Vec::new()),
        outcome_polls: AtomicUsize::new(0),
    });

    let _ = tokio::time::timeout(
        Duration::from_millis(1200),
        run_intake(config, Arc::clone(&sink) as Arc<dyn IntakeSink>),
    )
    .await;

    // Exactly one run, for the unseen issue, with the issue in the task.
    let starts = sink
        .starts
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone();
    assert_eq!(starts.len(), 1, "{starts:?}");
    assert_eq!(starts[0].0, "demo");
    assert!(starts[0].1.contains("#2") && starts[0].1.contains("Fix the flaky test"));

    let calls = gh_calls(dir.path());
    let labels_created = calls
        .iter()
        .filter(|c| c.first().map(String::as_str) == Some("label"))
        .count();
    assert_eq!(labels_created, 3, "{calls:?}");
    let comments: Vec<&Vec<String>> = calls
        .iter()
        .filter(|c| c.len() > 2 && c[0] == "issue" && c[1] == "comment")
        .collect();
    assert_eq!(comments.len(), 2, "{comments:?}");
    assert_eq!(comments[0][2], "2");
    assert!(comments[0][4].starts_with("🏃 Shiioo started run `run-1` on branch `shiioo/run1`"));
    assert_eq!(comments[1][2], "2");
    assert!(comments[1][4].starts_with("✅ Done: merged the fix"));
    let relabels: Vec<&Vec<String>> = calls
        .iter()
        .filter(|c| c.len() > 2 && c[0] == "issue" && c[1] == "edit")
        .collect();
    assert_eq!(relabels.len(), 1, "{relabels:?}");
    assert_eq!(
        relabels[0][2..],
        [
            "2".to_owned(),
            "--remove-label".to_owned(),
            "shiioo".to_owned(),
            "--add-label".to_owned(),
            "shiioo:done".to_owned()
        ]
    );
    // No call ever touched issue #1 again.
    assert!(
        !calls
            .iter()
            .any(|c| c.len() > 2 && c[0] == "issue" && c[1] != "list" && c[2] == "1")
    );

    let state = State::load(&state_file)
        .await
        .unwrap_or_else(|e| panic!("{e}"));
    let expected: BTreeMap<String, IssueStatus> = state
        .issues
        .iter()
        .map(|(k, r)| (k.clone(), r.status))
        .collect();
    assert_eq!(expected.get("demo#1"), Some(&IssueStatus::Done));
    assert_eq!(expected.get("demo#2"), Some(&IssueStatus::Done));
    assert_eq!(state.issues["demo#2"].run_id, "run-1");
}

#[tokio::test]
async fn a_corrupt_state_file_stops_the_intake_instead_of_rerunning_everything() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap_or_else(|e| panic!("{e}"));
    let gh = fake_gh(dir.path());
    let state_file = dir.path().join("intake-state.json");
    std::fs::write(&state_file, "{ not json").unwrap_or_else(|e| panic!("{e}"));

    let mut config = IntakeConfig::new(
        vec![RepoIntake {
            label: "demo".into(),
            path: repo,
        }],
        &state_file,
    );
    config.interval = Duration::from_millis(50);
    config.gh_bin = Some(PathBuf::from("/bin/sh"));
    config.gh_prefix_args = vec![gh.to_string_lossy().into_owned()];
    let sink = Arc::new(RecordingSink {
        starts: Mutex::new(Vec::new()),
        outcome_polls: AtomicUsize::new(0),
    });
    // Returns on its own, quickly, having started nothing.
    tokio::time::timeout(
        Duration::from_millis(500),
        run_intake(config, Arc::clone(&sink) as Arc<dyn IntakeSink>),
    )
    .await
    .unwrap_or_else(|_| panic!("intake kept running with a corrupt state file"));
    assert!(
        sink.starts
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty()
    );
    assert!(gh_calls(dir.path()).is_empty());
}

#[tokio::test]
async fn gh_runner_talks_to_the_fake() {
    let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap_or_else(|e| panic!("{e}"));
    let gh = fake_gh(dir.path());
    let runner = shiioo_intake::Gh::with_prefix(
        Some(PathBuf::from("/bin/sh")),
        vec![gh.to_string_lossy().into_owned()],
    );
    let issues = runner
        .list_issues(&repo, "shiioo")
        .await
        .unwrap_or_else(|e| panic!("list: {e}"));
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[1].title, "Fix the flaky test");
    assert_eq!(gh_calls(dir.path()).len(), 1);
}
