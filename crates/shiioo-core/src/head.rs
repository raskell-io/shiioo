use crate::event::{Event, EventHash, Payload};
use crate::ids::{Epoch, RunId, Seq, Timestamp, WorkerId};
use crate::lease::Lease;
use crate::store::StoreError;
use serde::{Deserialize, Serialize};

/// Everything a backend needs to know about a run to accept or reject a
/// write. Backends load it (under a lock or row lock), call one method, and
/// persist the returned events plus the updated head in one transaction.
/// Keeping the rules here means every backend enforces exactly the same ones.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunHead {
    pub run_id: RunId,
    pub next_seq: Seq,
    pub last_hash: Option<EventHash>,
    pub closed: bool,
    /// Latest lease granted. Its epoch stays current after it expires, so
    /// stale holders remain fenced until someone takes over.
    pub lease: Option<Lease>,
}

impl RunHead {
    /// Starts a run: its head and the `RunCreated` event at `Seq(0)`.
    pub fn create(
        run_id: RunId,
        spec: serde_json::Value,
        now: Timestamp,
    ) -> Result<(Self, Event), StoreError> {
        let mut head = Self {
            run_id,
            next_seq: Seq(0),
            last_hash: None,
            closed: false,
            lease: None,
        };
        let ev = head.seal(Epoch(0), now, Payload::RunCreated { spec })?;
        Ok((head, ev))
    }

    pub fn current_epoch(&self) -> Epoch {
        self.lease.as_ref().map_or(Epoch(0), |l| l.epoch)
    }

    pub fn check_lease(&self, lease: &Lease, now: Timestamp) -> Result<(), StoreError> {
        let current = self.current_epoch();
        if lease.run_id != self.run_id || lease.epoch != current {
            return Err(StoreError::Fenced {
                presented: lease.epoch,
                current,
            });
        }
        match &self.lease {
            Some(held) if held.expires_at > now => Ok(()),
            _ => Err(StoreError::LeaseExpired(lease.epoch)),
        }
    }

    /// Grants a lease with a new epoch and returns the `LeaseAcquired` event
    /// to persist. Always a new epoch, even for the same worker id: a
    /// restarted worker must not keep writing on assumptions from before.
    pub fn grant_lease(
        &mut self,
        worker: WorkerId,
        ttl_ms: u64,
        now: Timestamp,
    ) -> Result<(Lease, Event), StoreError> {
        if self.closed {
            return Err(StoreError::RunClosed(self.run_id));
        }
        if let Some(held) = &self.lease
            && held.expires_at > now
            && held.worker != worker
        {
            return Err(StoreError::LeaseHeld {
                holder: held.worker.clone(),
                expires_at: held.expires_at,
            });
        }
        let lease = Lease {
            run_id: self.run_id,
            worker: worker.clone(),
            epoch: self.current_epoch().next(),
            expires_at: now.plus_ms(ttl_ms),
        };
        let ev = self.seal(
            lease.epoch,
            now,
            Payload::LeaseAcquired {
                worker,
                expires_at: lease.expires_at,
            },
        )?;
        self.lease = Some(lease.clone());
        Ok((lease, ev))
    }

    pub fn renew_lease(
        &mut self,
        lease: &Lease,
        ttl_ms: u64,
        now: Timestamp,
    ) -> Result<Lease, StoreError> {
        self.check_lease(lease, now)?;
        let renewed = Lease {
            expires_at: now.plus_ms(ttl_ms),
            ..lease.clone()
        };
        self.lease = Some(renewed.clone());
        Ok(renewed)
    }

    /// Validates and seals a batch. On error the head is left unchanged.
    pub fn append(
        &mut self,
        lease: &Lease,
        expected_next: Seq,
        payloads: Vec<Payload>,
        now: Timestamp,
    ) -> Result<Vec<Event>, StoreError> {
        validate_batch(&payloads)?;
        self.check_lease(lease, now)?;
        if self.closed {
            return Err(StoreError::RunClosed(self.run_id));
        }
        if expected_next != self.next_seq {
            return Err(StoreError::SeqConflict {
                expected: expected_next,
                actual: self.next_seq,
            });
        }
        let mut next = self.clone();
        let events = payloads
            .into_iter()
            .map(|p| next.seal(lease.epoch, now, p))
            .collect::<Result<Vec<_>, _>>()?;
        *self = next;
        Ok(events)
    }

    fn seal(
        &mut self,
        epoch: Epoch,
        now: Timestamp,
        payload: Payload,
    ) -> Result<Event, StoreError> {
        let terminal = payload.is_terminal();
        let ev = Event::seal(
            self.run_id,
            self.next_seq,
            epoch,
            now,
            self.last_hash,
            payload,
        )
        .map_err(|e| StoreError::Backend(format!("encoding event: {e}")))?;
        self.next_seq = self.next_seq.next();
        self.last_hash = Some(ev.hash);
        self.closed |= terminal;
        Ok(ev)
    }
}

fn validate_batch(payloads: &[Payload]) -> Result<(), StoreError> {
    if payloads.iter().any(|p| {
        matches!(
            p,
            Payload::RunCreated { .. } | Payload::LeaseAcquired { .. }
        )
    }) {
        return Err(StoreError::InvalidPayload(
            "RunCreated and LeaseAcquired are written by the store",
        ));
    }
    if payloads.iter().rev().skip(1).any(Payload::is_terminal) {
        return Err(StoreError::InvalidPayload(
            "a terminal event must be the last in its batch",
        ));
    }
    Ok(())
}
