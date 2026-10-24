//! One git worktree and branch per run, so parallel runs never touch each
//! other's files and every run's work lands on a branch the owner can
//! review, merge or throw away.

use shiioo_core::RunId;
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    pub path: PathBuf,
    pub branch: String,
}

#[derive(Debug, thiserror::Error)]
pub enum WorktreeError {
    #[error("running git: {0}")]
    Io(#[from] std::io::Error),
    #[error("git {args}: {stderr}")]
    Git { args: String, stderr: String },
}

/// The branch a run works on.
pub fn branch_for(run_id: RunId) -> String {
    format!("shiioo/{}", &run_id.to_string()[..8])
}

/// Returns the run's worktree under `root`, creating it (and its branch
/// from the repo's current HEAD) on first use. Safe to call again after a
/// crash: an existing worktree is reused as is.
pub async fn ensure(root: &Path, repo: &Path, run_id: RunId) -> Result<Worktree, WorktreeError> {
    let path = root.join(run_id.to_string());
    let branch = branch_for(run_id);
    if path.join(".git").exists() {
        return Ok(Worktree { path, branch });
    }
    tokio::fs::create_dir_all(root).await?;
    // Clean up a registration whose directory is gone (for example a
    // deleted data directory), or git refuses to reuse the branch.
    git(repo, &["worktree", "prune"]).await?;
    let exists = git(repo, &["branch", "--list", &branch]).await?;
    let path_str = path.to_string_lossy().into_owned();
    if exists.trim().is_empty() {
        git(repo, &["worktree", "add", "-b", &branch, &path_str]).await?;
    } else {
        git(repo, &["worktree", "add", &path_str, &branch]).await?;
    }
    Ok(Worktree { path, branch })
}

/// Removes the worktree directory and its registration; the branch stays.
pub async fn remove(repo: &Path, worktree: &Path) -> Result<(), WorktreeError> {
    let path = worktree.to_string_lossy().into_owned();
    git(repo, &["worktree", "remove", "--force", &path]).await?;
    Ok(())
}

async fn git(repo: &Path, args: &[&str]) -> Result<String, WorktreeError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .await?;
    if !out.status.success() {
        return Err(WorktreeError::Git {
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
