//! Behaviour every `EventStore` backend must have. Run against `MemoryStore`
//! today; the Postgres backend will run the same cases.

use serde_json::json;
use shiioo_core::{
    CallId, Epoch, EventStore, ManualClock, Payload, Recovery, Seq, StoreError, ToolClass,
    WorkerId, dangling_tool_calls, verify_chain,
};
use shiioo_store::MemoryStore;

const TTL: u64 = 30_000;

fn store() -> (MemoryStore<ManualClock>, ManualClock) {
    let clock = ManualClock::at(1_700_000_000_000);
    (MemoryStore::new(clock.clone()), clock)
}

fn model_step(n: u64) -> Payload {
    Payload::ModelResponseRecorded {
        model: "test-model".into(),
        response: json!({ "turn": n }),
    }
}

#[tokio::test]
async fn log_is_a_valid_hash_chain() -> Result<(), StoreError> {
    let (store, _) = store();
    let run = store.create_run(json!({ "goal": "test" })).await?;
    let lease = store.acquire_lease(run, "w1".into(), TTL).await?;
    store
        .append(&lease, Seq(2), vec![model_step(1), model_step(2)])
        .await?;

    let events = store.read(run, Seq(0)).await?;
    assert_eq!(events.len(), 4);
    assert_eq!(verify_chain(&events), Ok(()));
    Ok(())
}

#[tokio::test]
async fn live_lease_blocks_other_workers() -> Result<(), StoreError> {
    let (store, _) = store();
    let run = store.create_run(json!({})).await?;
    store.acquire_lease(run, "w1".into(), TTL).await?;

    let err = store.acquire_lease(run, "w2".into(), TTL).await;
    assert!(matches!(err, Err(StoreError::LeaseHeld { holder, .. }) if holder == WorkerId::from("w1")));
    Ok(())
}

/// The split-brain case: w1 stalls (GC pause, partition), its lease expires,
/// w2 takes over, then w1 wakes up and tries to write.
#[tokio::test]
async fn stale_worker_is_fenced_after_takeover() -> Result<(), StoreError> {
    let (store, clock) = store();
    let run = store.create_run(json!({})).await?;
    let w1 = store.acquire_lease(run, "w1".into(), TTL).await?;

    clock.advance_ms(TTL + 1);
    let w2 = store.acquire_lease(run, "w2".into(), TTL).await?;
    assert_eq!(w2.epoch, Epoch(2));
    store.append(&w2, Seq(3), vec![model_step(1)]).await?;

    let err = store.append(&w1, Seq(4), vec![model_step(99)]).await;
    assert_eq!(
        err,
        Err(StoreError::Fenced {
            presented: Epoch(1),
            current: Epoch(2)
        })
    );
    let err = store.renew_lease(&w1, TTL).await;
    assert!(matches!(err, Err(StoreError::Fenced { .. })));
    Ok(())
}

#[tokio::test]
async fn expired_lease_cannot_write_even_without_takeover() -> Result<(), StoreError> {
    let (store, clock) = store();
    let run = store.create_run(json!({})).await?;
    let lease = store.acquire_lease(run, "w1".into(), TTL).await?;

    clock.advance_ms(TTL);
    let err = store.append(&lease, Seq(2), vec![model_step(1)]).await;
    assert_eq!(err, Err(StoreError::LeaseExpired(Epoch(1))));
    Ok(())
}

#[tokio::test]
async fn renewal_keeps_the_epoch() -> Result<(), StoreError> {
    let (store, clock) = store();
    let run = store.create_run(json!({})).await?;
    let lease = store.acquire_lease(run, "w1".into(), TTL).await?;

    clock.advance_ms(TTL - 1);
    let renewed = store.renew_lease(&lease, TTL).await?;
    assert_eq!(renewed.epoch, lease.epoch);
    clock.advance_ms(TTL - 1);
    store.append(&renewed, Seq(2), vec![model_step(1)]).await?;
    Ok(())
}

