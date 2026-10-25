//! Work intake: a GitHub issue labelled for the office becomes a run.
//!
//! The owner labels an issue `shiioo`. The intake starts a harness run for
//! it, comments with the run id, and when the run ends comments with the
//! outcome and swaps the label to `shiioo:done` or `shiioo:failed`. See
//! `docs/design/intake.md`.

mod gh;
mod state;

pub use gh::{Gh, GhError, GhIssue};
pub use state::{IssueRecord, IssueStatus, State};

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A repository the intake watches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RepoIntake {
    /// The repository's label in Shiioo's config (`[repos]`).
    pub label: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct IntakeConfig {
    pub repos: Vec<RepoIntake>,
    /// The label that hands an issue to the office. Default `shiioo`.
    pub issue_label: String,
    /// How often to poll. Default 60 s.
    pub interval: Duration,
    /// `<data>/intake-state.json`.
    pub state_file: PathBuf,
    /// The `gh` executable; default from `SHIIOO_GH_BIN` or the path.
    pub gh_bin: Option<PathBuf>,
    /// Arguments placed before every `gh` argument list (tests run a fake
    /// through `sh <script>`). Empty in production.
    pub gh_prefix_args: Vec<String>,
}

impl IntakeConfig {
    pub fn new(repos: Vec<RepoIntake>, state_file: impl Into<PathBuf>) -> Self {
        Self {
            repos,
            issue_label: "shiioo".into(),
            interval: Duration::from_secs(60),
            state_file: state_file.into(),
            gh_bin: None,
            gh_prefix_args: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Started {
    pub run_id: String,
    pub branch: String,
}

/// What the intake asks the office to do. Implemented by the server.
pub trait IntakeSink: Send + Sync {
    /// Starts a run on `repo` with `task` as the full prompt.
    fn start_run(&self, repo: &str, task: String) -> BoxFuture<'_, Result<Started, String>>;
    /// `None` while the run is still going; `Some(Ok(summary))` when it
    /// finished well; `Some(Err(reason))` when it failed, was cancelled, or
    /// needs the owner.
    fn run_outcome(&self, run_id: &str) -> BoxFuture<'_, Option<Result<String, String>>>;
}

/// The prompt a run gets for an issue. The branch is where the work must
/// land; the agent is told to push it and open a pull request.
pub fn task_for(issue: &Issue, branch: &str) -> String {
    let body = if issue.body.trim().is_empty() {
        "(no description)".to_owned()
    } else {
        issue.body.trim().to_owned()
    };
    format!(
        "Work on GitHub issue #{number}: {title}\n\
         {url}\n\n\
         Issue description:\n\
         ---\n{body}\n---\n\n\
         You are working in a git worktree on branch `{branch}`. Make the change, commit with \
         clear messages, push the branch with `git push -u origin {branch}`, then open a pull \
         request with `gh pr create --fill --head {branch}` whose description says \
         `Closes #{number}`. If something is unclear or the issue asks for something you \
         shouldn't do, stop and explain instead of guessing.",
        number = issue.number,
        title = issue.title.trim(),
        url = issue.url,
    )
}

/// Polls forever (until the future is dropped): new labelled issues become
/// runs, finished runs are reported back. Never panics; errors are logged
/// and retried on the next poll.
pub async fn run_intake(config: IntakeConfig, sink: Arc<dyn IntakeSink>) {
    let gh = Gh::with_prefix(config.gh_bin.clone(), config.gh_prefix_args.clone());
    let mut state = match State::load(&config.state_file).await {
        Ok(state) => state,
        Err(e) => {
            eprintln!(
                "intake: cannot read {}: {e}; the intake stays off until it is fixed or removed",
                config.state_file.display()
            );
            return;
        }
    };
    eprintln!(
        "intake: watching {} repo(s) for issues labelled {:?}, every {:?}",
        config.repos.len(),
        config.issue_label,
        config.interval
    );
    for repo in &config.repos {
        if let Err(e) = gh.ensure_labels(&repo.path, &config.issue_label).await {
            eprintln!("intake: labels for {}: {e}", repo.label);
        }
    }
    loop {
        for repo in &config.repos {
            if let Err(e) = poll_repo(&config, &gh, repo, &sink, &mut state).await {
                eprintln!("intake: {}: {e}", repo.label);
            }
        }
        if let Err(e) = report_outcomes(&config, &gh, &sink, &mut state).await {
            eprintln!("intake: {e}");
        }
        tokio::time::sleep(config.interval).await;
    }
}

async fn poll_repo(
    config: &IntakeConfig,
    gh: &Gh,
    repo: &RepoIntake,
    sink: &Arc<dyn IntakeSink>,
    state: &mut State,
) -> Result<(), String> {
    let issues = gh
        .list_issues(&repo.path, &config.issue_label)
        .await
        .map_err(|e| e.to_string())?;
    for gh_issue in issues {
        let key = State::key(&repo.label, gh_issue.number);
        if state.issues.contains_key(&key) {
            continue;
        }
        let issue = Issue {
            repo: repo.label.clone(),
            number: gh_issue.number,
            title: gh_issue.title,
            body: gh_issue.body,
            url: gh_issue.url,
        };
        // The branch isn't known before the run exists, and the run needs
        // the task. The sink gets a task with a placeholder it fills in,
        // or ignores: see `IntakeSink::start_run`.
        let started = match sink
            .start_run(&repo.label, task_for(&issue, BRANCH_PLACEHOLDER))
            .await
        {
            Ok(started) => started,
            Err(e) => {
                eprintln!("intake: {key}: could not start a run: {e}");
                continue;
            }
        };
        state.issues.insert(
            key.clone(),
            IssueRecord {
                run_id: started.run_id.clone(),
                status: IssueStatus::Running,
                started_at: now_ms(),
            },
        );
        persist(config, state).await;
        let comment = format!(
            "🏃 Shiioo started run `{}` on branch `{}`.",
            started.run_id, started.branch
        );
        if let Err(e) = gh.comment(&repo.path, issue.number, &comment).await {
            eprintln!("intake: {key}: comment: {e}");
        }
    }
    Ok(())
}

/// Stands in for the branch name in the task. The server replaces it with
/// the run's branch once the run id (and so the branch) is known.
pub const BRANCH_PLACEHOLDER: &str = "{branch}";

async fn report_outcomes(
    config: &IntakeConfig,
    gh: &Gh,
    sink: &Arc<dyn IntakeSink>,
    state: &mut State,
) -> Result<(), String> {
    let running: Vec<(String, IssueRecord)> = state
        .issues
        .iter()
        .filter(|(_, r)| r.status == IssueStatus::Running)
        .map(|(k, r)| (k.clone(), r.clone()))
        .collect();
    for (key, record) in running {
        let Some(outcome) = sink.run_outcome(&record.run_id).await else {
            continue;
        };
        let Some((repo_label, number)) = split_key(&key) else {
            continue;
        };
        let Some(repo) = config.repos.iter().find(|r| r.label == repo_label) else {
            continue;
        };
        let (status, comment, label) = match &outcome {
            Ok(summary) => (
                IssueStatus::Done,
                format!("✅ Done: {}", clip(summary, 2000)),
                format!("{}:done", config.issue_label),
            ),
            Err(reason) => (
                IssueStatus::Failed,
                format!("❌ {}", clip(reason, 2000)),
                format!("{}:failed", config.issue_label),
            ),
        };
        if let Some(r) = state.issues.get_mut(&key) {
            r.status = status;
        }
        persist(config, state).await;
        if let Err(e) = gh.comment(&repo.path, number, &comment).await {
            eprintln!("intake: {key}: comment: {e}");
        }
        if let Err(e) = gh
            .relabel(&repo.path, number, &config.issue_label, &label)
            .await
        {
            eprintln!("intake: {key}: relabel: {e}");
        }
    }
    Ok(())
}

async fn persist(config: &IntakeConfig, state: &State) {
    if let Err(e) = state.save(&config.state_file).await {
        eprintln!("intake: cannot write {}: {e}", config.state_file.display());
    }
}

fn split_key(key: &str) -> Option<(&str, u64)> {
    let (repo, number) = key.rsplit_once('#')?;
    Some((repo, number.parse().ok()?))
}

fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_owned(),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Replaces the branch placeholder in a task once the branch is known.
pub fn with_branch(task: &str, branch: &str) -> String {
    task.replace(BRANCH_PLACEHOLDER, branch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_mentions_everything_the_agent_needs() {
        let issue = Issue {
            repo: "demo".into(),
            number: 42,
            title: "Flaky test in net.rs".into(),
            body: "It fails every third run.".into(),
            url: "https://github.com/me/demo/issues/42".into(),
        };
        let task = with_branch(&task_for(&issue, BRANCH_PLACEHOLDER), "shiioo/abcd1234");
        for needle in [
            "#42",
            "Flaky test in net.rs",
            "It fails every third run.",
            "https://github.com/me/demo/issues/42",
            "shiioo/abcd1234",
            "Closes #42",
            "git push -u origin shiioo/abcd1234",
            "gh pr create --fill --head shiioo/abcd1234",
        ] {
            assert!(task.contains(needle), "missing {needle:?} in:\n{task}");
        }
        assert!(!task.contains(BRANCH_PLACEHOLDER));
    }

    #[test]
    fn empty_body_is_said_so() {
        let issue = Issue {
            repo: "demo".into(),
            number: 1,
            title: "t".into(),
            body: "  \n".into(),
            url: "u".into(),
        };
        assert!(task_for(&issue, "b").contains("(no description)"));
    }

    #[test]
    fn keys_round_trip() {
        let key = State::key("my-repo", 7);
        assert_eq!(split_key(&key), Some(("my-repo", 7)));
        assert_eq!(split_key("nonsense"), None);
    }
}
