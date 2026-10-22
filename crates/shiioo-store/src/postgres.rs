use shiioo_core::{
    Clock, Epoch, Event, EventHash, EventStore, Lease, Payload, RunHead, RunId, Seq, StoreError,
    Timestamp, WorkerId,
};
use sqlx::postgres::{PgPool, PgRow};
use sqlx::{Postgres, Row, Transaction};
use std::sync::Arc;

pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Postgres backend. Each write is one transaction that locks the run's head
/// row, applies the `RunHead` rules from `shiioo-core`, inserts the sealed
/// events and updates the head.
///
/// Lease expiry uses the database clock by default, so clock skew between
/// worker machines cannot extend a lease. Tests inject a `Clock` instead.
#[derive(Clone)]
pub struct PostgresStore {
    pool: PgPool,
    clock: Option<Arc<dyn Clock>>,
}

impl PostgresStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool, clock: None }
    }

    pub fn with_clock(pool: PgPool, clock: impl Clock + 'static) -> Self {
        Self {
            pool,
            clock: Some(Arc::new(clock)),
        }
    }

    pub async fn migrate(&self) -> Result<(), StoreError> {
        MIGRATOR.run(&self.pool).await.map_err(backend)
    }

    async fn now(&self, tx: &mut Transaction<'_, Postgres>) -> Result<Timestamp, StoreError> {
        if let Some(clock) = &self.clock {
            return Ok(clock.now());
        }
        let ms: i64 =
            sqlx::query_scalar("SELECT (extract(epoch FROM clock_timestamp()) * 1000)::bigint")
                .fetch_one(&mut **tx)
                .await
                .map_err(backend)?;
        Ok(Timestamp(from_i64(ms)?))
    }

    /// Locks the head row until the transaction ends.
    async fn lock_head(
        tx: &mut Transaction<'_, Postgres>,
        run_id: RunId,
    ) -> Result<RunHead, StoreError> {
        let row = sqlx::query(
            "SELECT next_seq, last_hash, closed, lease_worker, lease_epoch, lease_expires_at_ms
             FROM runs WHERE run_id = $1 FOR UPDATE",
        )
        .bind(run_id.0)
        .fetch_optional(&mut **tx)
        .await
        .map_err(backend)?
        .ok_or(StoreError::RunNotFound(run_id))?;
        head_from_row(run_id, &row)
    }

    async fn insert_events(
        tx: &mut Transaction<'_, Postgres>,
        events: &[Event],
    ) -> Result<(), StoreError> {
        for ev in events {
            let payload = serde_json::to_string(&ev.payload)
                .map_err(|e| StoreError::Backend(format!("encoding payload: {e}")))?;
            sqlx::query(
                "INSERT INTO events
                   (run_id, seq, epoch, recorded_at_ms, schema, prev_hash, hash, payload)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8::json)",
            )
            .bind(ev.run_id.0)
            .bind(to_i64(ev.seq.0)?)
            .bind(to_i64(ev.epoch.0)?)
            .bind(to_i64(ev.recorded_at.0)?)
            .bind(ev.schema as i16)
            .bind(ev.prev_hash.map(|h| h.0.to_vec()))
            .bind(ev.hash.0.to_vec())
            .bind(payload)
            .execute(&mut **tx)
            .await
            .map_err(backend)?;
        }
        Ok(())
    }

    async fn save_head(
        tx: &mut Transaction<'_, Postgres>,
        head: &RunHead,
    ) -> Result<(), StoreError> {
        let lease = head.lease.as_ref();
        sqlx::query(
            "UPDATE runs SET next_seq = $2, last_hash = $3, closed = $4,
                    lease_worker = $5, lease_epoch = $6, lease_expires_at_ms = $7
             WHERE run_id = $1",
        )
        .bind(head.run_id.0)
        .bind(to_i64(head.next_seq.0)?)
        .bind(head.last_hash.map(|h| h.0.to_vec()))
        .bind(head.closed)
        .bind(lease.map(|l| l.worker.0.clone()))
        .bind(to_i64(head.current_epoch().0)?)
        .bind(lease.map(|l| to_i64(l.expires_at.0)).transpose()?)
        .execute(&mut **tx)
        .await
        .map_err(backend)?;
        Ok(())
    }
}

