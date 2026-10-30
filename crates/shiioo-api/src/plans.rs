//! The chief of staff: a goal becomes a plan, a plan becomes work.
//!
//! `/plan <repo> <goal>` starts a planning run: a harness, in the
//! repository, with read-only tools and instructions to end with a JSON
//! plan. When the run completes, the office parses the plan, keeps it in
//! `.shiioo/plans.json` and tells the owner. `/plan approve <id>` turns
//! each item into a GitHub issue carrying the intake's label, and the
//! intake picks them up one by one. The planner itself changes nothing and
//! needs no approvals; the owner's approval is the plan's.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    pub title: String,
    #[serde(default)]
    pub body: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PlanStatus {
    /// The planning run is going.
    Planning,
    /// Items are ready for the owner's decision.
    Proposed,
    /// Issues were created; the intake takes it from here.
    Approved { issues: Vec<String> },
    /// The planning run failed, or the plan could not be read.
    Failed { reason: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Plan {
    /// The planning run's id; the plan is referred to by its prefix.
    pub run_id: String,
    pub repo: String,
    pub goal: String,
    pub created_at: u64,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub items: Vec<PlanItem>,
    pub status: PlanStatus,
}

/// Every plan the office has made, keyed by run id.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Plans {
    #[serde(default)]
    pub plans: BTreeMap<String, Plan>,
}

impl Plans {
    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(std::io::Error::other),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(
            &tmp,
            serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?,
        )?;
        std::fs::rename(&tmp, path)
    }

    /// A plan by its run id or a unique prefix (4+ characters).
    pub fn find(&self, id_or_prefix: &str) -> Result<&Plan, String> {
        let p = id_or_prefix.trim().to_lowercase();
        if p.len() < 4 {
            return Err("give at least 4 characters of the plan id".into());
        }
        let matches: Vec<&Plan> = self
            .plans
            .values()
            .filter(|plan| plan.run_id.starts_with(&p))
            .collect();
        match matches.as_slice() {
            [one] => Ok(one),
            [] => Err(format!("no plan starts with {p}")),
            _ => Err(format!("{} plans start with {p}", matches.len())),
        }
    }
}

/// Creates issues for an approved plan. Implemented by the server over the
/// `gh` CLI; tests use a fake.
pub trait IssueCreator: Send + Sync {
    /// Returns the issue's URL.
    fn create_issue(
        &self,
        repo: &Path,
        title: &str,
        body: &str,
        label: &str,
    ) -> BoxFuture<'_, Result<String, String>>;
}

/// The planner's instructions, as the run's task.
pub fn planner_task(repo: &str, goal: &str) -> String {
    format!(
        "You are the chief of staff of a one-person company. The owner's goal for the \
         repository \"{repo}\" is:\n\n{goal}\n\n\
         Study the repository first: README, docs, any roadmap, TODO or CHANGELOG files, and \
         the recent git log, so the plan fits how this project works. Then break the goal into \
         work items that one coding agent can each finish in a single session of a few hours: \
         concrete, independently mergeable, in a sensible order, with dependencies between \
         items named in their bodies. Do not change any file; this is planning only. Stick to \
         reading files and to git log, show, diff, status, tag, describe and branch listings: \
         anything else needs the owner's approval and would hold up the plan.\n\n\
         Answer with the plan as a JSON object in a ```json fenced block, as the last thing you \
         print:\n\
         {{\"notes\": \"<two to four sentences on the approach and what you deliberately left \
         out>\", \"items\": [{{\"title\": \"<imperative, under 70 characters>\", \"body\": \
         \"<what to do, how to know it is done, files likely involved, dependencies on other \
         items>\"}}]}}\n\
         Between one and twelve items."
    )
}

/// Tools a planner may use: reading and git history, nothing that writes.
pub fn planner_tools() -> Vec<String> {
    [
        "Read",
        "Glob",
        "Grep",
        "LS",
        "Bash(git log *)",
        "Bash(git log)",
        "Bash(git status*)",
        "Bash(git show *)",
        "Bash(git diff *)",
        "Bash(ls *)",
        "Bash(ls)",
        "Bash(find *)",
        "Bash(cat *)",
        "Bash(head *)",
        "Bash(tail *)",
        "Bash(wc *)",
    ]
    .map(String::from)
    .to_vec()
}

/// Reads the plan out of the planner's final output: the last ```json
/// block, or the whole output if it is JSON. Items need a title.
pub fn parse_plan(output: &str) -> Result<(String, Vec<PlanItem>), String> {
    let candidate = last_json_block(output).unwrap_or_else(|| output.trim().to_owned());
    let value: Value = serde_json::from_str(&candidate)
        .map_err(|e| format!("the planner's answer is not JSON ({e})"))?;
    let (notes, items) = match &value {
        Value::Array(items) => (String::new(), items.clone()),
        Value::Object(map) => (
            map.get("notes")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .trim()
                .to_owned(),
            map.get("items")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| "the plan has no \"items\" list".to_owned())?,
        ),
        _ => return Err("the plan is neither an object nor a list".into()),
    };
    let items: Vec<PlanItem> = items
        .iter()
        .filter_map(|item| {
            let title = item["title"].as_str()?.trim();
            if title.is_empty() {
                return None;
            }
            Some(PlanItem {
                title: title.chars().take(120).collect(),
                body: item["body"].as_str().unwrap_or_default().trim().to_owned(),
            })
        })
        .collect();
    if items.is_empty() {
        return Err("the plan has no items with a title".into());
    }
    if items.len() > 24 {
        return Err(format!(
            "the plan has {} items; that is not a plan",
            items.len()
        ));
    }
    Ok((notes, items))
}

