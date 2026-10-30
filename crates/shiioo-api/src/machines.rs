//! The office's machines: nodes that enrolled and work for it.
//!
//! A node is another `shiioo` process (a DGX Spark, a second Mac, a Linux
//! box) that dials out to the office, takes the runs addressed to it, and
//! drives them exactly as the office's own workers do, appending to the
//! office's log over HTTP (`shiioo-node`). The office knows a machine by
//! its name, labels (`gpu=true`, `os=linux`), what it can do, and when it
//! last called in. Credentials: a one-time enrollment token the owner
//! creates, exchanged by the node for a long-lived machine token; both are
//! stored hashed in `.shiioo/machines.json`. Revoking a machine deletes its
//! record, and its token stops working at once.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::Path;

/// A machine is "online" if it called in within this many milliseconds.
pub const ONLINE_WINDOW_MS: u64 = 60_000;
/// Enrollment tokens are good for an hour.
pub const ENROLLMENT_TTL_MS: u64 = 60 * 60 * 1000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Machine {
    pub name: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub version: String,
    pub created_at: u64,
    #[serde(default)]
    pub last_seen: Option<u64>,
    /// Runs the node said it was driving at its last heartbeat.
    #[serde(default)]
    pub in_flight: Vec<String>,
    /// SHA-256 of the machine token, hex. Never the token.
    pub token_hash: String,
}

impl Machine {
    pub fn online(&self, now_ms: u64) -> bool {
        self.last_seen
            .is_some_and(|seen| now_ms.saturating_sub(seen) <= ONLINE_WINDOW_MS)
    }

    /// Whether `selector` (a name, or `label=value`) picks this machine.
    pub fn matches(&self, selector: &str) -> bool {
        match selector.split_once('=') {
            Some((k, v)) => self.labels.get(k.trim()).is_some_and(|x| x == v.trim()),
            None => self.name == selector.trim(),
        }
    }

    /// One line for `/machines`.
    pub fn describe(&self, now_ms: u64) -> String {
        let state = if self.online(now_ms) { "🟢" } else { "⚪" };
        let labels: Vec<String> = self
            .labels
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        let mut line = format!("{state} {}", self.name);
        if !labels.is_empty() {
            line.push_str(&format!(" [{}]", labels.join(", ")));
        }
        if !self.version.is_empty() {
            line.push_str(&format!(" v{}", self.version));
        }
        if !self.in_flight.is_empty() {
            line.push_str(&format!(", {} run(s)", self.in_flight.len()));
        }
        match self.last_seen {
            Some(seen) if !self.online(now_ms) => {
                let ago = now_ms.saturating_sub(seen) / 1000;
                line.push_str(&format!(", last seen {}", humanize_secs(ago)));
            }
            None => line.push_str(", never connected"),
            _ => {}
        }
        line
    }
}

