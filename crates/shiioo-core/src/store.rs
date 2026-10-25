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
