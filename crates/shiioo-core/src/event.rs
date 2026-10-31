use crate::ids::{CallId, Epoch, RunId, Seq, Timestamp, WorkerId};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};
use std::fmt;

/// Version of the payload schema written by this build. Readers upcast older
/// versions after verifying the chain, never before.
pub const SCHEMA_VERSION: u16 = 1;

const HASH_DOMAIN: &[u8] = b"shiioo/event/v1\0";

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventHash(pub [u8; 32]);

impl fmt::Debug for EventHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EventHash({})", hex::encode(self.0))
    }
}

impl fmt::Display for EventHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex::encode(self.0))
    }
}

impl Serialize for EventHash {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for EventHash {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let mut out = [0u8; 32];
        hex::decode_to_slice(&s, &mut out).map_err(serde::de::Error::custom)?;
        Ok(Self(out))
    }
}

/// How a tool behaves when it may have run but its result was never recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "class", rename_all = "snake_case")]
pub enum ToolClass {
    /// No effects outside the call. Safe to run again.
    Pure,
    /// Running again with the same key has the same effect as running once.
    Idempotent { key: String },
    /// Must not be run again blindly (a push, a payment, a sent message).
    SideEffecting,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Payload {
    RunCreated {
        spec: Value,
    },
    /// Written by the store itself when a lease is granted, never appended
    /// by a worker. Makes every takeover visible in the log.
    LeaseAcquired {
        worker: WorkerId,
        expires_at: Timestamp,
    },
    /// The complete model response, recorded before any tool in it runs.
    ModelResponseRecorded {
        model: String,
        response: Value,
        /// How many events of the log the request was built from: input
        /// at or after this seq (a message sent while the model was busy)
        /// was not seen by this response, even if it is recorded before
        /// it. Absent for harness responses and older events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_seq: Option<Seq>,
    },
    ToolIntent {
        call_id: CallId,
        tool: String,
        class: ToolClass,
        input: Value,
    },
    ToolResult {
        call_id: CallId,
        output: Value,
        is_error: bool,
    },
    /// A side-effecting call may or may not have happened. Needs a reconcile
    /// hook or a human decision before the run continues.
    ToolOutcomeUnknown {
        call_id: CallId,
    },
    /// An external agent harness (Claude Code, Codex, …) started or resumed
    /// a session for this run. Its steps follow as `ModelResponseRecorded`,
    /// `ToolIntent` and `ToolResult` events observed from its output.
    HarnessSessionStarted {
        harness: String,
        version: String,
        session_id: String,
        cwd: String,
        model: String,
    },
    /// The harness's own summary of a finished session: cost, usage, what
    /// it was denied. Recorded before the terminal event.
    HarnessResult {
        harness: String,
        session_id: String,
        subtype: String,
        is_error: bool,
        cost_usd: Option<f64>,
        usage: Value,
        permission_denials: Value,
    },
    /// A provider reported its rate-limit windows (for example Claude's
    /// five-hour and seven-day windows). Input for capacity routing.
    CapacityObserved {
        provider: String,
        windows: Value,
    },
    /// Input from the owner or an integration, written by the server
    /// without the run's lease. Makes a parked run runnable.
    UserMessage {
        author: String,
        content: String,
    },
    /// The rules' verdict on a tool call: `allow`, `deny` or `ask`.
    /// Written by the server when the harness (or the native loop) asks.
    PolicyDecision {
        call_id: Option<CallId>,
        tool: String,
        decision: String,
        rule: Option<String>,
        reason: String,
    },
    /// The owner has been asked. The call waits for `ApprovalDecided`.
    ApprovalRequested {
        approval_id: String,
        call_id: Option<CallId>,
        tool: String,
        input: Value,
        summary: String,
        expires_at: Option<Timestamp>,
    },
    /// The owner (or a timeout) answered: `allow` or `deny`.
    ApprovalDecided {
        approval_id: String,
        decision: String,
        by: String,
        note: Option<String>,
    },
    /// The owner asked for the run to stop. The worker stops at its next
    /// step boundary and writes `RunCancelled`.
    CancelRequested {
        by: String,
        reason: String,
    },
    /// The owner wants the run's current harness process stopped so it
    /// reads their message now (desk "Now"), not when its turn ends. Comes
    /// right after the `UserMessage`. The worker interrupts the process and
    /// resumes the session with the message; with no process running, or
    /// on a native run, it has no effect.
    InterruptRequested {
        by: String,
    },
    /// Where the run's harness executes: the isolation level and what it
    /// was given (image, mounts, egress mode). Written by the worker before
    /// the harness starts.
    SandboxStarted {
        /// `worktree`, `container` or `openshell`.
        level: String,
        detail: Value,
    },
    /// A secret's value appeared in something about to be recorded and was
    /// replaced by `[secret:<name>]`. Records the name, never the value.
    SecretRedacted {
        name: String,
        count: u32,
    },
    /// The office sent a mail on the run's behalf, after the rules or the
    /// owner allowed it. Written by the server, without the lease.
    MailSent {
        to: Vec<String>,
        subject: String,
        message_id: String,
    },
    /// The worker has nothing to do until something arrives: `input` (a
    /// `UserMessage`) for now; `timer` with `wake_at` later. Appending it
    /// ends the writer's lease, so a parked run holds no lease and any
    /// worker can take it when it is woken.
    RunParked {
        waiting_for: String,
        wake_at: Option<Timestamp>,
    },
    /// The first thing a worker writes after taking a parked run again,
    /// with what woke it (`input`).
    RunWoken {
        reason: String,
    },
    RunCompleted {
        output: Value,
    },
    RunFailed {
        reason: String,
    },
    RunCancelled {
        reason: String,
    },
}

impl Payload {
    /// Ends the writer's lease: nothing may follow it in the same batch.
    pub fn is_parking(&self) -> bool {
        matches!(self, Payload::RunParked { .. })
    }