fn humanize_secs(s: u64) -> String {
    if s < 120 {
        format!("{s}s ago")
    } else if s < 7200 {
        format!("{}m ago", s / 60)
    } else if s < 172_800 {
        format!("{}h ago", s / 3600)
    } else {
        format!("{}d ago", s / 86_400)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Enrollment {
    pub token_hash: String,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
    pub expires_at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Machines {
    #[serde(default)]
    pub machines: BTreeMap<String, Machine>,
    #[serde(default)]
    pub enrollments: Vec<Enrollment>,
}

pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn random_token() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

impl Machines {
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
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
        }
        std::fs::rename(&tmp, path)
    }

    /// A single-use enrollment token with these labels. Returned once, in
    /// the clear; only its hash is kept.
    pub fn create_enrollment(&mut self, labels: BTreeMap<String, String>, now_ms: u64) -> String {
        self.enrollments.retain(|e| e.expires_at > now_ms);
        let token = random_token();
        self.enrollments.push(Enrollment {
            token_hash: token_hash(&token),
            labels,
            expires_at: now_ms + ENROLLMENT_TTL_MS,
        });
        token
    }

    /// Exchanges an enrollment token for a machine token. The enrollment's
    /// labels and the node's are merged (the node's win). A name already in
    /// use is replaced only by a node holding a fresh enrollment: that is
    /// how a reinstalled machine comes back.
    pub fn enroll(
        &mut self,
        enrollment_token: &str,
        name: &str,
        labels: BTreeMap<String, String>,
        capabilities: Vec<String>,
        version: String,
        now_ms: u64,
    ) -> Result<String, String> {
        let name = name.trim();
        if name.is_empty()
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(format!(
                "machine name {name:?}: use letters, digits, '-' and '_'"
            ));
        }
        let hash = token_hash(enrollment_token);
        let index = self
            .enrollments
            .iter()
            .position(|e| e.token_hash == hash && e.expires_at > now_ms)
            .ok_or_else(|| "enrollment token unknown or expired; make a new one".to_owned())?;
        let enrollment = self.enrollments.remove(index);
        let mut merged = enrollment.labels;
        merged.extend(labels);
        let token = random_token();
        self.machines.insert(
            name.to_owned(),
            Machine {
                name: name.to_owned(),
                labels: merged,
                capabilities,
                version,
                created_at: now_ms,
                last_seen: Some(now_ms),
                in_flight: Vec::new(),
                token_hash: token_hash(&token),
            },
        );
        Ok(token)
    }

    /// The machine a token belongs to.
    pub fn authenticate(&self, token: &str) -> Option<&Machine> {
        let hash = token_hash(token);
        self.machines.values().find(|m| m.token_hash == hash)
    }

    pub fn heartbeat(
        &mut self,
        name: &str,
        version: Option<String>,
        capabilities: Option<Vec<String>>,
        in_flight: Vec<String>,
        now_ms: u64,
    ) -> bool {
        let Some(m) = self.machines.get_mut(name) else {
            return false;
        };
        m.last_seen = Some(now_ms);
        if let Some(v) = version {
            m.version = v;
        }
        if let Some(c) = capabilities {
            m.capabilities = c;
        }
        m.in_flight = in_flight;
        true
    }

    pub fn revoke(&mut self, name: &str) -> bool {
        self.machines.remove(name).is_some()
    }

    /// Machines a selector picks, online first.
    pub fn select(&self, selector: &str, now_ms: u64) -> Vec<&Machine> {
        let mut found: Vec<&Machine> = self
            .machines
            .values()
            .filter(|m| m.matches(selector))
            .collect();
        found.sort_by_key(|m| std::cmp::Reverse(m.online(now_ms)));
        found
    }

    pub fn text(&self, now_ms: u64) -> String {
        if self.machines.is_empty() {
            return "No machines enrolled. /enroll <name> gives you a token for `shiioo node enroll`."
                .to_owned();
        }
        self.machines
            .values()
            .map(|m| m.describe(now_ms))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrollment_is_single_use_and_names_are_checked() {
        let mut ms = Machines::default();
        let now = 1_700_000_000_000;
        let labels = BTreeMap::from([("pool".to_owned(), "gpu".to_owned())]);
        let token = ms.create_enrollment(labels, now);
        assert!(
            ms.enroll("bad name", "x y", BTreeMap::new(), vec![], "1".into(), now)
                .is_err()
        );
        let machine_token = ms
            .enroll(
                &token,
                "spark",
                BTreeMap::from([("gpu".to_owned(), "true".to_owned())]),
                vec!["harness.codex".into()],
                "0.1".into(),
                now,
            )
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            ms.enroll(&token, "again", BTreeMap::new(), vec![], "1".into(), now)
                .is_err()
        );
        let m = ms
            .authenticate(&machine_token)
            .unwrap_or_else(|| panic!("no machine"));
        assert_eq!(m.name, "spark");
        assert_eq!(m.labels.get("pool").map(String::as_str), Some("gpu"));
        assert_eq!(m.labels.get("gpu").map(String::as_str), Some("true"));
        assert!(ms.authenticate("nope").is_none());
        assert!(m.matches("spark") && m.matches("gpu=true") && !m.matches("gpu=false"));
        assert!(m.online(now + 10_000) && !m.online(now + ONLINE_WINDOW_MS + 1));
        // An expired enrollment is gone.
        let t2 = ms.create_enrollment(BTreeMap::new(), now);
        assert!(
            ms.enroll(
                &t2,
                "late",
                BTreeMap::new(),
                vec![],
                "1".into(),
                now + ENROLLMENT_TTL_MS + 1
            )
            .is_err()
        );
    }

    #[test]
    fn heartbeat_revoke_and_text() {
        let mut ms = Machines::default();
        let now = 1_700_000_000_000;
        let token = ms.create_enrollment(BTreeMap::new(), now);
        ms.enroll(&token, "spark", BTreeMap::new(), vec![], "0.1".into(), now)
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(ms.heartbeat(
            "spark",
            Some("0.2".into()),
            None,
            vec!["r1".into()],
            now + 5000
        ));
        assert!(!ms.heartbeat("ghost", None, None, vec![], now));
        let text = ms.text(now + 6000);
        assert!(text.starts_with("🟢 spark v0.2, 1 run(s)"), "{text}");
        let later = ms.text(now + 7_300_000);
        assert!(
            later.contains("⚪ spark") && later.contains("last seen 2h ago"),
            "{later}"
        );
        assert!(ms.revoke("spark") && !ms.revoke("spark"));
        assert!(ms.text(now).starts_with("No machines"));
    }

    #[test]
    fn persists() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        let path = dir.path().join("machines.json");
        let mut ms = Machines::default();
        let token = ms.create_enrollment(BTreeMap::new(), 1);
        ms.enroll(&token, "m", BTreeMap::new(), vec![], "1".into(), 1)
            .unwrap_or_else(|e| panic!("{e}"));
        ms.save(&path).unwrap_or_else(|e| panic!("{e}"));
        let again = Machines::load(&path).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(again, ms);
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(!text.contains(&token), "the token must not be stored");
    }
}
