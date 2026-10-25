//! Roles: reusable agent configurations the owner names in a run
//! (`/run @reviewer …`, `--role reviewer`, a routine's `role`). The
//! glossary's "job title": instructions, tools, a model, an isolation
//! level and limits, applied on top of the employee that does the work.

use serde::{Deserialize, Serialize};
use shiioo_runtime::{AgentSpec, Runtime};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Role {
    pub name: String,
    /// Which employee does this role's work: a configured employee name,
    /// or `auto` (the capacity broker chooses).
    #[serde(default = "auto")]
    pub employee: String,
    /// Instructions added to every task given to this role.
    #[serde(default)]
    pub system: String,
    /// The harness's tool allow list (Claude Code syntax); empty keeps the
    /// employee's default.
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// `worktree`, `container` or `openshell`; None follows the office's policy.
    #[serde(default)]
    pub isolation: Option<String>,
    #[serde(default)]
    pub max_minutes: Option<u64>,
    #[serde(default)]
    pub max_turns: Option<u32>,
}

fn auto() -> String {
    "auto".into()
}

impl Role {
    /// Lays the role over what the employee was assigned.
    pub fn apply(&self, runtime: &mut Runtime, agent: &mut AgentSpec) {
        if !self.system.is_empty() {
            agent.system = if agent.system.is_empty() {
                self.system.clone()
            } else {
                format!("{}\n\n{}", self.system, agent.system)
            };
        }
        if !self.tools.is_empty() {
            agent.tools = self.tools.clone();
        }
        if let Some(model) = &self.model {
            agent.model = model.clone();
        }
        if let Some(turns) = self.max_turns {
            agent.max_turns = turns;
        }
        if let (Some(minutes), Runtime::Harness(h)) = (self.max_minutes, runtime) {
            h.max_minutes = Some(minutes);
        }
    }

    /// One line for `/roles`.
    pub fn describe(&self) -> String {
        let mut parts = vec![format!("employee {}", self.employee)];
        if let Some(level) = &self.isolation {
            parts.push(level.clone());
        }
        if let Some(m) = self.max_minutes {
            parts.push(format!("{m} min"));
        }
        if let Some(model) = &self.model {
            parts.push(model.clone());
        }
        let first_line = self.system.lines().next().unwrap_or_default();
        let gist: String = first_line.chars().take(80).collect();
        if gist.is_empty() {
            format!("@{} ({})", self.name, parts.join(", "))
        } else {
            format!("@{} ({}): {gist}", self.name, parts.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shiioo_runtime::HarnessSpec;

    #[test]
    fn applies_over_the_employee_defaults() {
        let role: Role = toml::from_str(
            r#"
            name = "reviewer"
            system = "Review only. Never push."
            tools = ["Read", "Grep"]
            isolation = "container"
            max_minutes = 30
            "#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(role.employee, "auto");
        let mut runtime = Runtime::Harness(HarnessSpec::claude_code());
        let mut agent = AgentSpec {
            model: String::new(),
            system: "Be brief.".into(),
            tools: vec!["Bash".into()],
            max_turns: 50,
        };
        role.apply(&mut runtime, &mut agent);
        assert_eq!(agent.system, "Review only. Never push.\n\nBe brief.");
        assert_eq!(agent.tools, vec!["Read", "Grep"]);
        assert_eq!(agent.max_turns, 50);
        match runtime {
            Runtime::Harness(h) => assert_eq!(h.max_minutes, Some(30)),
            Runtime::Native => panic!("runtime changed kind"),
        }
        assert_eq!(
            role.describe(),
            "@reviewer (employee auto, container, 30 min): Review only. Never push."
        );
    }
}