fn last_json_block(text: &str) -> Option<String> {
    let mut last = None;
    let mut rest = text;
    while let Some(start) = rest.find("```json") {
        let after = &rest[start + "```json".len()..];
        let Some(end) = after.find("```") else { break };
        last = Some(after[..end].trim().to_owned());
        rest = &after[end + 3..];
    }
    last
}

/// The issue body for a plan item: the item, where it came from, and the
/// plan's other items for context.
pub fn issue_body(plan: &Plan, index: usize) -> String {
    let item = &plan.items[index];
    let siblings: Vec<String> = plan
        .items
        .iter()
        .enumerate()
        .map(|(i, it)| {
            if i == index {
                format!("{}. **{}** (this issue)", i + 1, it.title)
            } else {
                format!("{}. {}", i + 1, it.title)
            }
        })
        .collect();
    format!(
        "{body}\n\n---\nPart {n} of {total} of a plan made by Shiioo for the goal:\n\n> {goal}\n\n\
         {notes}The plan:\n{siblings}",
        body = if item.body.is_empty() {
            "(no details)".to_owned()
        } else {
            item.body.clone()
        },
        n = index + 1,
        total = plan.items.len(),
        goal = plan.goal.replace('\n', " "),
        notes = if plan.notes.is_empty() {
            String::new()
        } else {
            format!("{}\n\n", plan.notes)
        },
        siblings = siblings.join("\n"),
    )
}

/// The plan as the owner sees it on the phone.
pub fn plan_text(plan: &Plan) -> String {
    let short = &plan.run_id[..8.min(plan.run_id.len())];
    let mut out = format!("📋 plan {short} for {}: {}\n", plan.repo, plan.goal);
    match &plan.status {
        PlanStatus::Planning => out.push_str("still planning…\n"),
        PlanStatus::Proposed => {
            if !plan.notes.is_empty() {
                out.push_str(&plan.notes);
                out.push('\n');
            }
            for (i, item) in plan.items.iter().enumerate() {
                out.push_str(&format!("{}. {}\n", i + 1, item.title));
            }
            out.push_str(&format!(
                "\n/plan approve {short} creates {} issue(s) for the office to work on; \
                 /plan show {short} prints the details.",
                plan.items.len()
            ));
        }
        PlanStatus::Approved { issues } => {
            out.push_str(&format!("approved, {} issue(s):\n", issues.len()));
            for (i, url) in issues.iter().enumerate() {
                let title = plan.items.get(i).map_or("", |it| it.title.as_str());
                out.push_str(&format!("{}. {title}\n   {url}\n", i + 1));
            }
        }
        PlanStatus::Failed { reason } => out.push_str(&format!("failed: {reason}\n")),
    }
    out.trim_end().to_owned()
}

/// The plan with every item's body, for `/plan show`.
pub fn plan_details(plan: &Plan) -> String {
    let mut out = plan_text(plan);
    if matches!(
        plan.status,
        PlanStatus::Proposed | PlanStatus::Approved { .. }
    ) {
        for (i, item) in plan.items.iter().enumerate() {
            out.push_str(&format!("\n\n{}. {}\n{}", i + 1, item.title, item.body));
        }
    }
    out
}

pub fn plans_file(data_dir: &Path) -> PathBuf {
    data_dir.join("plans.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_last_json_block_and_validates_items() {
        let output = "I looked around.\n```json\n{\"draft\": true}\n```\nThen I decided:\n\
                      ```json\n{\"notes\": \"Two steps.\", \"items\": [\
                      {\"title\": \"Add the CLI flag\", \"body\": \"In main.rs\"}, \
                      {\"title\": \"  \"}, {\"title\": \"Document it\"}]}\n```\n";
        let (notes, items) = parse_plan(output).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(notes, "Two steps.");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Add the CLI flag");
        assert_eq!(items[0].body, "In main.rs");
        assert_eq!(items[1].body, "");
        // A bare list is accepted too.
        let (_, items) = parse_plan("[{\"title\": \"x\"}]").unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(items.len(), 1);
        assert!(parse_plan("no plan here").is_err());
        assert!(parse_plan("{\"items\": []}").is_err());
    }

    #[test]
    fn plan_text_and_issue_bodies() {
        let plan = Plan {
            run_id: "abcdef12-0000".into(),
            repo: "shiioo".into(),
            goal: "Ship v0.3".into(),
            created_at: 0,
            notes: "Keep it small.".into(),
            items: vec![
                PlanItem {
                    title: "Write the changelog".into(),
                    body: "From the git log.".into(),
                },
                PlanItem {
                    title: "Tag the release".into(),
                    body: String::new(),
                },
            ],
            status: PlanStatus::Proposed,
        };
        let text = plan_text(&plan);
        assert!(text.starts_with("📋 plan abcdef12 for shiioo: Ship v0.3"));
        assert!(text.contains("1. Write the changelog\n2. Tag the release"));
        assert!(text.contains("/plan approve abcdef12 creates 2 issue(s)"));
        let body = issue_body(&plan, 1);
        assert!(body.starts_with("(no details)"));
        assert!(body.contains("Part 2 of 2"));
        assert!(body.contains("2. **Tag the release** (this issue)"));
        assert!(body.contains("> Ship v0.3"));
        let mut plans = Plans::default();
        plans.plans.insert(plan.run_id.clone(), plan);
        assert!(plans.find("abcd").is_ok());
        assert!(plans.find("abc").is_err());
        assert!(plans.find("zzzz").is_err());
    }
}
