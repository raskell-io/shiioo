use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition, WriteTransaction};
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use shiioo_core::{
    Clock, Epoch, Event, EventHash, EventStore, Lease, Payload, RunHead, RunId, Seq, StoreError,
    SystemClock, Timestamp, WorkerId,
};
use std::path::Path;
use std::sync::Arc;

/// Run heads, keyed by run id. Value: JSON-encoded `RunHead`.
const HEADS: TableDefinition<&[u8; 16], &[u8]> = TableDefinition::new("run_heads");
/// Events, keyed by (run id, seq) so a run's log is one contiguous range.
const EVENTS: TableDefinition<(&[u8; 16], u64), &[u8]> = TableDefinition::new("events");

/// An event as stored. `payload` keeps the exact JSON text that was hashed.
#[derive(Serialize, Deserialize)]
struct StoredEvent {
    epoch: Epoch,
    recorded_at: Timestamp,
    schema: u16,
    prev_hash: Option<EventHash>,
    hash: EventHash,
    payload: Box<RawValue>,
}

/// Embedded backend on redb: a single crash-safe file, no server. The
/// default for on-device and single-node use. redb allows one writer at a
/// time, which serializes writes across all runs in the file; readers never
/// block. Every operation runs on Tokio's blocking pool.
#[derive(Clone)]
pub struct RedbStore {
    db: Arc<Database>,
    clock: Arc<dyn Clock>,
}

impl RedbStore {
    /// Opens the database at `path`, creating it if needed.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StoreError> {
        Self::with_clock(path, SystemClock)
    }

    pub fn with_clock(
        path: impl AsRef<Path>,
        clock: impl Clock + 'static,
    ) -> Result<Self, StoreError> {
        let db = Database::create(path).map_err(backend)?;
        let txn = db.begin_write().map_err(backend)?;
        txn.open_table(HEADS).map_err(backend)?;
        txn.open_table(EVENTS).map_err(backend)?;
        txn.commit().map_err(backend)?;
        Ok(Self {
            db: Arc::new(db),
            clock: Arc::new(clock),
        })
    }

    async fn blocking<T, F>(&self, f: F) -> Result<T, StoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Database, &dyn Clock) -> Result<T, StoreError> + Send + 'static,
    {
        let db = Arc::clone(&self.db);
        let clock = Arc::clone(&self.clock);
        tokio::task::spawn_blocking(move || f(&db, clock.as_ref()))
            .await
            .map_err(backend)?
    }
}

/// One write transaction: load the head, apply a `RunHead` rule, persist the
/// new events and the head, commit. Any error drops (aborts) the transaction.
fn write_run<T>(
    db: &Database,
    clock: &dyn Clock,
    run_id: RunId,
    apply: impl FnOnce(&mut RunHead, Timestamp) -> Result<(T, Vec<Event>), StoreError>,
) -> Result<T, StoreError> {
    let txn = db.begin_write().map_err(backend)?;
    let mut head = {
        let heads = txn.open_table(HEADS).map_err(backend)?;
        let guard = heads
            .get(run_id.0.as_bytes())
            .map_err(backend)?
            .ok_or(StoreError::RunNotFound(run_id))?;
        serde_json::from_slice::<RunHead>(guard.value()).map_err(backend)?
    };
    // Read the clock after taking the write lock, as the Postgres backend does.
    let now = clock.now();
    let (out, events) = apply(&mut head, now)?;
    persist(&txn, &head, &events)?;
    txn.commit().map_err(backend)?;
    Ok(out)
}

fn persist(txn: &WriteTransaction, head: &RunHead, events: &[Event]) -> Result<(), StoreError> {
    let key = head.run_id.0.as_bytes();
    let mut table = txn.open_table(EVENTS).map_err(backend)?;
    for ev in events {
        let bytes = encode_event(ev)?;
        if table
            .insert((key, ev.seq.0), bytes.as_slice())
            .map_err(backend)?
            .is_some()
        {
            return Err(StoreError::Backend(format!(
                "event {:?} of run {} already exists",
                ev.seq, head.run_id
            )));
        }
    }
    let head_bytes = serde_json::to_vec(head).map_err(backend)?;
    txn.open_table(HEADS)
        .map_err(backend)?
        .insert(key, head_bytes.as_slice())
        .map_err(backend)?;
    Ok(())
}

