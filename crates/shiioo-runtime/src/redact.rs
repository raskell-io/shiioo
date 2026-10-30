//! Keeps secret values out of the record.
//!
//! A harness runs its own tools, so a secret handed to it (an injected
//! `GH_TOKEN`, the office's own API token) can show up in a command's
//! output or in the model's text. Before anything is appended to a run's
//! log, every string in it is searched for the known values, raw and in
//! the encodings a shell or a URL produces, and each occurrence becomes
//! `[secret:<name>]`. One `SecretRedacted` event per affected secret is
//! appended after the redacted events, naming the secret, never the value.
//!
//! Best effort by design (`docs/design/security.md` §4): a value that
//! appears transformed in a way this module does not recognize is not
//! caught.

use serde_json::Value;
use shiioo_core::Payload;
use std::collections::BTreeMap;

/// Values shorter than this are not redacted: a three-letter secret would
/// match ordinary text everywhere.
pub const MIN_LEN: usize = 6;

#[derive(Clone, Debug, Default)]
pub struct Redactor {
    /// Needle → secret name, longest needles first so that a value which
    /// contains another is replaced as a whole.
    needles: Vec<(String, String)>,
}

impl Redactor {
    /// Builds a redactor for `(name, value)` pairs.
    pub fn new(secrets: impl IntoIterator<Item = (String, String)>) -> Self {
        let mut needles = Vec::new();
        for (name, value) in secrets {
            if value.chars().count() < MIN_LEN {
                continue;
            }
            for form in [value.clone(), base64(value.as_bytes()), url_encode(&value)] {
                if form.len() >= MIN_LEN && !needles.iter().any(|(n, _)| *n == form) {
                    needles.push((form, name.clone()));
                }
            }
        }
        needles.sort_by(|a, b| b.0.len().cmp(&a.0.len()).then_with(|| a.0.cmp(&b.0)));
        Self { needles }
    }

    pub fn is_empty(&self) -> bool {
        self.needles.is_empty()
    }

    /// Replaces every known value in `s`, counting replacements per secret.
    pub fn redact_str(&self, s: &str, counts: &mut BTreeMap<String, u32>) -> String {
        if self.needles.is_empty() || s.is_empty() {
            return s.to_owned();
        }
        let mut out = s.to_owned();
        for (needle, name) in &self.needles {
            let n = out.matches(needle.as_str()).count();
            if n > 0 {
                out = out.replace(needle.as_str(), &format!("[secret:{name}]"));
                *counts.entry(name.clone()).or_default() += n as u32;
            }
        }
        out
    }

    /// Redacts every string inside a JSON value, including object keys.
    pub fn redact_value(&self, v: &mut Value, counts: &mut BTreeMap<String, u32>) {
        match v {
            Value::String(s) => {
                let redacted = self.redact_str(s, counts);
                if redacted != *s {
                    *s = redacted;
                }
            }
            Value::Array(items) => items.iter_mut().for_each(|i| self.redact_value(i, counts)),
            Value::Object(map) => {
                let keys: Vec<String> = map.keys().cloned().collect();
                for key in keys {
                    let new_key = self.redact_str(&key, counts);
                    if let Some(mut value) = map.remove(&key) {
                        self.redact_value(&mut value, counts);
                        map.insert(new_key, value);
                    }
                }
            }
            _ => {}
        }
    }

    /// Redacts the payloads about to be appended and adds one
    /// `SecretRedacted` per secret that was found, before a terminal event
    /// if the batch ends with one (nothing may follow it). Payloads that
    /// contain no secret are returned unchanged (byte for byte: they are
    /// not re-encoded).
    pub fn redact_payloads(&self, payloads: Vec<Payload>) -> Vec<Payload> {
        if self.needles.is_empty() {
            return payloads;
        }
        let mut counts = BTreeMap::new();
        let mut out = Vec::with_capacity(payloads.len() + 1);
        for payload in payloads {
            out.push(self.redact_payload(payload, &mut counts));
        }
        let terminal = out.pop_if(|p| p.is_terminal());
        for (name, count) in counts {
            out.push(Payload::SecretRedacted { name, count });
        }
        out.extend(terminal);
        out
    }

