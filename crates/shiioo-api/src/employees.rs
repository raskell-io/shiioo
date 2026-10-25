//! Who takes a new task: the capacity broker.
//!
//! Employees are tried in the owner's order. An employee whose provider's
//! rate-limit windows are above its thresholds is skipped; a provider with
//! no observation yet is assumed available. The owner's subscriptions, not
//! money, are the scarce resource (docs/positioning.md).

use crate::CapacityTracker;
use serde::{Deserialize, Serialize};

/// What kind of employee, and which provider's windows gate it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum EmployeeKind {
    /// Claude Code on the Claude subscription. Provider key `claude-code`.
    ClaudeCode,
    /// Codex on the OpenAI subscription. Provider key `codex`.
    Codex,
    /// Shiioo's native loop against a model, for example `ollama/qwen3:32b`.
    /// Local models have no window; the key is the model's prefix.
    Native { model: String },
}

impl EmployeeKind {
    /// The provider whose `CapacityObserved` windows gate this employee.
    pub fn provider_key(&self) -> String {
        match self {
            EmployeeKind::ClaudeCode => "claude-code".into(),
            EmployeeKind::Codex => "codex".into(),
            EmployeeKind::Native { model } => model
                .split_once('/')
                .map_or_else(|| model.clone(), |(prefix, _)| prefix.to_owned()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Employee {
    pub name: String,
    #[serde(flatten)]
    pub kind: EmployeeKind,
    /// Skip this employee once the provider's five-hour window is above
    /// this fraction (0.0–1.0).
    #[serde(default = "default_five_hour")]
    pub max_five_hour: f64,
    /// Skip this employee once the seven-day window is above this fraction.
    #[serde(default = "default_seven_day")]
    pub max_seven_day: f64,
}

fn default_five_hour() -> f64 {
    0.95
}

fn default_seven_day() -> f64 {
    0.90
}

/// Why an employee was skipped.
#[derive(Clone, Debug, PartialEq)]
pub struct Skipped {
    pub name: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub employee: Employee,
    pub skipped: Vec<Skipped>,
}

/// Chooses the first employee whose provider has room.
pub fn pick(employees: &[Employee], capacity: &CapacityTracker) -> Result<Pick, Vec<Skipped>> {
    let mut skipped = Vec::new();
    for e in employees {
        match room(e, capacity) {
            Ok(()) => {
                return Ok(Pick {
                    employee: e.clone(),
                    skipped,
                });
            }
            Err(reason) => skipped.push(Skipped {
                name: e.name.clone(),
                reason,
            }),
        }
    }
    Err(skipped)
}

fn room(e: &Employee, capacity: &CapacityTracker) -> Result<(), String> {
    let Some(latest) = capacity.provider(&e.kind.provider_key()) else {
        return Ok(());
    };
    for (window, limit) in [
        ("five_hour", e.max_five_hour),
        ("seven_day", e.max_seven_day),
    ] {
        if let Some(w) = latest.windows.get(window)
            && w.utilization > limit
        {
            return Err(format!(
                "{window} window at {:.0}% (limit {:.0}%)",
                w.utilization * 100.0,
                limit * 100.0
            ));
        }
    }
    Ok(())
}

/// The default staff: Claude Code first, Codex second.
pub fn default_employees() -> Vec<Employee> {
    vec![
        Employee {
            name: "claude".into(),
            kind: EmployeeKind::ClaudeCode,
            max_five_hour: default_five_hour(),
            max_seven_day: default_seven_day(),
        },
        Employee {
            name: "codex".into(),
            kind: EmployeeKind::Codex,
            max_five_hour: default_five_hour(),
            max_seven_day: default_seven_day(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tracker_with(provider: &str, five_hour: f64, seven_day: f64) -> CapacityTracker {
        let t = CapacityTracker::default();
        t.observe(
            provider,
            &json!({
                "five_hour": { "utilization": five_hour, "resetsAt": 1_800_000_000 },
                "seven_day": { "utilization": seven_day, "resetsAt": 1_800_500_000 },
            }),
            1,
        );
        t
    }

    #[test]
    fn unknown_capacity_means_available() {
        let pick = pick(&default_employees(), &CapacityTracker::default());
        assert_eq!(pick.map(|p| p.employee.name), Ok("claude".to_owned()));
    }

    #[test]
    fn exhausted_week_falls_through_to_the_next_employee() {
        let t = tracker_with("claude-code", 0.2, 0.94);
        let pick = pick(&default_employees(), &t).unwrap_or_else(|s| panic!("{s:?}"));
        assert_eq!(pick.employee.name, "codex");
        assert_eq!(pick.skipped.len(), 1);
        assert!(pick.skipped[0].reason.contains("seven_day window at 94%"));
    }

    #[test]
    fn everyone_exhausted_reports_why() {
        let t = tracker_with("claude-code", 0.99, 0.5);
        t.observe("codex", &json!({ "five_hour": { "utilization": 0.97 } }), 2);
        let err = pick(&default_employees(), &t).err().unwrap_or_default();
        assert_eq!(err.len(), 2);
    }

    #[test]
    fn thresholds_are_per_employee_and_configurable() {
        let t = tracker_with("claude-code", 0.0, 0.94);
        let lenient = vec![Employee {
            name: "claude-lenient".into(),
            kind: EmployeeKind::ClaudeCode,
            max_five_hour: 1.0,
            max_seven_day: 1.0,
        }];
        assert!(pick(&lenient, &t).is_ok());
    }

    #[test]
    fn native_employees_key_on_the_model_prefix() {
        let local = EmployeeKind::Native {
            model: "ollama/qwen3:32b".into(),
        };
        assert_eq!(local.provider_key(), "ollama");
    }

    #[test]
    fn toml_shape() {
        let text = r#"
            [[employees]]
            name = "claude"
            kind = "claude-code"
            max_seven_day = 0.85

            [[employees]]
            name = "local"
            kind = "native"
            model = "ollama/qwen3:32b"
        "#;
        #[derive(Deserialize)]
        struct Doc {
            employees: Vec<Employee>,
        }
        let doc: Doc = toml::from_str(text).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(doc.employees[0].max_seven_day, 0.85);
        assert_eq!(doc.employees[0].max_five_hour, 0.95);
        assert_eq!(
            doc.employees[1].kind,
            EmployeeKind::Native {
                model: "ollama/qwen3:32b".into()
            }
        );
    }

    #[test]
    fn tracker_keeps_the_newest_observation_and_formats_text() {
        let t = tracker_with("claude-code", 0.2, 0.5);
        t.observe(
            "claude-code",
            &json!({ "five_hour": { "utilization": 0.1 } }),
            0,
        );
        let latest = t
            .provider("claude-code")
            .unwrap_or_else(|| panic!("missing"));
        assert_eq!(
            latest.windows["five_hour"].utilization, 0.2,
            "older observation ignored"
        );
        t.observe(
            "claude-code",
            &json!({ "five_hour": { "utilization": 0.3 } }),
            5,
        );
        let latest = t
            .provider("claude-code")
            .unwrap_or_else(|| panic!("missing"));
        assert_eq!(latest.windows["five_hour"].utilization, 0.3);
        assert!(t.text().starts_with("claude-code: 5h 30%"), "{}", t.text());
        assert!(
            CapacityTracker::default()
                .text()
                .contains("No capacity data")
        );
    }
}
