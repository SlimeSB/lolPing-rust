use std::collections::HashMap;
use std::sync::Arc;

/// Milliseconds since some fixed origin; injected so the bucket is testable.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

struct Bucket {
    tokens: f64,
    at: u64,
}

/// Token bucket per key: `burst` tokens (one second's worth by default), refilled continuously at `per_second`.
pub struct RateLimiter {
    now: Clock,
    buckets: HashMap<String, Bucket>,
}

impl RateLimiter {
    pub fn new(now: Clock) -> Self {
        Self { now, buckets: HashMap::new() }
    }

    pub fn allow(&mut self, key: &str, per_second: f64) -> bool {
        self.allow_burst(key, per_second, per_second)
    }

    /// `per_second` 0 or less means unlimited.
    pub fn allow_burst(&mut self, key: &str, per_second: f64, burst: f64) -> bool {
        if per_second <= 0.0 {
            return true;
        }
        let now = (self.now)();
        let bucket = self.buckets.entry(key.to_string()).or_insert(Bucket { tokens: burst, at: now });
        bucket.tokens = (bucket.tokens + (now.saturating_sub(bucket.at) as f64 / 1000.0) * per_second).min(burst);
        bucket.at = now;
        if bucket.tokens < 1.0 {
            return false;
        }
        bucket.tokens -= 1.0;
        true
    }

    pub fn forget(&mut self, key: &str) {
        self.buckets.remove(key);
    }

    pub fn len(&self) -> usize {
        self.buckets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buckets.is_empty()
    }

    pub fn clear(&mut self) {
        self.buckets.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn limiter() -> (RateLimiter, Arc<AtomicU64>) {
        let now = Arc::new(AtomicU64::new(0));
        let clock = now.clone();
        (RateLimiter::new(Arc::new(move || clock.load(Ordering::Relaxed))), now)
    }

    fn advance(now: &Arc<AtomicU64>, ms: u64) {
        now.fetch_add(ms, Ordering::Relaxed);
    }

    #[test]
    fn allows_a_burst_then_refills() {
        let (mut rl, now) = limiter();
        for _ in 0..5 {
            assert!(rl.allow("a", 5.0));
        }
        assert!(!rl.allow("a", 5.0));
        advance(&now, 200);
        assert!(rl.allow("a", 5.0));
        assert!(!rl.allow("a", 5.0));
    }

    #[test]
    fn never_stores_more_than_one_second_of_tokens() {
        let (mut rl, now) = limiter();
        assert!(rl.allow("a", 2.0));
        advance(&now, 60_000);
        assert!(rl.allow("a", 2.0));
        assert!(rl.allow("a", 2.0));
        assert!(!rl.allow("a", 2.0));
    }

    #[test]
    fn zero_is_unlimited() {
        let (mut rl, _) = limiter();
        for _ in 0..1000 {
            assert!(rl.allow("a", 0.0));
        }
    }

    #[test]
    fn burst_can_exceed_the_rate() {
        let (mut rl, now) = limiter();
        for _ in 0..10 {
            assert!(rl.allow_burst("a", 5.0, 10.0));
        }
        assert!(!rl.allow_burst("a", 5.0, 10.0));
        advance(&now, 200);
        assert!(rl.allow_burst("a", 5.0, 10.0));
        assert!(!rl.allow_burst("a", 5.0, 10.0));
    }

    #[test]
    fn keys_are_apart_and_forget_resets_one() {
        let (mut rl, _) = limiter();
        assert!(rl.allow("a", 1.0));
        assert!(rl.allow("b", 1.0));
        assert!(!rl.allow("a", 1.0));
        rl.forget("a");
        assert!(rl.allow("a", 1.0));
        assert_eq!(rl.len(), 2);
    }
}
