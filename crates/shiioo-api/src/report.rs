//! The morning report: what happened since the last one, what waits for
//! the owner, what is left of the week's capacity, which branches await
//! review. Counted from the record and the repositories; no model
//! involved, so it is free and never wrong about a number.

use crate::routines::ms_to_local;
use crate::schedule::humanize;
use crate::{ApiState, RunView};
use chrono::Local;
use shiioo_core::EventStore;
use shiioo_runtime::RunStatus;
use std::path::Path;

impl<S: EventStore + 'static> ApiState<S> {
    /// The report covering everything updated since `since_ms` (or the
    /// last 24 hours).
    pub async fn report(&self, since_ms: Option<u64>) -> String {
        let now = Local::now();
        let since = since_ms.unwrap_or_else(|| {
            u64::try_from(now.timestamp_millis()).unwrap_or(0) - 24 * 3600 * 1000
        });
        let runs = self.list_runs().await.unwrap_or_default();
        let recent: Vec<&RunView> = runs.iter().filter(|r| r.updated_at >= since).collect();

        let mut out = format!("📋 Report, {}\n", humanize(now));
        out.push_str(&format!(
            "Since {}:\n",
            ms_to_local(since).map_or_else(|| "the office started".to_owned(), humanize)
        ));
        let mut finished = Vec::new();
        let mut failed = Vec::new();
        let mut attention = Vec::new();
        for run in &recent {
            match &run.status {
                RunStatus::Completed { .. } => finished.push(*run),
                RunStatus::Failed { .. } | RunStatus::Cancelled { .. } => failed.push(*run),
                RunStatus::NeedsAttention { .. } => attention.push(*run),
                RunStatus::Open | RunStatus::WaitingApproval { .. } => {}
            }
        }
        let waiting: Vec<&RunView> = runs
            .iter()
            .filter(|r| matches!(r.status, RunStatus::WaitingApproval { .. }))
            .collect();
        let open: Vec<&RunView> = runs
            .iter()
            .filter(|r| matches!(r.status, RunStatus::Open))
            .collect();

        if finished.is_empty() && failed.is_empty() && attention.is_empty() {
            out.push_str("nothing finished.\n");
        }
        push_group(&mut out, "✅", "finished", &finished);
        push_group(&mut out, "❌", "failed or cancelled", &failed);
        push_group(&mut out, "⚠️", "need you", &attention);
        if !waiting.is_empty() {
            out.push_str(&format!(
                "🔐 {} waiting for your approval (see /status)\n",
                waiting.len()
            ));
        }
        if !open.is_empty() {
            out.push_str(&format!("▶️ {} still running\n", open.len()));
        }

        let branches = self.branches_awaiting_review().await;
        if !branches.is_empty() {
            out.push_str("\nBranches awaiting review:\n");
            for (repo, names) in branches {
                out.push_str(&format!("{repo}: {} ({})\n", names.len(), names.join(", ")));
            }
        }

        out.push_str("\nCapacity:\n");
        out.push_str(&self.options.capacity.text());
        out.push('\n');

        let routines = self.routines_summary();
        if !routines.is_empty() {
            out.push_str("\nNext routines:\n");
            out.push_str(&routines);
            out.push('\n');
        }
        out.trim_end().to_owned()
    }

    /// Per repository, the `shiioo/*` branches with commits not in HEAD.
    async fn branches_awaiting_review(&self) -> Vec<(String, Vec<String>)> {
        let mut out = Vec::new();
        for (label, path) in &self.options.repos {
            let names = unmerged_branches(path).await;
            if !names.is_empty() {
                out.push((label.clone(), names));
            }
        }
        out
    }
}

fn push_group(out: &mut String, mark: &str, what: &str, runs: &[&RunView]) {
    if runs.is_empty() {
        return;
    }
    out.push_str(&format!("{mark} {} {what}:\n", runs.len()));
    for run in runs.iter().take(12) {
        let task: String = run.task.chars().take(70).collect();
        let short = &run.id.to_string()[..8];
        let detail = match &run.status {
            RunStatus::Failed { reason } | RunStatus::NeedsAttention { reason } => {
                format!(" ({})", reason.chars().take(60).collect::<String>())
            }
            _ => String::new(),
        };
        out.push_str(&format!("  {short} {task}{detail}\n"));
    }
    if runs.len() > 12 {
        out.push_str(&format!("  … and {} more\n", runs.len() - 12));
    }
}

/// `shiioo/*` branches of `repo` that have commits HEAD does not.
pub async fn unmerged_branches(repo: &Path) -> Vec<String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args([
            "for-each-ref",
            "--format=%(refname:short)",
            "refs/heads/shiioo/",
        ])
        .output()
        .await;
    let Ok(out) = out else { return Vec::new() };
    let mut names = Vec::new();
    for branch in String::from_utf8_lossy(&out.stdout).lines() {
        let branch = branch.trim();
        if branch.is_empty() {
            continue;
        }
        let count = tokio::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["rev-list", "--count", &format!("HEAD..{branch}")])
            .output()
            .await;
        let ahead = count
            .ok()
            .and_then(|c| {
                String::from_utf8_lossy(&c.stdout)
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
            .unwrap_or(0);
        if ahead > 0 {
            names.push(branch.to_owned());
        }
    }
    names
}
