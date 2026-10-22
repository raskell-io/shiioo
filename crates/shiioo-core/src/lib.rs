//! Shiioo's execution kernel: the event model and the rules every backend
//! and runtime must obey. This crate does no I/O, so it can run unchanged
//! under deterministic simulation.
//!
//! Invariants (see `docs/architecture.md`):
//! - The per-run event log is the only state. Workers rebuild from it.
//! - Every event carries `seq` (contiguous from 0), the lease `epoch` it was
//!   written under, and the hash of the previous event.
//! - A write is accepted only from the current, unexpired lease (fencing).
//! - Side-effecting tool calls are recorded as an intent before they run; an
//!   intent without a result after a crash becomes an explicit unknown outcome.

pub mod chain;
pub mod clock;
pub mod event;
pub mod ids;
pub mod lease;
pub mod recovery;
pub mod store;

pub use chain::{ChainError, verify_chain};
pub use clock::{Clock, ManualClock, SystemClock};
pub use event::{Event, EventHash, Payload, SCHEMA_VERSION, ToolClass};
pub use ids::{CallId, Epoch, RunId, Seq, Timestamp, WorkerId};
pub use lease::Lease;
pub use recovery::{Recovery, dangling_tool_calls};
pub use store::{EventStore, StoreError};
