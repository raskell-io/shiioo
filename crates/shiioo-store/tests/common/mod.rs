//! Behaviour every `EventStore` backend must have. Each backend's test file
//! calls `conformance_suite!(setup)`, where `setup` returns the store and the
//! `ManualClock` it reads time from (or `None` to skip).

use serde_json::json;
use shiioo_core::{
    CallId, Epoch, EventStore, ManualClock, Payload, Recovery, Seq, StoreError, ToolClass,
    WorkerId, dangling_tool_calls, verify_chain,
};

pub const TTL: u64 = 30_000;
pub const START_MS: u64 = 1_700_000_000_000;

#[macro_export]
macro_rules! conformance_suite {
    ($setup:path) => {
        $crate::conformance_suite!(@cases $setup;
            log_is_a_valid_hash_chain,
            live_lease_blocks_other_workers,
            stale_worker_is_fenced_after_takeover,
            expired_lease_cannot_write_even_without_takeover,
            renewal_keeps_the_epoch,
            stale_view_gets_seq_conflict,
            concurrent_appends_exactly_one_wins,
            takeover_after_crash_recovers_tool_calls_safely,
            finished_run_is_closed,
            failed_batch_writes_nothing,
            workers_cannot_forge_store_events,
            unknown_run_is_not_found,
            read_from_offset,
        );
    };
    (@cases $setup:path; $($case:ident),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $case() -> Result<(), shiioo_core::StoreError> {
                let Some((store, clock)) = $setup().await else {
                    return Ok(());
                };
                common::$case(&store, &clock).await
            }
        )*
    };
}

fn model_step(n: u64) -> Payload {
    Payload::ModelResponseRecorded {
        model: "test-model".into(),
        response: json!({ "turn": n, "ratio": 0.1 + 0.2, "text": "héllo\n\"quoted\"" }),
    }
}

pub async fn log_is_a_valid_hash_chain<S: EventStore>(
    store: &S,
    _: &ManualClock,
) -> Result<(), StoreError> {
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

pub async fn live_lease_blocks_other_workers<S: EventStore>(
    store: &S,
    _: &ManualClock,
) -> Result<(), StoreError> {
    let run = store.create_run(json!({})).await?;
    store.acquire_lease(run, "w1".into(), TTL).await?;

    let err = store.acquire_lease(run, "w2".into(), TTL).await;
    assert!(
        matches!(err, Err(StoreError::LeaseHeld { holder, .. }) if holder == WorkerId::from("w1"))
    );
    Ok(())
}

/// The split-brain case: w1 stalls (GC pause, partition), its lease expires,
/// w2 takes over, then w1 wakes up and tries to write.
pub async fn stale_worker_is_fenced_after_takeover<S: EventStore>(
    store: &S,
    clock: &ManualClock,
) -> Result<(), StoreError> {
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

pub async fn expired_lease_cannot_write_even_without_takeover<S: EventStore>(
    store: &S,
    clock: &ManualClock,
) -> Result<(), StoreError> {
    let run = store.create_run(json!({})).await?;
    let lease = store.acquire_lease(run, "w1".into(), TTL).await?;

    clock.advance_ms(TTL);
    let err = store.append(&lease, Seq(2), vec![model_step(1)]).await;
    assert_eq!(err, Err(StoreError::LeaseExpired(Epoch(1))));
    Ok(())
}

pub async fn renewal_keeps_the_epoch<S: EventStore>(
    store: &S,
    clock: &ManualClock,
) -> Result<(), StoreError> {
    let run = store.create_run(json!({})).await?;
    let lease = store.acquire_lease(run, "w1".into(), TTL).await?;

    clock.advance_ms(TTL - 1);
    let renewed = store.renew_lease(&lease, TTL).await?;
    assert_eq!(renewed.epoch, lease.epoch);
    clock.advance_ms(TTL - 1);
    store.append(&renewed, Seq(2), vec![model_step(1)]).await?;
    Ok(())
}

pub async fn stale_view_gets_seq_conflict<S: EventStore>(
    store: &S,
    _: &ManualClock,
) -> Result<(), StoreError> {
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

/// Two writers race for the same slot (for example a worker and its own
/// retried request). Exactly one wins; the log stays a valid chain.
pub async fn concurrent_appends_exactly_one_wins<S: EventStore>(
    store: &S,
    _: &ManualClock,
) -> Result<(), StoreError> {
    let run = store.create_run(json!({})).await?;
    let lease = store.acquire_lease(run, "w1".into(), TTL).await?;

    for round in 0..10u64 {
        let next = Seq(2 + round);
        let (a, b) = tokio::join!(
            store.append(&lease, next, vec![model_step(round)]),
            store.append(&lease, next, vec![model_step(round + 100)]),
        );
        let wins = [&a, &b].iter().filter(|r| r.is_ok()).count();
        assert_eq!(wins, 1, "round {round}: {a:?} / {b:?}");
        for r in [a, b] {
            if let Err(e) = r {
                assert!(matches!(e, StoreError::SeqConflict { .. }), "{e:?}");
            }
        }
    }
    let events = store.read(run, Seq(0)).await?;
    assert_eq!(events.len(), 12);
    assert_eq!(verify_chain(&events), Ok(()));
    Ok(())
}

/// The kill -9 case: a worker records two tool intents and dies before
/// recording either result. The next worker must re-run the idempotent one
/// and must not re-run the side-effecting one.
pub async fn takeover_after_crash_recovers_tool_calls_safely<S: EventStore>(
    store: &S,
    clock: &ManualClock,
) -> Result<(), StoreError> {
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

pub async fn finished_run_is_closed<S: EventStore>(
    store: &S,
    _: &ManualClock,
) -> Result<(), StoreError> {
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

pub async fn failed_batch_writes_nothing<S: EventStore>(
    store: &S,
    _: &ManualClock,
) -> Result<(), StoreError> {
    let run = store.create_run(json!({})).await?;
    let lease = store.acquire_lease(run, "w1".into(), TTL).await?;

    let err = store
        .append(
            &lease,
            Seq(2),
            vec![
                model_step(1),
                Payload::RunFailed { reason: "x".into() },
                model_step(2),
            ],
        )
        .await;
    assert!(matches!(err, Err(StoreError::InvalidPayload(_))));
    assert_eq!(store.read(run, Seq(0)).await?.len(), 2);
    store.append(&lease, Seq(2), vec![model_step(1)]).await?;
    Ok(())
}

pub async fn workers_cannot_forge_store_events<S: EventStore>(
    store: &S,
    _: &ManualClock,
) -> Result<(), StoreError> {
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

pub async fn unknown_run_is_not_found<S: EventStore>(
    store: &S,
    _: &ManualClock,
) -> Result<(), StoreError> {
    let ghost = shiioo_core::RunId::new();
    assert_eq!(
        store.read(ghost, Seq(0)).await,
        Err(StoreError::RunNotFound(ghost))
    );
    assert_eq!(
        store.acquire_lease(ghost, "w1".into(), TTL).await,
        Err(StoreError::RunNotFound(ghost))
    );
    Ok(())
}

pub async fn read_from_offset<S: EventStore>(store: &S, _: &ManualClock) -> Result<(), StoreError> {
    let run = store.create_run(json!({})).await?;
    let lease = store.acquire_lease(run, "w1".into(), TTL).await?;
    store
        .append(&lease, Seq(2), vec![model_step(1), model_step(2)])
        .await?;

    let tail = store.read(run, Seq(2)).await?;
    assert_eq!(
        tail.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![Seq(2), Seq(3)]
    );
    assert!(store.read(run, Seq(10)).await?.is_empty());
    Ok(())
}
