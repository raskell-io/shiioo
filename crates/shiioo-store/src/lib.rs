//! Event store backends. `MemoryStore` is the reference implementation the
//! Postgres backend must match, and the backend used in simulation tests.

mod memory;

pub use memory::MemoryStore;