    /// Nothing may be appended after a terminal event.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Payload::RunCompleted { .. } | Payload::RunFailed { .. } | Payload::RunCancelled { .. }
        )
    }

    /// Input that comes from outside the run's worker and may be appended
    /// without its lease: the owner's messages and decisions. Everything
    /// else is written by the lease holder (or the store itself).
    pub fn is_external(&self) -> bool {
        matches!(
            self,
            Payload::UserMessage { .. }
                | Payload::PolicyDecision { .. }
                | Payload::ApprovalRequested { .. }
                | Payload::ApprovalDecided { .. }
                | Payload::CancelRequested { .. }
                | Payload::InterruptRequested { .. }
                | Payload::MailSent { .. }
        )
    }
}

/// A payload together with the exact JSON text it was hashed over.
///
/// The hash covers these bytes, not a re-encoding of the decoded value, so
/// what a backend stored is exactly what verification checks. Decoding
/// (and, later, upcasting older schema versions) never changes the bytes.
#[derive(Clone, Debug)]
pub struct EventPayload {
    raw: Box<RawValue>,
    typed: Payload,
}

impl EventPayload {
    /// Encodes a payload written by this build.
    pub fn encode(payload: Payload) -> Result<Self, serde_json::Error> {
        Ok(Self {
            raw: serde_json::value::to_raw_value(&payload)?,
            typed: payload,
        })
    }

    /// Decodes stored bytes written under payload schema `schema`.
    pub fn from_raw(schema: u16, raw: Box<RawValue>) -> Result<Self, PayloadError> {
        if schema > SCHEMA_VERSION {
            return Err(PayloadError::NewerSchema(schema));
        }
        // Only schema 1 exists. Upcasters for older versions go here, and
        // they produce `typed` only: `raw` stays as stored.
        let typed = serde_json::from_str(raw.get()).map_err(PayloadError::Decode)?;
        Ok(Self { raw, typed })
    }

    /// The exact stored JSON text.
    pub fn raw(&self) -> &RawValue {
        &self.raw
    }
}

impl std::ops::Deref for EventPayload {
    type Target = Payload;

    fn deref(&self) -> &Payload {
        &self.typed
    }
}

impl PartialEq for EventPayload {
    fn eq(&self, other: &Self) -> bool {
        self.raw.get() == other.raw.get()
    }
}

impl Serialize for EventPayload {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.raw.serialize(s)
    }
}

impl<'de> Deserialize<'de> for EventPayload {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = Box::<RawValue>::deserialize(d)?;
        Self::from_raw(SCHEMA_VERSION, raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PayloadError {
    #[error("payload schema {0} is newer than this build supports ({SCHEMA_VERSION})")]
    NewerSchema(u16),
    #[error("decoding payload: {0}")]
    Decode(serde_json::Error),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub run_id: RunId,
    pub seq: Seq,
    pub epoch: Epoch,
    pub recorded_at: Timestamp,
    pub schema: u16,
    pub prev_hash: Option<EventHash>,
    pub hash: EventHash,
    pub payload: EventPayload,
}

impl Event {
    /// Builds an event from a payload written by this build and computes its
    /// hash over everything but the hash.
    pub fn seal(
        run_id: RunId,
        seq: Seq,
        epoch: Epoch,
        recorded_at: Timestamp,
        prev_hash: Option<EventHash>,
        payload: Payload,
    ) -> Result<Self, serde_json::Error> {
        let payload = EventPayload::encode(payload)?;
        let hash = compute_hash(
            run_id,
            seq,
            epoch,
            recorded_at,
            SCHEMA_VERSION,
            prev_hash,
            payload.raw(),
        );
        Ok(Self {
            run_id,
            seq,
            epoch,
            recorded_at,
            schema: SCHEMA_VERSION,
            prev_hash,
            hash,
            payload,
        })
    }

    pub fn recompute_hash(&self) -> EventHash {
        compute_hash(
            self.run_id,
            self.seq,
            self.epoch,
            self.recorded_at,
            self.schema,
            self.prev_hash,
            self.payload.raw(),
        )
    }
}

fn compute_hash(
    run_id: RunId,
    seq: Seq,
    epoch: Epoch,
    recorded_at: Timestamp,
    schema: u16,
    prev_hash: Option<EventHash>,
    payload: &RawValue,
) -> EventHash {
    let mut h = Sha256::new();
    h.update(HASH_DOMAIN);
    h.update(run_id.0.as_bytes());
    h.update(seq.0.to_be_bytes());
    h.update(epoch.0.to_be_bytes());
    h.update(recorded_at.0.to_be_bytes());
    h.update(schema.to_be_bytes());
    match prev_hash {
        Some(p) => {
            h.update([1u8]);
            h.update(p.0);
        }
        None => h.update([0u8]),
    }
    h.update(payload.get().as_bytes());
    EventHash(h.finalize().into())
}
