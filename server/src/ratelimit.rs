//! Per-sender token-bucket rate limiting (PROTOCOL.md §7).
//! Applied to paired senders keyed by pubkey; strangers never get past
//! the allowlist check, so they don't need buckets.

use std::collections::HashMap;
use std::sync::Mutex;

/// PROTOCOL.md §7: 60 requests/minute, burst 10.
const TOKENS_PER_SEC: f64 = 1.0;
const BURST: f64 = 10.0;

struct Bucket {
    tokens: f64,
    last_refill: u64,
}

#[derive(Default)]
pub struct RateLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// Consume one token for `key`. Returns false when the bucket is
    /// empty (caller responds `rate_limited`).
    pub fn allow(&self, key: &str, now: u64) -> bool {
        let mut buckets = self.buckets.lock().unwrap();
        let b = buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: BURST,
            last_refill: now,
        });
        let elapsed = now.saturating_sub(b.last_refill) as f64;
        b.tokens = (b.tokens + elapsed * TOKENS_PER_SEC).min(BURST);
        b.last_refill = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn burst_then_deny_then_refill() {
        let rl = RateLimiter::new();
        for _ in 0..10 {
            assert!(rl.allow("pk", 1000));
        }
        assert!(!rl.allow("pk", 1000));
        // one token/sec: after 3s, 3 requests allowed
        for _ in 0..3 {
            assert!(rl.allow("pk", 1003));
        }
        assert!(!rl.allow("pk", 1003));
        // independent buckets per key
        assert!(rl.allow("other", 1003));
    }

    #[test]
    fn caps_at_burst() {
        let rl = RateLimiter::new();
        rl.allow("pk", 1000);
        // long idle: tokens cap at BURST, not 1000
        for _ in 0..10 {
            assert!(rl.allow("pk", 1000 + 3600));
        }
        assert!(!rl.allow("pk", 1000 + 3600));
    }
}
