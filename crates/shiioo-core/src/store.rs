use crate::event::{Event, Payload};
use crate::ids::{Epoch, RunId, Seq, Timestamp, WorkerId};
use crate::lease::Lease;
use serde_json::Value;
use std::future::Future;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum StoreError {
    #[error("run {0} not found")]
    RunNotFound(RunId),
    #[error("run {0} has finished; nothing more can be appended")]
    RunClosed(RunId),
    #[error("lease is held by {holder:?} until {expires_at:?}")]
    LeaseHeld {
        holder: WorkerId,
        expires_at: Timestamp,
    },
    /// The presented lease was superseded. The worker must stop: someone
    /// else owns the run now.
    #[error("fenced: presented epoch {presented:?}, current is {current:?}")]
    Fenced { presented: Epoch, current: Epoch },
    /// The lease ran out. The worker must re-acquire (getting a new epoch)
    /// and re-read the log before writing again.
    #[error("lease for epoch {0:?} has expired")]
    LeaseExpired(Epoch),
    /// Another write landed first, or the worker's view of the log is stale.
    #[error("expected next seq {expected:?}, log is at {actual:?}")]
    SeqConflict { expected: Seq, actual: Seq },
    #[error("payload not allowed here: {0}")]
    InvalidPayload(&'static str),
    #[error("backend error: {0}")]
    Backend(String),
}

impl StoreError {
    /// A JSON form that survives a trip over HTTP (`shiioo-node`'s remote
    /// store speaks it), so a node worker sees the same fencing errors a
    /// local one does.
    pub fn to_json(&self) -> Value {
        use serde_json::json;
        match self {
            StoreError::RunNotFound(run) => json!({ "kind": "run_not_found", "run": run }),
            StoreError::RunClosed(run) => json!({ "kind": "run_closed", "run": run }),
            StoreError::LeaseHeld { holder, expires_at } => {
                json!({ "kind": "lease_held", "holder": holder, "expires_at": expires_at })
            }
            StoreError::Fenced { presented, current } => {
                json!({ "kind": "fenced", "presented": presented, "current": current })
            }
            StoreError::LeaseExpired(epoch) => json!({ "kind": "lease_expired", "epoch": epoch }),
            StoreError::SeqConflict { expected, actual } => {
                json!({ "kind": "seq_conflict", "expected": expected, "actual": actual })
            }
            StoreError::InvalidPayload(detail) => {
                json!({ "kind": "invalid_payload", "detail": detail })
            }
            StoreError::Backend(detail) => json!({ "kind": "backend", "detail": detail }),
        }
    }

    /// The inverse of [`to_json`](Self::to_json). `invalid_payload` comes
    /// back as `Backend` (its detail is not a static string).
    pub fn from_json(v: &Value) -> Option<Self> {
        fn field<T: serde::de::DeserializeOwned>(v: &Value, k: &str) -> Option<T> {
            serde_json::from_value(v.get(k)?.clone()).ok()
        }
        let get = |k: &str| v.get(k).cloned();
        Some(match v.get("kind")?.as_str()? {
            "run_not_found" => StoreError::RunNotFound(field(v, "run")?),
            "run_closed" => StoreError::RunClosed(field(v, "run")?),
            "lease_held" => StoreError::LeaseHeld {
                holder: field(v, "holder")?,
                expires_at: field(v, "expires_at")?,
            },
            "fenced" => StoreError::Fenced {
                presented: field(v, "presented")?,
                current: field(v, "current")?,
            },
            "lease_expired" => StoreError::LeaseExpired(field(v, "epoch")?),
            "seq_conflict" => StoreError::SeqConflict {
                expected: field(v, "expected")?,
                actual: field(v, "actual")?,
            },
            "invalid_payload" => StoreError::Backend(format!(
                "invalid payload: {}",
                get("detail").and_then(|d| d.as_str().map(str::to_owned)).unwrap_or_default()
            )),
            "backend" => StoreError::Backend(
                get("detail").and_then(|d| d.as_str().map(str::to_owned)).unwrap_or_default(),
            ),
            _ => return None,
        })
    }
}

/// Durable, per-run, append-only log with leases. All backends must pass the
/// same conformance tests (`shiioo-store/tests`).
pub trait EventStore: Send + Sync {
    fn create_run(&self, spec: Value) -> impl Future<Output = Result<RunId, StoreError>> + Send;

    /// Grants a lease with a fresh epoch, fencing off every earlier holder,
    /// and records `LeaseAcquired` in the log. Fails while another worker
    /// holds an unexpired lease.
    fn acquire_lease(
        &self,
        run_id: RunId,
        worker: WorkerId,
        ttl_ms: u64,
    ) -> impl Future<Output = Result<Lease, StoreError>> + Send;

    /// Extends a still-valid lease without changing its epoch.
    fn renew_lease(
        &self,
        lease: &Lease,
        ttl_ms: u64,
    ) -> impl Future<Output = Result<Lease, StoreError>> + Send;

    /// Appends atomically: all payloads or none. `expected_next` is the seq
    /// the first new event gets; it must equal the current log length.
    fn append(
        &self,
        lease: &Lease,
        expected_next: Seq,
        payloads: Vec<Payload>,
    ) -> impl Future<Output = Result<Vec<Event>, StoreError>> + Send;

    /// Appends external input (owner messages, decisions, cancellation
    /// requests) without a lease. Rejects execution payloads and closed
    /// runs. See `RunHead::append_external`.
    fn append_external(
        &self,
        run_id: RunId,
        payloads: Vec<Payload>,
    ) -> impl Future<Output = Result<Vec<Event>, StoreError>> + Send;

    fn read(
        &self,
        run_id: RunId,
        from: Seq,
    ) -> impl Future<Output = Result<Vec<Event>, StoreError>> + Send;

    /// Every run in the store, in no particular order. A projection (run
    /// list with status and times) is the long-term answer; this serves a
    /// personal office with hundreds of runs well enough.
    fn list_runs(&self) -> impl Future<Output = Result<Vec<RunId>, StoreError>> + Send;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_errors_round_trip_through_json() {
        let run = RunId::new();
        for err in [
            StoreError::RunNotFound(run),
            StoreError::RunClosed(run),
            StoreError::LeaseHeld {
                holder: WorkerId("w".into()),
                expires_at: Timestamp(5),
            },
            StoreError::Fenced {
                presented: Epoch(1),
                current: Epoch(2),
            },
            StoreError::LeaseExpired(Epoch(3)),
            StoreError::SeqConflict {
                expected: Seq(4),
                actual: Seq(9),
            },
            StoreError::Backend("x".into()),
        ] {
            assert_eq!(StoreError::from_json(&err.to_json()), Some(err));
        }
        assert_eq!(
            StoreError::from_json(&StoreError::InvalidPayload("bad").to_json()),
            Some(StoreError::Backend("invalid payload: bad".into()))
        );
        assert_eq!(StoreError::from_json(&serde_json::json!({ "kind": "?" })), None);
    }
}
