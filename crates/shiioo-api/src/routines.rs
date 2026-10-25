//! Routines: runs that start themselves on a schedule, and the morning
//! report. The scheduler is one task in the server; what it has already
//! fired is in `.shiioo/routines-state.json`, so a restart neither fires a
//! routine twice nor catches up on everything missed while the office was
//! down (a 07:00 report at 15:00 helps nobody).

use crate::schedule::{self, Schedule};
use chrono::{DateTime, Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Routine {
    pub name: String,
    /// See [`crate::schedule`]: `daily 02:00`, `weekdays 07:00`, `every 6h`, …
    pub schedule: String,
    /// Repository label from `[repos]`.
    #[serde(default)]
    pub repo: Option<String>,
    /// A role from `[[roles]]`, or none.
    #[serde(default)]
    pub role: Option<String>,
    /// An employee name, or `auto`. Ignored when `role` names one.
    #[serde(default = "auto")]
    pub employee: String,
    /// A machine to run on (an enrolled node's name or `label=value`);
    /// None: the office's own machine.
    #[serde(default)]
    pub machine: Option<String>,
    pub task: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn auto() -> String {
    "auto".into()
}

fn yes() -> bool {
    true
}

impl Routine {
    pub fn parsed(&self) -> Result<Schedule, String> {
        schedule::parse(&self.schedule).map_err(|e| format!("routine {}: {e}", self.name))
    }
}

/// What has fired, in milliseconds since the Unix epoch.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RoutineState {
    #[serde(default)]
    pub last_fired: BTreeMap<String, u64>,
    #[serde(default)]
    pub last_report: Option<u64>,
}

impl RoutineState {
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
}

/// Decides whether something scheduled is due now. `anchor` is when it
/// last fired, or when the office started if it never did: nothing is
/// backfilled.
pub fn due(schedule: &Schedule, anchor_ms: u64, now: DateTime<Local>) -> Option<DateTime<Local>> {
    let anchor = ms_to_local(anchor_ms)?;
    let next = schedule.next_after(anchor);
    (now >= next).then_some(next)
}

pub fn ms_to_local(ms: u64) -> Option<DateTime<Local>> {
    Local
        .timestamp_millis_opt(i64::try_from(ms).ok()?)
        .single()
}

pub fn now_ms() -> u64 {
    u64::try_from(Local::now().timestamp_millis()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_round_trips() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("routines-state.json");
        assert_eq!(
            RoutineState::load(&path).unwrap_or_else(|e| panic!("{e}")),
            RoutineState::default()
        );
        let mut state = RoutineState::default();
        state.last_fired.insert("nightly".into(), 1_700_000_000_000);
        state.last_report = Some(1_700_000_100_000);
        state.save(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(RoutineState::load(&path).unwrap_or_else(|e| panic!("{e}")), state);
    }

    #[test]
    fn due_is_strictly_after_the_anchor_and_never_backfills() {
        let every = schedule::parse("every 10m").unwrap_or_else(|e| panic!("{e}"));
        let anchor = now_ms();
        let now = Local::now();
        assert_eq!(due(&every, anchor, now), None);
        let later = now + chrono::Duration::minutes(11);
        assert!(due(&every, anchor, later).is_some());
        // A routine that fired long ago is due once, not once per missed slot.
        let long_ago = anchor - 100 * 24 * 3600 * 1000;
        let next = due(&every, long_ago, now).unwrap_or_else(|| panic!("due"));
        assert!(next < now);
    }

    #[test]
    fn routine_toml_shape() {
        let r: Routine = toml::from_str(
            r#"
            name = "nightly-tests"
            schedule = "daily 02:00"
            repo = "shiioo"
            task = "Run the test suite and fix flaky tests."
            "#,
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert!(r.enabled);
        assert_eq!(r.employee, "auto");
        assert_eq!(r.parsed().map(|s| s.to_string()), Ok("daily 02:00".to_owned()));
    }
}