impl EventStore for RedbStore {
    async fn create_run(&self, spec: serde_json::Value) -> Result<RunId, StoreError> {
        self.blocking(move |db, clock| {
            let txn = db.begin_write().map_err(backend)?;
            let run_id = RunId::new();
            let (head, ev) = RunHead::create(run_id, spec, clock.now())?;
            persist(&txn, &head, std::slice::from_ref(&ev))?;
            txn.commit().map_err(backend)?;
            Ok(run_id)
        })
        .await
    }

    async fn acquire_lease(
        &self,
        run_id: RunId,
        worker: WorkerId,
        ttl_ms: u64,
    ) -> Result<Lease, StoreError> {
        self.blocking(move |db, clock| {
            write_run(db, clock, run_id, |head, now| {
                let (lease, ev) = head.grant_lease(worker, ttl_ms, now)?;
                Ok((lease, vec![ev]))
            })
        })
        .await
    }

    async fn renew_lease(&self, lease: &Lease, ttl_ms: u64) -> Result<Lease, StoreError> {
        let lease = lease.clone();
        self.blocking(move |db, clock| {
            write_run(db, clock, lease.run_id, |head, now| {
                Ok((head.renew_lease(&lease, ttl_ms, now)?, Vec::new()))
            })
        })
        .await
    }

    async fn append(
        &self,
        lease: &Lease,
        expected_next: Seq,
        payloads: Vec<Payload>,
    ) -> Result<Vec<Event>, StoreError> {
        let lease = lease.clone();
        self.blocking(move |db, clock| {
            write_run(db, clock, lease.run_id, |head, now| {
                let events = head.append(&lease, expected_next, payloads, now)?;
                Ok((events.clone(), events))
            })
        })
        .await
    }

    async fn read(&self, run_id: RunId, from: Seq) -> Result<Vec<Event>, StoreError> {
        self.blocking(move |db, _| {
            let txn = db.begin_read().map_err(backend)?;
            let key = run_id.0.as_bytes();
            let events = txn.open_table(EVENTS).map_err(backend)?;
            let mut out = Vec::new();
            for entry in events
                .range((key, from.0)..=(key, u64::MAX))
                .map_err(backend)?
            {
                let (k, v) = entry.map_err(backend)?;
                out.push(decode_event(run_id, Seq(k.value().1), v.value())?);
            }
            if out.is_empty() {
                let heads = txn.open_table(HEADS).map_err(backend)?;
                if heads.get(key).map_err(backend)?.is_none() {
                    return Err(StoreError::RunNotFound(run_id));
                }
            }
            Ok(out)
        })
        .await
    }
}

fn encode_event(ev: &Event) -> Result<Vec<u8>, StoreError> {
    let payload = serde_json::to_string(&ev.payload).map_err(backend)?;
    serde_json::to_vec(&StoredEvent {
        epoch: ev.epoch,
        recorded_at: ev.recorded_at,
        schema: ev.schema,
        prev_hash: ev.prev_hash,
        hash: ev.hash,
        payload: RawValue::from_string(payload).map_err(backend)?,
    })
    .map_err(backend)
}

fn decode_event(run_id: RunId, seq: Seq, bytes: &[u8]) -> Result<Event, StoreError> {
    let stored: StoredEvent = serde_json::from_slice(bytes).map_err(backend)?;
    Ok(Event {
        run_id,
        seq,
        epoch: stored.epoch,
        recorded_at: stored.recorded_at,
        schema: stored.schema,
        prev_hash: stored.prev_hash,
        hash: stored.hash,
        payload: serde_json::from_str(stored.payload.get()).map_err(backend)?,
    })
}

fn backend(e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(e.to_string())
}
