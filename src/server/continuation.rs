//! Transient recovery of the latest completed WS window. Never stored or logged.
use serde_json::Value;
use std::{io, io::Read, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

pub(super) const TOTAL_BYTES: usize = 64 * 1024 * 1024;
// Recovery is not input admission. Large windows are compressed in memory,
// rather than requiring client-side full reconstruction on every tool turn.
const WINDOW_BYTES: usize = 16 * 1024 * 1024;
const DECODED_BYTES: usize = 64 * 1024 * 1024;
const ITEMS: usize = 16384;

pub(super) struct Window {
    response: String,
    input: History,
    decoded_bytes: usize,
    _bytes: OwnedSemaphorePermit,
}

enum History {
    Plain(Vec<Value>),
    Compressed(Vec<u8>),
}

impl History {
    fn decode(&self, bytes: usize) -> Option<Vec<Value>> {
        match self {
            Self::Plain(input) => Some(input.clone()),
            Self::Compressed(input) => Self::decompress(input, bytes),
        }
    }

    fn into_input(self, bytes: usize) -> Option<Vec<Value>> {
        match self {
            Self::Plain(input) => Some(input),
            Self::Compressed(input) => Self::decompress(&input, bytes),
        }
    }

    fn decompress(input: &[u8], bytes: usize) -> Option<Vec<Value>> {
        if bytes > DECODED_BYTES {
            return None;
        }
        let mut decoder = zstd::stream::read::Decoder::new(input).ok()?;
        decoder.window_log_max(23).ok()?;
        // The serialized size is measured before encoding and is never supplied
        // by a client. Still bound decompression independently of encoded size.
        let mut decoded = Vec::new();
        decoded.try_reserve_exact(bytes + 1).ok()?;
        decoder
            .take(bytes as u64 + 1)
            .read_to_end(&mut decoded)
            .ok()?;
        if decoded.len() != bytes {
            return None;
        }
        let input: Vec<Value> = serde_json::from_slice(&decoded).ok()?;
        (input.len() <= ITEMS).then_some(input)
    }
}

struct Encoded(Vec<u8>);
impl io::Write for Encoded {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > WINDOW_BYTES {
            return Err(io::Error::other("compressed continuation window too large"));
        }
        let needed = self.0.len() + bytes.len();
        if needed > self.0.capacity() {
            self.0
                .try_reserve_exact(needed.next_power_of_two().min(WINDOW_BYTES) - self.0.len())
                .map_err(|_| io::Error::other("continuation memory unavailable"))?;
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

// Count without allocating another serialized history or accepting expansion.
struct Size(usize);
impl io::Write for Size {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        if self.0 > DECODED_BYTES {
            return Err(io::Error::other("continuation window too large"));
        }
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Window {
    pub(super) fn expand(&self, payload: &mut Value) -> bool {
        if payload
            .get("previous_response_id")
            .is_none_or(Value::is_null)
        {
            return true;
        }
        if payload["previous_response_id"].as_str() != Some(&self.response) {
            return false;
        }
        let Some(delta) = payload["input"].as_array() else {
            return false;
        };
        let mut size = Size(0);
        if serde_json::to_writer(&mut size, delta).is_err()
            || self.decoded_bytes.saturating_add(size.0) > DECODED_BYTES
        {
            return false;
        }
        let Some(mut input) = self.input.decode(self.decoded_bytes) else {
            return false;
        };
        if input.len().saturating_add(delta.len()) > ITEMS {
            return false;
        }
        input.extend(delta.iter().cloned());
        let body = payload.as_object_mut().expect("validated request");
        let original_input = body.insert("input".into(), Value::Array(input));
        let previous = body.remove("previous_response_id");
        if Self::fits(payload) {
            return true;
        }
        // Failed soft-threshold recovery must leave the original request usable.
        let body = payload.as_object_mut().expect("validated request");
        if let Some(input) = original_input {
            body.insert("input".into(), input);
        }
        if let Some(previous) = previous {
            body.insert("previous_response_id".into(), previous);
        }
        false
    }

    pub(super) fn fits(payload: &Value) -> bool {
        serde_json::to_writer(&mut Size(0), payload).is_ok()
    }

    pub(super) fn completed(
        old: Option<Self>,
        payload: &mut Value,
        response: &mut Value,
        budget: &Arc<Semaphore>,
    ) -> Option<Self> {
        let id = response["id"].as_str()?.to_owned();
        let input = payload.get_mut("input")?.take();
        let mut input = match input {
            Value::Array(items) => items,
            Value::String(text) => vec![serde_json::json!({"role":"user","content":text})],
            _ => return None,
        };
        let mut combined = if let Some(previous) = payload["previous_response_id"].as_str() {
            let old = old?;
            if old.response != previous {
                return None;
            }
            let Window {
                input,
                decoded_bytes,
                _bytes,
                ..
            } = old;
            drop(_bytes);
            input.into_input(decoded_bytes)?
        } else {
            drop(old);
            Vec::new()
        };
        let output = response.get_mut("output")?.take();
        let Value::Array(mut output) = output else {
            return None;
        };
        combined.append(&mut input);
        combined.append(&mut output);
        // A newly issued routing receipt is present in the dispatch-side input
        // and the downstream output. Retain it once, as the client does.
        let mut receipts = std::collections::BTreeSet::new();
        combined.retain(|item| {
            use sha2::Digest;
            !crate::affinity::is_transfer_item(item)
                || receipts.insert(<[u8; 32]>::from(sha2::Sha256::digest(
                    item["encrypted_content"]
                        .as_str()
                        .unwrap_or_default()
                        .as_bytes(),
                )))
        });
        if combined.len() > ITEMS {
            return None;
        }
        let mut size = Size(0);
        serde_json::to_writer(&mut size, &combined).ok()?;
        let (input, retained_bytes) = if size.0 <= WINDOW_BYTES {
            (History::Plain(combined), size.0)
        } else {
            let mut encoder = zstd::stream::write::Encoder::new(Encoded(Vec::new()), 1).ok()?;
            encoder.window_log(23).ok()?;
            serde_json::to_writer(&mut encoder, &combined).ok()?;
            let encoded = encoder.finish().ok()?.0;
            let retained = encoded.capacity();
            (History::Compressed(encoded), retained)
        };
        // Old reservation is released before reserving the new complete window.
        let permit = budget
            .clone()
            .try_acquire_many_owned(retained_bytes.max(1) as u32)
            .ok()?;
        Some(Self {
            response: id,
            input,
            decoded_bytes: size.0,
            _bytes: permit,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn large_windows_compress_in_memory_and_expand_exact_history_with_bounded_reservations() {
        let budget = Arc::new(Semaphore::new(TOTAL_BYTES));
        let text = "synthetic large history 🌍 ".repeat(700_000);
        let mut request = json!({"input":[{"role":"user","content":text}]});
        let mut response = json!({"id":"large","output":[{"type":"function_call","call_id":"tool","arguments":"{}"}]});
        let window = Window::completed(None, &mut request, &mut response, &budget).unwrap();
        assert!(window.decoded_bytes > WINDOW_BYTES);
        assert!(matches!(window.input, History::Compressed(_)));
        assert!(budget.available_permits() > TOTAL_BYTES - WINDOW_BYTES);
        let mut delta = json!({"previous_response_id":"large","input":[{"type":"function_call_output","call_id":"tool","output":"synthetic result"}]});
        assert!(window.expand(&mut delta));
        assert!(delta.get("previous_response_id").is_none());
        assert_eq!(delta["input"].as_array().unwrap().len(), 3);
        assert_eq!(delta["input"][0]["content"], text);
        assert_eq!(delta["input"][1]["call_id"], delta["input"][2]["call_id"]);
        assert_eq!(
            budget.available_permits() + window._bytes.num_permits(),
            TOTAL_BYTES
        );
        let History::Compressed(encoded) = &window.input else {
            unreachable!()
        };
        assert!(History::decompress(&encoded[..encoded.len() / 2], window.decoded_bytes).is_none());
        assert!(History::decompress(encoded, DECODED_BYTES + 1).is_none());
        assert!(History::decompress(encoded, window.decoded_bytes - 1).is_none());
        drop(window);
        assert_eq!(budget.available_permits(), TOTAL_BYTES);
        let tiny = Arc::new(Semaphore::new(1));
        assert!(Window::completed(
            None,
            &mut delta,
            &mut json!({"id":"next","output":[]}),
            &tiny
        )
        .is_none());
        assert_eq!(tiny.available_permits(), 1);
    }

    #[test]
    fn compressed_storage_cannot_grow_past_the_encoded_window_bound() {
        use io::Write;
        let mut buffer = Encoded(vec![0; WINDOW_BYTES]);
        assert!(buffer.write_all(b"x").is_err());
        assert_eq!(buffer.0.len(), WINDOW_BYTES);
    }

    #[test]
    fn oversized_expansion_leaves_original_incremental_request_unchanged() {
        let budget = Arc::new(Semaphore::new(WINDOW_BYTES));
        let mut request = json!({"input":[{"role":"user","content":"synthetic"}]});
        let mut response = json!({"id":"a","output":[]});
        let window = Window::completed(None, &mut request, &mut response, &budget).unwrap();
        let mut delta =
            json!({"previous_response_id":"a","input":[],"instructions":"x".repeat(DECODED_BYTES)});
        let before = delta.clone();
        assert!(!window.expand(&mut delta));
        assert_eq!(delta, before);
    }
    #[test]
    fn latest_delta_recovers_complete_tool_history_and_releases_budget() {
        let budget = Arc::new(Semaphore::new(4096));
        let mut request = json!({"input":[{"role":"user","content":"remember"}]});
        let mut response =
            json!({"id":"a","output":[{"type":"function_call","call_id":"tool","arguments":"{}"}]});
        let window = Window::completed(None, &mut request, &mut response, &budget).unwrap();
        let mut delta = json!({"previous_response_id":"a","input":[{"type":"function_call_output","call_id":"tool","output":"done"}]});
        assert!(window.expand(&mut delta));
        assert!(delta.get("previous_response_id").is_none());
        assert_eq!(delta["input"].as_array().unwrap().len(), 3);
        assert_eq!(delta["input"][1]["call_id"], delta["input"][2]["call_id"]);
        assert!(!window.expand(&mut json!({"previous_response_id":"older","input":[]})));
        drop(window);
        assert_eq!(budget.available_permits(), 4096);
        assert!(Window::completed(
            None,
            &mut json!({"input":[{"content":"x".repeat(4096)}]}),
            &mut json!({"id":"b","output":[]}),
            &budget
        )
        .is_none());
        assert_eq!(budget.available_permits(), 4096);
    }
}
