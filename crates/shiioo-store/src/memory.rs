use shiioo_core::{
    Clock, Event, EventStore, Lease, Payload, RunHead, RunId, Seq, StoreError, WorkerId,
};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

struct RunLog {
    head: RunHead,
    events: Vec<Event>,
}

/// Reference backend: one mutex over all runs. Used in tests and simulation.
pub struct MemoryStore<C> {
    clock: C,
    runs: Mutex<HashMap<RunId, RunLog>>,
}

impl<C: Clock> MemoryStore<C> {
    pub fn new(clock: C) -> Self {
        Self {
            clock,
            runs: Mutex::new(HashMap::new()),
        }
    }

    fn runs(&self) -> Result<MutexGuard<'_, HashMap<RunId, RunLog>>, StoreError> {
        self.runs
            .lock()
            .map_err(|_| StoreError::Backend("store mutex poisoned".into()))
    }
}

impl<C: Clock> EventStore for MemoryStore<C> {
    async fn create_run(&self, spec: serde_json::Value) -> Result<RunId, StoreError> {
        let run_id = RunId::new();
        let (head, ev) = RunHead::create(run_id, spec, self.clock.now())?;
        self.runs()?.insert(
            run_id,
            RunLog {
                head,
                events: vec![ev],
            },
        );
        Ok(run_id)
    }

    async fn acquire_lease(
        &self,
        run_id: RunId,
        worker: WorkerId,
        ttl_ms: u64,
    ) -> Result<Lease, StoreError> {
        let now = self.clock.now();
        let mut runs = self.runs()?;
        let log = runs
            .get_mut(&run_id)
            .ok_or(StoreError::RunNotFound(run_id))?;
        let (lease, ev) = log.head.grant_lease(worker, ttl_ms, now)?;
        log.events.push(ev);
        Ok(lease)
    }

    async fn renew_lease(&self, lease: &Lease, ttl_ms: u64) -> Result<Lease, StoreError> {
        let now = self.clock.now();
        let mut runs = self.runs()?;
        let log = runs
            .get_mut(&lease.run_id)
            .ok_or(StoreError::RunNotFound(lease.run_id))?;
        log.head.renew_lease(lease, ttl_ms, now)
    }

    async fn append(
        &self,
        lease: &Lease,
        expected_next: Seq,
        payloads: Vec<Payload>,
    ) -> Result<Vec<Event>, StoreError> {
        let now = self.clock.now();
        let mut runs = self.runs()?;
        let log = runs
            .get_mut(&lease.run_id)
            .ok_or(StoreError::RunNotFound(lease.run_id))?;
        let events = log.head.append(lease, expected_next, payloads, now)?;
        log.events.extend(events.iter().cloned());
        Ok(events)
    }

    async fn append_external(
        &self,
        run_id: RunId,
        payloads: Vec<Payload>,
    ) -> Result<Vec<Event>, StoreError> {
        let now = self.clock.now();
        let mut runs = self.runs()?;
        let log = runs
            .get_mut(&run_id)
            .ok_or(StoreError::RunNotFound(run_id))?;
        let events = log.head.append_external(payloads, now)?;
        log.events.extend(events.iter().cloned());
        Ok(events)
    }

    async fn list_runs(&self) -> Result<Vec<RunId>, StoreError> {
        Ok(self.runs()?.keys().copied().collect())
    }

    async fn read(&self, run_id: RunId, from: Seq) -> Result<Vec<Event>, StoreError> {
        let runs = self.runs()?;
        let log = runs.get(&run_id).ok_or(StoreError::RunNotFound(run_id))?;
        Ok(log.events.iter().skip(from.0 as usize).cloned().collect())
    }
}
