//! Which isolation level a new run gets.
//!
//! The owner sets a default level and overrides per repository label in
//! `config.toml`; a request may name a level explicitly. The settings of
//! each level (image, limits, egress) are office-wide.

use serde::{Deserialize, Serialize};
use shiioo_runtime::{ContainerSpec, Isolation, OpenshellSpec};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct IsolationPolicy {
    /// Level for runs without a repo-specific setting: `worktree` (the
    /// default when empty), `container` or `openshell`.
    pub default: String,
    /// Repository label → level.
    pub repos: BTreeMap<String, String>,
    pub container: ContainerSpec,
    pub openshell: OpenshellSpec,
}

impl IsolationPolicy {
    /// The level for a run on `repo` (a configured label), or the one
    /// `requested` by name.
    pub fn resolve(&self, repo: Option<&str>, requested: Option<&str>) -> Result<Isolation, String> {
        let level = requested
            .or_else(|| repo.and_then(|r| self.repos.get(r).map(String::as_str)))
            .unwrap_or(if self.default.is_empty() {
                "worktree"
            } else {
                self.default.as_str()
            });
        match level {
            "worktree" => Ok(Isolation::Worktree),
            "container" => Ok(Isolation::Container(self.container.clone())),
            "openshell" => Ok(Isolation::Openshell(self.openshell.clone())),
            other => Err(format!(
                "unknown isolation level {other}; use worktree, container or openshell"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_beats_repo_beats_default() {
        let mut policy = IsolationPolicy {
            default: "container".into(),
            ..IsolationPolicy::default()
        };
        policy.repos.insert("trusted".into(), "worktree".into());
        policy.container.memory = Some("8g".into());
        assert_eq!(
            policy.resolve(None, None),
            Ok(Isolation::Container(policy.container.clone()))
        );
        assert_eq!(policy.resolve(Some("trusted"), None), Ok(Isolation::Worktree));
        assert!(matches!(
            policy.resolve(Some("trusted"), Some("openshell")),
            Ok(Isolation::Openshell(_))
        ));
        assert!(policy.resolve(None, Some("vm")).is_err());
        assert_eq!(IsolationPolicy::default().resolve(None, None), Ok(Isolation::Worktree));
    }
}
