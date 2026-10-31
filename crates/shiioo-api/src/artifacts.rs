//! Deliverables: the files a run produced. Code goes out as a branch and a
//! pull request; a rendered image, an audio file, a PDF or a dataset goes
//! out as a file the owner can look at on the phone. The list is the
//! branch's changes against the repository's HEAD plus anything untracked
//! the agent left in the worktree.

use crate::{ApiError, ApiState};
use serde::Serialize;
use shiioo_core::{EventStore, RunId, Seq};
use shiioo_runtime::{Runtime, branch_for};
use std::path::{Path, PathBuf};

/// Files above this size are listed but not sent to the phone.
pub const SEND_LIMIT_BYTES: u64 = 50 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Artifact {
    /// Path relative to the repository root.
    pub path: String,
    /// `added`, `modified`, `deleted`, `untracked`.
    pub status: String,
    pub size: u64,
    /// `image`, `audio`, `video`, `pdf`, `archive`, `text`, `other`.
    pub kind: String,
    pub content_type: String,
}

impl Artifact {
    /// Worth sending to the phone on its own: a thing to look at or listen
    /// to, small enough.
    pub fn is_media(&self) -> bool {
        matches!(self.kind.as_str(), "image" | "audio" | "video" | "pdf")
            && self.status != "deleted"
            && self.size > 0
            && self.size <= SEND_LIMIT_BYTES
    }
}

/// Kind and media type from the extension.
pub fn classify(path: &str) -> (&'static str, &'static str) {
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => ("image", "image/png"),
        "jpg" | "jpeg" => ("image", "image/jpeg"),
        "gif" => ("image", "image/gif"),
        "webp" => ("image", "image/webp"),
        "svg" => ("image", "image/svg+xml"),
        "mp3" => ("audio", "audio/mpeg"),
        "wav" => ("audio", "audio/wav"),
        "flac" => ("audio", "audio/flac"),
        "ogg" => ("audio", "audio/ogg"),
        "m4a" => ("audio", "audio/mp4"),
        "mp4" => ("video", "video/mp4"),
        "mov" => ("video", "video/quicktime"),
        "webm" => ("video", "video/webm"),
        "mkv" => ("video", "video/x-matroska"),
        "pdf" => ("pdf", "application/pdf"),
        "zip" => ("archive", "application/zip"),
        "tar" | "gz" | "tgz" => ("archive", "application/gzip"),
        "md" | "txt" | "rs" | "py" | "js" | "ts" | "toml" | "json" | "yaml" | "yml" | "html"
        | "css" | "sh" | "csv" => ("text", "text/plain; charset=utf-8"),
        _ => ("other", "application/octet-stream"),
    }
}

