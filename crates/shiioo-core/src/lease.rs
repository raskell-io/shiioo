use crate::ids::{Epoch, RunId, Timestamp, WorkerId};
use serde::{Deserialize, Serialize};

/// Exclusive right to append to one run, until `expires_at`. Every append
/// presents the lease; the store rejects it unless `epoch` is still current.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lease {
    pub run_id: RunId,
    pub worker: WorkerId,
    pub epoch: Epoch,
    pub expires_at: Timestamp,
}
