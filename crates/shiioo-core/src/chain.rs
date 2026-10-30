use crate::event::Event;
use crate::ids::{Epoch, RunId, Seq};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ChainError {
    #[error("event at index {index} belongs to run {found}, expected {expected}")]
    WrongRun {
        index: usize,
        expected: RunId,
        found: RunId,
    },
    #[error("event at index {index} has seq {found:?}, expected {expected:?}")]
    Gap {
        index: usize,
        expected: Seq,
        found: Seq,
    },
    #[error("event {seq:?} does not link to the previous event's hash")]
    BrokenLink { seq: Seq },
    #[error("event {seq:?} content does not match its hash")]
    Tampered { seq: Seq },
    #[error("event {seq:?} has epoch {found:?}, lower than earlier {previous:?}")]
    EpochRegressed {
        seq: Seq,
        previous: Epoch,
        found: Epoch,
    },
}

/// Checks a run's full log from `Seq(0)`: contiguous, linked, unmodified,
/// and with epochs that never go backwards.
pub fn verify_chain(events: &[Event]) -> Result<(), ChainError> {
    let Some(first) = events.first() else {
        return Ok(());
    };
    let run_id = first.run_id;
    let mut prev: Option<&Event> = None;

    for (index, ev) in events.iter().enumerate() {
        if ev.run_id != run_id {
            return Err(ChainError::WrongRun {
                index,
                expected: run_id,
                found: ev.run_id,
            });
        }
        let expected = Seq(index as u64);
        if ev.seq != expected {
            return Err(ChainError::Gap {
                index,
                expected,
                found: ev.seq,
            });
        }
        if ev.prev_hash != prev.map(|p| p.hash) {
            return Err(ChainError::BrokenLink { seq: ev.seq });
        }
        if let Some(p) = prev
            && ev.epoch < p.epoch
        {
            return Err(ChainError::EpochRegressed {
                seq: ev.seq,
                previous: p.epoch,
                found: ev.epoch,
            });
        }
        let recomputed = ev.recompute_hash();
        if recomputed != ev.hash {
            return Err(ChainError::Tampered { seq: ev.seq });
        }
        prev = Some(ev);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventPayload, Payload};
    use crate::ids::Timestamp;
    use serde_json::json;
    use serde_json::value::RawValue;

    fn log(n: u64) -> Vec<Event> {
        let run_id = RunId::new();
        let mut out: Vec<Event> = Vec::new();
        for i in 0..n {
            let ev = Event::seal(
                run_id,
                Seq(i),
                Epoch(1),
                Timestamp(1_000 + i),
                out.last().map(|e| e.hash),
                Payload::ModelResponseRecorded {
                    model: "test".into(),
                    response: json!({ "i": i }),
                    context_seq: None,
                },
            );
            match ev {
                Ok(ev) => out.push(ev),
                Err(e) => panic!("seal failed: {e}"),
            }
        }
        out
    }

    #[test]
    fn valid_chain_verifies() {
        assert_eq!(verify_chain(&log(5)), Ok(()));
    }

    #[test]
    fn edited_payload_is_detected() {
        let mut events = log(3);
        events[1].payload = encode(Payload::RunFailed {
            reason: "rewritten history".into(),
        });
        assert_eq!(
            verify_chain(&events),
            Err(ChainError::Tampered { seq: Seq(1) })
        );
    }

    /// The hash covers the stored bytes, not a re-encoding: bytes that are
    /// valid but not in serde's canonical form (older writers, other
    /// languages) still verify, as long as nobody changed them.
    #[test]
    fn non_canonical_stored_bytes_verify() {
        let raw = r#"{ "reason" : "written by an older build",  "type": "run_failed" }"#;
        let payload = from_raw(raw);
        let reencoded = serde_json::to_string(&*payload).unwrap_or_default();
        assert_ne!(
            reencoded, raw,
            "test needs bytes that differ from a re-encoding"
        );

        let mut events = log(1);
        let mut ev = events[0].clone();
        ev.seq = Seq(1);
        ev.prev_hash = Some(events[0].hash);
        ev.payload = payload;
        ev.hash = ev.recompute_hash();
        events.push(ev);
        assert_eq!(verify_chain(&events), Ok(()));
    }

    /// Even a change that keeps the decoded value identical (here: only
    /// whitespace) is a change to the record, and is detected.
    #[test]
    fn reformatted_stored_bytes_are_detected() {
        let mut events = log(2);
        let pretty = serde_json::to_string_pretty(&*events[1].payload).unwrap_or_default();
        events[1].payload = from_raw(&pretty);
        assert_eq!(
            verify_chain(&events),
            Err(ChainError::Tampered { seq: Seq(1) })
        );
    }

    fn encode(p: Payload) -> EventPayload {
        EventPayload::encode(p).unwrap_or_else(|e| panic!("encode: {e}"))
    }

    fn from_raw(raw: &str) -> EventPayload {
        let raw = RawValue::from_string(raw.to_owned()).unwrap_or_else(|e| panic!("raw: {e}"));
        EventPayload::from_raw(1, raw).unwrap_or_else(|e| panic!("decode: {e}"))
    }

    #[test]
    fn dropped_event_is_detected() {
        let mut events = log(3);
        events.remove(1);
        assert!(matches!(
            verify_chain(&events),
            Err(ChainError::Gap { index: 1, .. })
        ));
    }

    #[test]
    fn hash_survives_serde_roundtrip() {
        let events = log(2);
        let json = serde_json::to_string(&events).unwrap_or_default();
        let back: Vec<Event> = serde_json::from_str(&json).unwrap_or_default();
        assert_eq!(verify_chain(&back), Ok(()));
        assert_eq!(back, events);
    }
}
