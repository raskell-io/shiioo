//! What the intake has already done, so a restart neither starts a second
//! run for an issue nor forgets to report on one that is still running.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueStatus {
    Running,
    Done,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueRecord {
    pub run_id: String,
    pub status: IssueStatus,
    /// Milliseconds since the Unix epoch.
    pub started_at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    /// Keyed by `<repo label>#<issue number>`.
    #[serde(default)]
    pub issues: BTreeMap<String, IssueRecord>,
}

impl State {
    pub fn key(repo: &str, number: u64) -> String {
        format!("{repo}#{number}")
    }

    /// Loads the file, or starts empty if it doesn't exist. A corrupt file
    /// is an error: silently starting over would re-run every issue.
    pub async fn load(path: &Path) -> Result<Self, std::io::Error> {
        match tokio::fs::read(path).await {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(std::io::Error::other),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    /// Writes to a temporary file next to `path`, then renames it into
    /// place, so a crash mid-write never leaves a half-written file.
    pub async fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, path).await
    }
}
