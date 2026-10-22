#![cfg(feature = "redb")]

mod common;

use serde_json::json;
use shiioo_core::{Epoch, EventStore, ManualClock, Payload, Seq, StoreError, verify_chain};
use shiioo_store::RedbStore;
use tempfile::TempDir;

/// The store plus the temp dir that holds its file; the dir is deleted when
/// this is dropped, so it must live as long as the store.
struct TempStore {
    store: RedbStore,
    _dir: TempDir,
}

impl EventStore for TempStore {
    async fn create_run(&self, spec: serde_json::Value) -> Result<shiioo_core::RunId, StoreError> {
        self.store.create_run(spec).await
    }
    async fn acquire_lease(
        &self,
        run_id: shiioo_core::RunId,
        worker: shiioo_core::WorkerId,
        ttl_ms: u64,
    ) -> Result<shiioo_core::Lease, StoreError> {
        self.store.acquire_lease(run_id, worker, ttl_ms).await
    }
    async fn renew_lease(
        &self,
        lease: &shiioo_core::Lease,
        ttl_ms: u64,
    ) -> Result<shiioo_core::Lease, StoreError> {
        self.store.renew_lease(lease, ttl_ms).await
    }
    async fn append(
        &self,
        lease: &shiioo_core::Lease,
        expected_next: Seq,
        payloads: Vec<Payload>,
    ) -> Result<Vec<shiioo_core::Event>, StoreError> {
        self.store.append(lease, expected_next, payloads).await
    }
    async fn read(
        &self,
        run_id: shiioo_core::RunId,
        from: Seq,
    ) -> Result<Vec<shiioo_core::Event>, StoreError> {
        self.store.read(run_id, from).await
    }
}

fn temp_dir() -> TempDir {
    match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(e) => panic!("creating temp dir: {e}"),
    }
}

async fn setup() -> Option<(TempStore, ManualClock)> {
    let dir = temp_dir();
    let clock = ManualClock::at(common::START_MS);
    let store = match RedbStore::with_clock(dir.path().join("shiioo.redb"), clock.clone()) {
        Ok(s) => s,
        Err(e) => panic!("opening store: {e}"),
    };
    Some((TempStore { store, _dir: dir }, clock))
}

conformance_suite!(setup);

/// Everything survives closing and reopening the file, including the lease
/// epoch: a lease granted before the restart stays fenced after a takeover.
#[tokio::test]
async fn state_survives_reopen() -> Result<(), StoreError> {
    let dir = temp_dir();
    let path = dir.path().join("shiioo.redb");
    let clock = ManualClock::at(common::START_MS);

    let store = RedbStore::with_clock(&path, clock.clone())?;
    let run = store.create_run(json!({ "goal": "reopen" })).await?;
    let w1 = store.acquire_lease(run, "w1".into(), common::TTL).await?;
    store
        .append(
            &w1,
            Seq(2),
            vec![Payload::ModelResponseRecorded {
                model: "m".into(),
                response: json!({ "ratio": 0.1 + 0.2 }),
            }],
        )
        .await?;
    let before = store.read(run, Seq(0)).await?;
    drop(store);

    let store = RedbStore::with_clock(&path, clock.clone())?;
    let after = store.read(run, Seq(0)).await?;
    assert_eq!(after, before);
    assert_eq!(verify_chain(&after), Ok(()));

    // w1's lease is still live after reopen, so w2 can't take over yet.
    assert!(matches!(
        store.acquire_lease(run, "w2".into(), common::TTL).await,
        Err(StoreError::LeaseHeld { .. })
    ));

    clock.advance_ms(common::TTL + 1);
    let w2 = store.acquire_lease(run, "w2".into(), common::TTL).await?;
    assert_eq!(w2.epoch, Epoch(2));
    assert_eq!(
        store.append(&w1, Seq(4), vec![]).await,
        Err(StoreError::Fenced {
            presented: Epoch(1),
            current: Epoch(2)
        })
    );
    Ok(())
}
