//! Event store backends. `MemoryStore` is the reference implementation and
//! the backend used in simulation. `RedbStore` (default) is the embedded
//! on-device backend; `PostgresStore` is the cluster backend. All of them
//! run the suite in `tests/common`.

mod memory;
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "redb")]
mod redb;

pub use memory::MemoryStore;
#[cfg(feature = "postgres")]
pub use postgres::{MIGRATOR, PostgresStore};
#[cfg(feature = "redb")]
pub use redb::RedbStore;
