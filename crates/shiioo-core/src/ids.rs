use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(pub Uuid);

impl RunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Position of an event in its run's log. The first event is `Seq(0)`.
/// Also used as the SSE `Last-Event-ID`, so clients resume from the log.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Seq(pub u64);

impl Seq {
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// Fencing token. Each lease acquisition gets a strictly larger epoch than
/// the one before; writes carrying an older epoch are rejected. `Epoch(0)`
/// means "written before any lease", which only `RunCreated` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Epoch(pub u64);

impl Epoch {
    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

/// Unix time in milliseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub u64);

impl Timestamp {
    pub fn plus_ms(self, ms: u64) -> Self {
        Self(self.0.saturating_add(ms))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkerId(pub String);

impl From<&str> for WorkerId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

/// Identifies one tool call within a run, so its intent and result pair up.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CallId(pub String);

impl From<&str> for CallId {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}