#[tokio::test]
async fn stale_view_gets_seq_conflict() -> Result<(), StoreError> {
    let (store, _) = store();
    let run = store.create_run(json!({})).await?;
    let lease = store.acquire_lease(run, "w1".into(), TTL).await?;
    store.append(&lease, Seq(2), vec![model_step(1)]).await?;

    let err = store.append(&lease, Seq(2), vec![model_step(2)]).await;
    assert_eq!(
        err,
        Err(StoreError::SeqConflict {
            expected: Seq(2),
            actual: Seq(3)
        })
    );
    Ok(())
}

/// The kill -9 case: a worker records two tool intents and dies before
/// recording either result. The next worker must re-run the idempotent one
/// and must not re-run the side-effecting one.
#[tokio::test]
async fn takeover_after_crash_recovers_tool_calls_safely() -> Result<(), StoreError> {
    let (store, clock) = store();
    let run = store.create_run(json!({})).await?;
    let w1 = store.acquire_lease(run, "w1".into(), TTL).await?;
    store
        .append(
            &w1,
            Seq(2),
            vec![
                model_step(1),
                Payload::ToolIntent {
                    call_id: "push".into(),
                    tool: "git.push".into(),
                    class: ToolClass::SideEffecting,
                    input: json!({ "branch": "main" }),
                },
                Payload::ToolIntent {
                    call_id: "read".into(),
                    tool: "fs.read".into(),
                    class: ToolClass::Pure,
                    input: json!({ "path": "README.md" }),
                },
                Payload::ToolResult {
                    call_id: "read".into(),
                    output: json!("..."),
                    is_error: false,
                },
                Payload::ToolIntent {
                    call_id: "label".into(),
                    tool: "github.add_label".into(),
                    class: ToolClass::Idempotent {
                        key: "issue-7:triaged".into(),
                    },
                    input: json!({ "issue": 7, "label": "triaged" }),
                },
            ],
        )
        .await?;
    drop(w1); // worker dies here

    clock.advance_ms(TTL + 1);
    let w2 = store.acquire_lease(run, "w2".into(), TTL).await?;
    let events = store.read(run, Seq(0)).await?;
    assert_eq!(verify_chain(&events), Ok(()));

    let plan = dangling_tool_calls(&events);
    assert_eq!(
        plan,
        vec![
            Recovery::MarkUnknown {
                call_id: CallId::from("push")
            },
            Recovery::Rerun {
                call_id: CallId::from("label"),
                class: ToolClass::Idempotent {
                    key: "issue-7:triaged".into()
                },
            },
        ]
    );

    let next = Seq(events.len() as u64);
    store
        .append(
            &w2,
            next,
            vec![Payload::ToolOutcomeUnknown {
                call_id: "push".into(),
            }],
        )
        .await?;
    let events = store.read(run, Seq(0)).await?;
    assert_eq!(dangling_tool_calls(&events).len(), 1);
    Ok(())
}

#[tokio::test]
async fn finished_run_is_closed() -> Result<(), StoreError> {
    let (store, _) = store();
    let run = store.create_run(json!({})).await?;
    let lease = store.acquire_lease(run, "w1".into(), TTL).await?;
    store
        .append(
            &lease,
            Seq(2),
            vec![Payload::RunCompleted {
                output: json!("done"),
            }],
        )
        .await?;

    let err = store.append(&lease, Seq(3), vec![model_step(1)]).await;
    assert_eq!(err, Err(StoreError::RunClosed(run)));
    let err = store.acquire_lease(run, "w2".into(), TTL).await;
    assert_eq!(err, Err(StoreError::RunClosed(run)));
    Ok(())
}

#[tokio::test]
async fn workers_cannot_forge_store_events() -> Result<(), StoreError> {
    let (store, _) = store();
    let run = store.create_run(json!({})).await?;
    let lease = store.acquire_lease(run, "w1".into(), TTL).await?;

    let err = store
        .append(
            &lease,
            Seq(2),
            vec![Payload::LeaseAcquired {
                worker: "w1".into(),
                expires_at: lease.expires_at,
            }],
        )
        .await;
    assert!(matches!(err, Err(StoreError::InvalidPayload(_))));
    Ok(())
}
