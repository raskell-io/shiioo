use shiioo_core::{Event, EventStore, Lease, Payload, RunId, Seq, StoreError, WorkerId};
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::{broadcast, watch};

/// How many run ids the office-wide channel buffers before a slow
/// subscriber lags.
const OFFICE_CHANNEL: usize = 1024;

/// Wraps a store and announces every successful write to a run, for live
/// streams in the same process.
///
/// The announcement is only a wake-up hint: subscribers always read the
/// events themselves from the log, so a missed or coalesced notification
/// can delay a stream but never lose or reorder events. In a cluster the
/// same hint comes from Postgres `LISTEN/NOTIFY` instead.
pub struct Notifying<S> {
    inner: S,
    heads: Mutex<HashMap<RunId, watch::Sender<u64>>>,
    all: broadcast::Sender<RunId>,
}

impl<S> Notifying<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            heads: Mutex::new(HashMap::new()),
            all: broadcast::channel(OFFICE_CHANNEL).0,
        }
    }

    pub fn inner(&self) -> &S {
        &self.inner
    }

    /// A receiver that changes whenever the run gets new events. Its value
    /// is a counter of writes seen since the first subscription; only the
    /// change matters.
    pub fn subscribe(&self, run_id: RunId) -> watch::Receiver<u64> {
        let mut heads = self.heads.lock().unwrap_or_else(|p| p.into_inner());
        heads
            .entry(run_id)
            .or_insert_with(|| watch::channel(0).0)
            .subscribe()
    }

    /// The id of every run that gets new events (or is created), across
    /// the office. A receiver that falls behind gets `Lagged` and should
    /// rescan; only writes through this process are seen.
    pub fn subscribe_all(&self) -> broadcast::Receiver<RunId> {
        self.all.subscribe()
    }

    fn publish(&self, run_id: RunId, finished: bool) {
        // Nobody listening is not an error.
        let _ = self.all.send(run_id);
        let mut heads = self.heads.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(tx) = heads.get(&run_id) {
            tx.send_modify(|writes| *writes += 1);
            // Forget runs nobody listens to, and finished runs.
            if tx.receiver_count() == 0 || finished {
                heads.remove(&run_id);
            }
        }
    }
}

impl<S: EventStore> EventStore for Notifying<S> {
    async fn create_run(&self, spec: serde_json::Value) -> Result<RunId, StoreError> {
        let run_id = self.inner.create_run(spec).await?;
        self.publish(run_id, false);
        Ok(run_id)
    }

    async fn acquire_lease(
        &self,
        run_id: RunId,
        worker: WorkerId,
        ttl_ms: u64,
    ) -> Result<Lease, StoreError> {
        let lease = self.inner.acquire_lease(run_id, worker, ttl_ms).await?;
        // The grant wrote a LeaseAcquired event.
        self.publish(run_id, false);
        Ok(lease)
    }

    async fn renew_lease(&self, lease: &Lease, ttl_ms: u64) -> Result<Lease, StoreError> {
        self.inner.renew_lease(lease, ttl_ms).await
    }

    async fn append(
        &self,
        lease: &Lease,
        expected_next: Seq,
        payloads: Vec<Payload>,
    ) -> Result<Vec<Event>, StoreError> {
        let events = self.inner.append(lease, expected_next, payloads).await?;
        self.publish(
            lease.run_id,
            events.last().is_some_and(|e| e.payload.is_terminal()),
        );
        Ok(events)
    }

    async fn append_external(
        &self,
        run_id: RunId,
        payloads: Vec<Payload>,
    ) -> Result<Vec<Event>, StoreError> {
        let events = self.inner.append_external(run_id, payloads).await?;
        self.publish(run_id, false);
        Ok(events)
    }

    async fn list_runs(&self) -> Result<Vec<RunId>, StoreError> {
        self.inner.list_runs().await
    }

    async fn read(&self, run_id: RunId, from: Seq) -> Result<Vec<Event>, StoreError> {
        self.inner.read(run_id, from).await
    }
}
