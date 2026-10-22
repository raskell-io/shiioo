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
    RunCompleted {
        output: Value,
    },
    RunFailed {
        reason: String,
    },
}

impl Payload {
    /// Nothing may be appended after a terminal event.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Payload::RunCompleted { .. } | Payload::RunFailed { .. }
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
