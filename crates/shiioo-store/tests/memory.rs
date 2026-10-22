mod common;

use shiioo_core::ManualClock;
use shiioo_store::MemoryStore;

async fn setup() -> Option<(MemoryStore<ManualClock>, ManualClock)> {
    let clock = ManualClock::at(common::START_MS);
    Some((MemoryStore::new(clock.clone()), clock))
}

conformance_suite!(setup);
