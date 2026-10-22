//! Event store backends. `MemoryStore` is the reference implementation and
//! the backend used in simulation. `RedbStore` (default) is the embedded
//! on-device backend; `PostgresStore` is the cluster backend. All of them
//! run the suite in `tests/common`.

mod memory;
mod notify;
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "redb")]
mod redb;

pub use memory::MemoryStore;
pub use notify::Notifying;
#[cfg(feature = "postgres")]
pub use postgres::{MIGRATOR, PostgresStore};
#[cfg(feature = "redb")]
pub use redb::RedbStore;
