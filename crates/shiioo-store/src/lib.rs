//! Event store backends. `MemoryStore` is the reference implementation and
//! the backend used in simulation; `PostgresStore` must behave identically
//! (both run the suite in `tests/common`).

mod memory;
#[cfg(feature = "postgres")]
mod postgres;

pub use memory::MemoryStore;
#[cfg(feature = "postgres")]
pub use postgres::{MIGRATOR, PostgresStore};
