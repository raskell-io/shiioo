use shiioo_core::{
    Clock, Epoch, Event, EventStore, Lease, Payload, RunId, Seq, StoreError, WorkerId,
};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard};

struct RunLog {
    events: Vec<Event>,
    /// Latest lease granted. Its epoch is the current epoch, even after
    /// it expires, so stale holders stay fenced.
    lease: Option<Lease>,
}

impl RunLog {
    fn current_epoch(&self) -> Epoch {
        self.lease.as_ref().map_or(Epoch(0), |l| l.epoch)
    }

    fn is_closed(&self) -> bool {
        self.events.last().is_some_and(|e| e.payload.is_terminal())
    }

    fn push(
        &mut self,
        run_id: RunId,
        epoch: Epoch,
        now: shiioo_core::Timestamp,
        payload: Payload,
    ) -> Result<Event, StoreError> {
        let seq = Seq(self.events.len() as u64);
        let prev = self.events.last().map(|e| e.hash);
        let ev = Event::seal(run_id, seq, epoch, now, prev, payload)
            .map_err(|e| StoreError::Backend(e.to_string()))?;
        self.events.push(ev.clone());
        Ok(ev)
    }
}

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
        let mut log = RunLog {
            events: Vec::new(),
            lease: None,
        };
        log.push(run_id, Epoch(0), self.clock.now(), Payload::RunCreated { spec })?;
        self.runs()?.insert(run_id, log);
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
        if log.is_closed() {
            return Err(StoreError::RunClosed(run_id));
        }
        if let Some(held) = &log.lease
            && held.expires_at > now
            && held.worker != worker
        {
            return Err(StoreError::LeaseHeld {
                holder: held.worker.clone(),
                expires_at: held.expires_at,
            });
        }
        // Always a new epoch, even for the same worker id: a restarted worker
        // must not keep writing with assumptions from before its restart.
        let lease = Lease {
            run_id,
            worker: worker.clone(),
            epoch: log.current_epoch().next(),
            expires_at: now.plus_ms(ttl_ms),
        };
        log.push(
            run_id,
            lease.epoch,
            now,
            Payload::LeaseAcquired {
                worker,
                expires_at: lease.expires_at,
            },
        )?;
        log.lease = Some(lease.clone());
        Ok(lease)
    }

    async fn renew_lease(&self, lease: &Lease, ttl_ms: u64) -> Result<Lease, StoreError> {
        let now = self.clock.now();
        let mut runs = self.runs()?;
        let log = runs
            .get_mut(&lease.run_id)
            .ok_or(StoreError::RunNotFound(lease.run_id))?;
        check_lease(log, lease, now)?;
        let renewed = Lease {
            expires_at: now.plus_ms(ttl_ms),
            ..lease.clone()
        };
        log.lease = Some(renewed.clone());
        Ok(renewed)
    }

    async fn append(
        &self,
        lease: &Lease,
        expected_next: Seq,
        payloads: Vec<Payload>,
    ) -> Result<Vec<Event>, StoreError> {
        if payloads
            .iter()
            .any(|p| matches!(p, Payload::RunCreated { .. } | Payload::LeaseAcquired { .. }))
        {
            return Err(StoreError::InvalidPayload(
                "RunCreated and LeaseAcquired are written by the store",
            ));
        }
        if payloads.iter().rev().skip(1).any(Payload::is_terminal) {
            return Err(StoreError::InvalidPayload(
                "a terminal event must be the last in its batch",
            ));
        }

        let now = self.clock.now();
        let mut runs = self.runs()?;
        let log = runs
            .get_mut(&lease.run_id)
            .ok_or(StoreError::RunNotFound(lease.run_id))?;
        check_lease(log, lease, now)?;
        if log.is_closed() {
            return Err(StoreError::RunClosed(lease.run_id));
        }
        let actual = Seq(log.events.len() as u64);
        if expected_next != actual {
            return Err(StoreError::SeqConflict {
                expected: expected_next,
                actual,
            });
        }
        // Validation is done and pushing cannot fail except on encoding, so
        // the batch is effectively atomic; roll back if encoding does fail.
        let mut written = Vec::with_capacity(payloads.len());
        for payload in payloads {
            match log.push(lease.run_id, lease.epoch, now, payload) {
                Ok(ev) => written.push(ev),
                Err(e) => {
                    log.events.truncate(actual.0 as usize);
                    return Err(e);
                }
            }
        }
        Ok(written)
    }

    async fn read(&self, run_id: RunId, from: Seq) -> Result<Vec<Event>, StoreError> {
        let runs = self.runs()?;
        let log = runs.get(&run_id).ok_or(StoreError::RunNotFound(run_id))?;
        Ok(log.events.iter().skip(from.0 as usize).cloned().collect())
    }
}

fn check_lease(
    log: &RunLog,
    lease: &Lease,
    now: shiioo_core::Timestamp,
) -> Result<(), StoreError> {
    let current = log.current_epoch();
    if lease.epoch != current {
        return Err(StoreError::Fenced {
            presented: lease.epoch,
            current,
        });
    }
    match &log.lease {
        Some(held) if held.expires_at > now => Ok(()),
        _ => Err(StoreError::LeaseExpired(lease.epoch)),
    }
}
