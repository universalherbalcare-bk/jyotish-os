//! Content-addressed in-memory cache. Keys are `sha256(tool || canonical
//! request JSON || kernel sha256)`, so a different kernel can never serve a
//! stale result. Bounded: when full, the whole map is cleared (simple, O(1)
//! amortised, no LRU bookkeeping; natal results are cheap to recompute).

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

pub struct Cache {
    map: Mutex<HashMap<String, Value>>,
    cap: usize,
}

impl Cache {
    pub fn new(cap: usize) -> Self {
        Self {
            map: Mutex::new(HashMap::new()),
            cap: cap.max(1),
        }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.map.lock().ok()?.get(key).cloned()
    }

    pub fn put(&self, key: String, value: Value) {
        if let Ok(mut m) = self.map.lock() {
            if m.len() >= self.cap {
                m.clear();
            }
            m.insert(key, value);
        }
    }

    pub fn len(&self) -> usize {
        self.map.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Canonical request key. `serde_json::Value` objects are `BTreeMap`-backed
/// (the `preserve_order` feature is not enabled), so serialisation already
/// yields sorted keys and therefore a canonical form.
pub fn request_key(tool: &str, args: &Value, kernel_sha256: &str) -> String {
    let canonical = serde_json::to_string(args).unwrap_or_default();
    let mut h = Sha256::new();
    h.update(tool.as_bytes());
    h.update(b"\n");
    h.update(canonical.as_bytes());
    h.update(b"\n");
    h.update(kernel_sha256.as_bytes());
    hex(&h.finalize())
}

pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_is_order_independent_and_kernel_bound() {
        let a = json!({"lat": 1.0, "lon": 2.0});
        let b = json!({"lon": 2.0, "lat": 1.0});
        assert_eq!(request_key("t", &a, "k"), request_key("t", &b, "k"));
        assert_ne!(request_key("t", &a, "k1"), request_key("t", &a, "k2"));
        assert_ne!(request_key("t1", &a, "k"), request_key("t2", &a, "k"));
        assert_eq!(request_key("t", &a, "k").len(), 64);
    }

    #[test]
    fn cache_bounded_and_roundtrips() {
        let c = Cache::new(2);
        c.put("a".into(), json!(1));
        c.put("b".into(), json!(2));
        assert_eq!(c.get("a"), Some(json!(1)));
        c.put("c".into(), json!(3));
        assert!(c.len() <= 2);
        assert_eq!(c.get("c"), Some(json!(3)));
    }
}