    fn redact_payload(&self, payload: Payload, counts: &mut BTreeMap<String, u32>) -> Payload {
        let Ok(mut value) = serde_json::to_value(&payload) else {
            return payload;
        };
        let before = counts.values().sum::<u32>();
        self.redact_value(&mut value, counts);
        if counts.values().sum::<u32>() == before {
            return payload;
        }
        serde_json::from_value(value).unwrap_or(payload)
    }
}

/// Standard base64 with padding, as `base64` and most languages produce.
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Percent-encodes everything but RFC 3986 unreserved characters, with
/// upper-case hex digits (what `curl --data-urlencode` and browsers emit).
pub fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn redactor() -> Redactor {
        Redactor::new([
            ("gh".to_owned(), "ghp_SECRETtoken123".to_owned()),
            ("pw".to_owned(), "p@ss w0rd!".to_owned()),
            ("short".to_owned(), "abc".to_owned()),
        ])
    }

    #[test]
    fn replaces_raw_base64_and_url_encoded_forms() {
        let r = redactor();
        let mut counts = BTreeMap::new();
        let s = format!(
            "token={} b64={} url={} plain",
            "ghp_SECRETtoken123",
            base64(b"ghp_SECRETtoken123"),
            url_encode("p@ss w0rd!")
        );
        let out = r.redact_str(&s, &mut counts);
        assert_eq!(
            out,
            "token=[secret:gh] b64=[secret:gh] url=[secret:pw] plain"
        );
        assert_eq!(counts.get("gh"), Some(&2));
        assert_eq!(counts.get("pw"), Some(&1));
    }

    #[test]
    fn short_values_are_left_alone() {
        let r = redactor();
        let mut counts = BTreeMap::new();
        assert_eq!(r.redact_str("abc abc", &mut counts), "abc abc");
        assert!(counts.is_empty());
    }

    #[test]
    fn redacts_inside_payloads_and_appends_the_events() {
        let r = redactor();
        let payloads = vec![
            Payload::ToolResult {
                call_id: shiioo_core::CallId("c1".into()),
                output: json!({"stdout": "GH_TOKEN=ghp_SECRETtoken123\n", "nested": ["p@ss w0rd!"]}),
                is_error: false,
            },
            Payload::RunFailed {
                reason: "nothing secret here".into(),
            },
        ];
        let out = r.redact_payloads(payloads);
        assert_eq!(out.len(), 4, "{out:?}");
        match &out[0] {
            Payload::ToolResult { output, .. } => {
                assert_eq!(output["stdout"], "GH_TOKEN=[secret:gh]\n");
                assert_eq!(output["nested"][0], "[secret:pw]");
            }
            other => panic!("{other:?}"),
        }
        // The redaction events come before the terminal event.
        assert!(matches!(&out[1], Payload::SecretRedacted { name, count: 1 } if name == "gh"));
        assert!(matches!(&out[2], Payload::SecretRedacted { name, count: 1 } if name == "pw"));
        assert!(
            matches!(&out[3], Payload::RunFailed { reason } if reason == "nothing secret here")
        );
    }

    #[test]
    fn redaction_events_go_before_a_terminal_event() {
        let r = redactor();
        let out = r.redact_payloads(vec![Payload::RunCompleted {
            output: json!("done, token ghp_SECRETtoken123"),
        }]);
        assert_eq!(out.len(), 2);
        assert!(matches!(&out[0], Payload::SecretRedacted { name, count: 1 } if name == "gh"));
        assert!(
            matches!(&out[1], Payload::RunCompleted { output } if output == "done, token [secret:gh]")
        );
    }

    #[test]
    fn empty_redactor_is_a_no_op() {
        let r = Redactor::new(Vec::<(String, String)>::new());
        assert!(r.is_empty());
        let payloads = vec![Payload::RunFailed { reason: "x".into() }];
        assert_eq!(r.redact_payloads(payloads.clone()), payloads);
    }

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(url_encode("a b/c~"), "a%20b%2Fc~");
    }
}