impl EventStore for PostgresStore {
    async fn create_run(&self, spec: serde_json::Value) -> Result<RunId, StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        let now = self.now(&mut tx).await?;
        let run_id = RunId::new();
        let (head, ev) = RunHead::create(run_id, spec, now)?;
        sqlx::query("INSERT INTO runs (run_id, next_seq) VALUES ($1, 0)")
            .bind(run_id.0)
            .execute(&mut *tx)
            .await
            .map_err(backend)?;
        Self::insert_events(&mut tx, std::slice::from_ref(&ev)).await?;
        Self::save_head(&mut tx, &head).await?;
        tx.commit().await.map_err(backend)?;
        Ok(run_id)
    }

    async fn acquire_lease(
        &self,
        run_id: RunId,
        worker: WorkerId,
        ttl_ms: u64,
    ) -> Result<Lease, StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        let mut head = Self::lock_head(&mut tx, run_id).await?;
        let now = self.now(&mut tx).await?;
        let (lease, ev) = head.grant_lease(worker, ttl_ms, now)?;
        Self::insert_events(&mut tx, std::slice::from_ref(&ev)).await?;
        Self::save_head(&mut tx, &head).await?;
        tx.commit().await.map_err(backend)?;
        Ok(lease)
    }

    async fn renew_lease(&self, lease: &Lease, ttl_ms: u64) -> Result<Lease, StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        let mut head = Self::lock_head(&mut tx, lease.run_id).await?;
        let now = self.now(&mut tx).await?;
        let renewed = head.renew_lease(lease, ttl_ms, now)?;
        Self::save_head(&mut tx, &head).await?;
        tx.commit().await.map_err(backend)?;
        Ok(renewed)
    }

    async fn append(
        &self,
        lease: &Lease,
        expected_next: Seq,
        payloads: Vec<Payload>,
    ) -> Result<Vec<Event>, StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        let mut head = Self::lock_head(&mut tx, lease.run_id).await?;
        let now = self.now(&mut tx).await?;
        let events = head.append(lease, expected_next, payloads, now)?;
        Self::insert_events(&mut tx, &events).await?;
        Self::save_head(&mut tx, &head).await?;
        tx.commit().await.map_err(backend)?;
        Ok(events)
    }

    async fn read(&self, run_id: RunId, from: Seq) -> Result<Vec<Event>, StoreError> {
        let rows = sqlx::query(
            "SELECT seq, epoch, recorded_at_ms, schema, prev_hash, hash, payload::text AS payload
             FROM events WHERE run_id = $1 AND seq >= $2 ORDER BY seq",
        )
        .bind(run_id.0)
        .bind(to_i64(from.0)?)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;

        if rows.is_empty() {
            let exists: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM runs WHERE run_id = $1)")
                    .bind(run_id.0)
                    .fetch_one(&self.pool)
                    .await
                    .map_err(backend)?;
            if !exists {
                return Err(StoreError::RunNotFound(run_id));
            }
        }
        rows.iter().map(|row| event_from_row(run_id, row)).collect()
    }
}

fn head_from_row(run_id: RunId, row: &PgRow) -> Result<RunHead, StoreError> {
    let epoch: i64 = row.try_get("lease_epoch").map_err(backend)?;
    let worker: Option<String> = row.try_get("lease_worker").map_err(backend)?;
    let expires: Option<i64> = row.try_get("lease_expires_at_ms").map_err(backend)?;
    let lease = match (worker, expires) {
        (Some(worker), Some(expires)) => Some(Lease {
            run_id,
            worker: WorkerId(worker),
            epoch: Epoch(from_i64(epoch)?),
            expires_at: Timestamp(from_i64(expires)?),
        }),
        _ => None,
    };
    Ok(RunHead {
        run_id,
        next_seq: Seq(from_i64(row.try_get("next_seq").map_err(backend)?)?),
        last_hash: hash_opt(row.try_get("last_hash").map_err(backend)?)?,
        closed: row.try_get("closed").map_err(backend)?,
        lease,
    })
}

fn event_from_row(run_id: RunId, row: &PgRow) -> Result<Event, StoreError> {
    let payload: String = row.try_get("payload").map_err(backend)?;
    let schema: i16 = row.try_get("schema").map_err(backend)?;
    Ok(Event {
        run_id,
        seq: Seq(from_i64(row.try_get("seq").map_err(backend)?)?),
        epoch: Epoch(from_i64(row.try_get("epoch").map_err(backend)?)?),
        recorded_at: Timestamp(from_i64(row.try_get("recorded_at_ms").map_err(backend)?)?),
        schema: u16::try_from(schema).map_err(backend)?,
        prev_hash: hash_opt(row.try_get("prev_hash").map_err(backend)?)?,
        hash: hash(row.try_get("hash").map_err(backend)?)?,
        payload: serde_json::from_str(&payload)
            .map_err(|e| StoreError::Backend(format!("decoding payload: {e}")))?,
    })
}

fn hash(bytes: Vec<u8>) -> Result<EventHash, StoreError> {
    <[u8; 32]>::try_from(bytes)
        .map(EventHash)
        .map_err(|b| StoreError::Backend(format!("hash has {} bytes, expected 32", b.len())))
}

fn hash_opt(bytes: Option<Vec<u8>>) -> Result<Option<EventHash>, StoreError> {
    bytes.map(hash).transpose()
}

fn to_i64(v: u64) -> Result<i64, StoreError> {
    i64::try_from(v).map_err(backend)
}

fn from_i64(v: i64) -> Result<u64, StoreError> {
    u64::try_from(v).map_err(backend)
}

fn backend(e: impl std::fmt::Display) -> StoreError {
    StoreError::Backend(e.to_string())
}
