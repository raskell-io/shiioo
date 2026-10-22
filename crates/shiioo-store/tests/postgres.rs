//! Runs the conformance suite against Postgres. Set
//! `SHIIOO_TEST_DATABASE_URL` to enable it; under `CI` a missing URL fails
//! instead of skipping, so the suite can't silently stop running.

#![cfg(feature = "postgres")]

mod common;

use shiioo_core::ManualClock;
use shiioo_store::PostgresStore;
use sqlx::postgres::PgPoolOptions;

async fn setup() -> Option<(PostgresStore, ManualClock)> {
    let Ok(url) = std::env::var("SHIIOO_TEST_DATABASE_URL") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "SHIIOO_TEST_DATABASE_URL must be set in CI"
        );
        eprintln!("skipping: SHIIOO_TEST_DATABASE_URL not set");
        return None;
    };
    let pool = match PgPoolOptions::new().max_connections(4).connect(&url).await {
        Ok(pool) => pool,
        Err(e) => panic!("connecting to {url}: {e}"),
    };
    let clock = ManualClock::at(common::START_MS);
    let store = PostgresStore::with_clock(pool, clock.clone());
    if let Err(e) = store.migrate().await {
        panic!("migrating: {e}");
    }
    Some((store, clock))
}

conformance_suite!(setup);

/// The append-only trigger rejects edits made outside Shiioo.
#[tokio::test]
async fn events_table_rejects_updates_and_deletes() -> Result<(), shiioo_core::StoreError> {
    use shiioo_core::EventStore;
    let Some((store, _)) = setup().await else {
        return Ok(());
    };
    let run = store.create_run(serde_json::json!({})).await?;
    let Ok(url) = std::env::var("SHIIOO_TEST_DATABASE_URL") else {
        return Ok(());
    };
    let pool = match sqlx::PgPool::connect(&url).await {
        Ok(p) => p,
        Err(e) => panic!("{e}"),
    };
    let update = sqlx::query("UPDATE events SET epoch = 99 WHERE run_id = $1")
        .bind(run.0)
        .execute(&pool)
        .await;
    assert!(update.is_err_and(|e| e.to_string().contains("append-only")));
    let delete = sqlx::query("DELETE FROM events WHERE run_id = $1")
        .bind(run.0)
        .execute(&pool)
        .await;
    assert!(delete.is_err_and(|e| e.to_string().contains("append-only")));
    Ok(())
}
