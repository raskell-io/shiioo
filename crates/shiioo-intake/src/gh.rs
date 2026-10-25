//! A thin runner for the `gh` CLI. Every command runs with the repository
//! as its working directory, so per-repository account wrappers (which pick
//! a GitHub account from the current repo) keep working.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use tokio::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum GhError {
    #[error("running gh: {0}")]
    Io(#[from] std::io::Error),
    #[error("gh {args}: {stderr}")]
    Failed { args: String, stderr: String },
    #[error("gh returned unexpected JSON: {0}")]
    Decode(#[from] serde_json::Error),
}

/// An issue as `gh issue list --json` reports it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct GhIssue {
    pub number: u64,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub url: String,
    #[serde(default)]
    pub labels: Vec<GhLabel>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct GhLabel {
    pub name: String,
}

pub struct Gh {
    program: PathBuf,
    /// Arguments placed before the `gh` arguments, so a test can run a
    /// fake through `sh <script>` instead of executing a fresh script
    /// directly (macOS assesses new executables on first run, slowly).
    prefix: Vec<String>,
}

impl Gh {
    /// Uses `binary`, else `SHIIOO_GH_BIN`, else `gh` on the path.
    pub fn new(binary: Option<PathBuf>) -> Self {
        Self::with_prefix(binary, Vec::new())
    }

    pub fn with_prefix(binary: Option<PathBuf>, prefix: Vec<String>) -> Self {
        Self {
            program: binary.unwrap_or_else(|| {
                std::env::var_os("SHIIOO_GH_BIN").map_or_else(|| PathBuf::from("gh"), PathBuf::from)
            }),
            prefix,
        }
    }

    pub async fn list_issues(&self, repo: &Path, label: &str) -> Result<Vec<GhIssue>, GhError> {
        let out = self
            .run(
                repo,
                &[
                    "issue",
                    "list",
                    "--label",
                    label,
                    "--state",
                    "open",
                    "--json",
                    "number,title,body,url,labels",
                    "--limit",
                    "50",
                ],
            )
            .await?;
        Ok(serde_json::from_str(out.trim())?)
    }

    pub async fn comment(&self, repo: &Path, number: u64, body: &str) -> Result<(), GhError> {
        self.run(
            repo,
            &["issue", "comment", &number.to_string(), "--body", body],
        )
        .await
        .map(drop)
    }

    /// Swaps `remove` for `add` on the issue.
    pub async fn relabel(
        &self,
        repo: &Path,
        number: u64,
        remove: &str,
        add: &str,
    ) -> Result<(), GhError> {
        self.run(
            repo,
            &[
                "issue",
                "edit",
                &number.to_string(),
                "--remove-label",
                remove,
                "--add-label",
                add,
            ],
        )
        .await
        .map(drop)
    }

    /// Creates or updates the labels the protocol uses. Failures are the
    /// caller's to ignore: labels may already exist or the token may lack
    /// the permission, and neither stops the intake.
    pub async fn ensure_labels(&self, repo: &Path, label: &str) -> Result<(), GhError> {
        let labels = [
            (
                label.to_owned(),
                "1D76DB",
                "Shiioo: an agent should work on this",
            ),
            (
                format!("{label}:done"),
                "0E8A16",
                "Shiioo: the agent finished",
            ),
            (
                format!("{label}:failed"),
                "B60205",
                "Shiioo: the agent could not finish",
            ),
        ];
        for (name, color, description) in &labels {
            self.run(
                repo,
                &[
                    "label",
                    "create",
                    name,
                    "--color",
                    color,
                    "--description",
                    description,
                    "--force",
                ],
            )
            .await?;
        }
        Ok(())
    }

    async fn run(&self, repo: &Path, args: &[&str]) -> Result<String, GhError> {
        let out = Command::new(&self.program)
            .args(&self.prefix)
            .args(args)
            .current_dir(repo)
            .output()
            .await?;
        if !out.status.success() {
            return Err(GhError::Failed {
                args: args.join(" "),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}