impl<S: EventStore + 'static> ApiState<S> {
    /// The repository, worktree and branch of a run on this machine, if it
    /// has one. A follow-up's are those of the run it continues.
    async fn run_paths(
        &self,
        run_id: RunId,
    ) -> Result<(PathBuf, Option<PathBuf>, String), ApiError> {
        let log = self.store.read(run_id, Seq(0)).await?;
        let spec = crate::run_spec(&log)
            .ok_or_else(|| ApiError::NotFound("run has no spec".into()))?;
        let workspace = spec.workspace_run(run_id);
        if let Some(machine) = spec.machine {
            return Err(ApiError::Conflict(format!(
                "this run worked on {machine}; its files are there, not in the office"
            )));
        }
        let repo = match spec.runtime {
            Runtime::Harness(h) => h.repo,
            Runtime::Native => None,
        }
        .ok_or_else(|| ApiError::NotFound("this run has no repository".into()))?;
        let worktree = self
            .options
            .worktrees
            .as_ref()
            .map(|root| root.join(workspace.to_string()))
            .filter(|p| p.exists());
        Ok((repo, worktree, branch_for(workspace)))
    }

    /// What the run produced: the branch's changes against HEAD, plus
    /// untracked files in the worktree.
    pub async fn artifacts(&self, run_id: RunId) -> Result<Vec<Artifact>, ApiError> {
        let (repo, worktree, branch) = self.run_paths(run_id).await?;
        let mut out = Vec::new();
        let diff = git(&repo, &["diff", "--name-status", &format!("HEAD...{branch}")]).await?;
        for line in diff.lines() {
            let mut parts = line.split('\t');
            let (Some(code), Some(path)) = (parts.next(), parts.next_back()) else {
                continue;
            };
            let status = match code.chars().next() {
                Some('A') => "added",
                Some('M') => "modified",
                Some('D') => "deleted",
                Some('R') => "added",
                _ => "modified",
            };
            let size = match &worktree {
                Some(wt) => std::fs::metadata(wt.join(path)).map(|m| m.len()).unwrap_or(0),
                None => git_size(&repo, &branch, path).await,
            };
            let (kind, content_type) = classify(path);
            out.push(Artifact {
                path: path.to_owned(),
                status: status.into(),
                size,
                kind: kind.into(),
                content_type: content_type.into(),
            });
        }
        if let Some(wt) = &worktree {
            let untracked = git(wt, &["ls-files", "--others", "--exclude-standard"]).await?;
            for path in untracked.lines().map(str::trim).filter(|p| !p.is_empty()) {
                if out.iter().any(|a| a.path == path) {
                    continue;
                }
                let size = std::fs::metadata(wt.join(path)).map(|m| m.len()).unwrap_or(0);
                let (kind, content_type) = classify(path);
                out.push(Artifact {
                    path: path.to_owned(),
                    status: "untracked".into(),
                    size,
                    kind: kind.into(),
                    content_type: content_type.into(),
                });
            }
        }
        Ok(out)
    }

    /// One artifact's bytes: from the worktree when it still exists, else
    /// from the branch.
    pub async fn artifact_bytes(
        &self,
        run_id: RunId,
        path: &str,
    ) -> Result<(Vec<u8>, String), ApiError> {
        if path.split('/').any(|p| p == "..") || path.starts_with('/') {
            return Err(ApiError::BadRequest("path must be inside the repository".into()));
        }
        let (repo, worktree, branch) = self.run_paths(run_id).await?;
        let (_, content_type) = classify(path);
        if let Some(wt) = &worktree {
            let full = wt.join(path);
            if full.is_file() {
                let size = std::fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
                if size > SEND_LIMIT_BYTES {
                    return Err(ApiError::Conflict(format!("{path} is larger than 50 MB")));
                }
                return std::fs::read(&full)
                    .map(|b| (b, content_type.to_owned()))
                    .map_err(|e| ApiError::Failed(format!("{path}: {e}")));
            }
        }
        let out = tokio::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["show", &format!("{branch}:{path}")])
            .output()
            .await
            .map_err(|e| ApiError::Failed(format!("running git: {e}")))?;
        if !out.status.success() {
            return Err(ApiError::NotFound(format!("{path} is not in the run's branch")));
        }
        Ok((out.stdout, content_type.to_owned()))
    }

    /// The artifacts as the phone lists them.
    pub async fn artifacts_text(&self, run_id: RunId) -> Result<String, ApiError> {
        let artifacts = self.artifacts(run_id).await?;
        if artifacts.is_empty() {
            return Ok("No files changed or produced.".to_owned());
        }
        let short = &run_id.to_string()[..8];
        let mut out = format!("Files of {short} ({}):\n", artifacts.len());
        for a in &artifacts {
            out.push_str(&format!(
                "{} {} ({}, {}){}\n",
                match a.status.as_str() {
                    "added" | "untracked" => "+",
                    "deleted" => "-",
                    _ => "~",
                },
                a.path,
                a.kind,
                human_size(a.size),
                if a.is_media() {
                    format!("  /get {short} {}", a.path)
                } else {
                    String::new()
                }
            ));
        }
        Ok(out.trim_end().to_owned())
    }
}

pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

async fn git(dir: &Path, args: &[&str]) -> Result<String, ApiError> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .await
        .map_err(|e| ApiError::Failed(format!("running git: {e}")))?;
    if !out.status.success() {
        return Err(ApiError::Failed(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn git_size(repo: &Path, branch: &str, path: &str) -> u64 {
    tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["cat-file", "-s", &format!("{branch}:{path}")])
        .output()
        .await
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_and_sizes() {
        assert_eq!(classify("out/cover.PNG"), ("image", "image/png"));
        assert_eq!(classify("song.flac").0, "audio");
        assert_eq!(classify("README.md").0, "text");
        assert_eq!(classify("model.bin").0, "other");
        assert_eq!(human_size(10), "10 B");
        assert_eq!(human_size(2048), "2 KB");
        assert_eq!(human_size(3 * 1024 * 1024), "3.0 MB");
        let a = Artifact {
            path: "a.png".into(),
            status: "added".into(),
            size: 10,
            kind: "image".into(),
            content_type: "image/png".into(),
        };
        assert!(a.is_media());
        assert!(!Artifact { status: "deleted".into(), ..a.clone() }.is_media());
        assert!(!Artifact { kind: "text".into(), ..a }.is_media());
    }
}
