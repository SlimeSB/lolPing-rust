use crate::room::rate::{Clock, RateLimiter};

#[derive(Clone, Copy)]
pub struct FloodLimits {
    /// Packets per second from one source address.
    pub per_ip: f64,
    /// Packets per second from all sources together: source addresses are easy to spoof on a LAN.
    pub total: f64,
    /// Most source addresses remembered; past this the per-address budgets start over, so memory stays bounded.
    pub max_tracked: usize,
}

pub const DEFAULT_FLOOD_LIMITS: FloodLimits = FloodLimits { per_ip: 100.0, total: 800.0, max_tracked: 1024 };

/// Decides, before any decryption is tried, whether a received packet is worth looking at.
pub struct FloodGuard {
    per_ip: RateLimiter,
    total: RateLimiter,
    limits: FloodLimits,
}

impl FloodGuard {
    pub fn new(now: Clock, limits: FloodLimits) -> Self {
        Self { per_ip: RateLimiter::new(now.clone()), total: RateLimiter::new(now), limits }
    }

    pub fn allow(&mut self, ip: &str) -> bool {
        if self.per_ip.len() >= self.limits.max_tracked {
            self.per_ip.clear();
        }
        // Per address first, so one noisy source doesn't use up everyone else's share.
        self.per_ip.allow(ip, self.limits.per_ip) && self.total.allow("*", self.limits.total)
    }

    pub fn tracked(&self) -> usize {
        self.per_ip.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    fn guard() -> (FloodGuard, Arc<AtomicU64>) {
        let now = Arc::new(AtomicU64::new(0));
        let clock = now.clone();
        let limits = FloodLimits { per_ip: 100.0, total: 800.0, max_tracked: 1024 };
        (FloodGuard::new(Arc::new(move || clock.load(Ordering::Relaxed)), limits), now)
    }

    #[test]
    fn limits_one_address_to_its_own_budget() {
        let (mut g, _) = guard();
        let allowed = (0..500).filter(|_| g.allow("10.0.0.9")).count();
        assert_eq!(allowed, 100);
    }

    #[test]
    fn caps_the_total_when_every_source_is_spoofed() {
        let (mut g, _) = guard();
        let allowed = (0..5000).filter(|i| g.allow(&format!("10.{}.{}.{}", (i >> 16) & 255, (i >> 8) & 255, i & 255))).count();
        assert_eq!(allowed, 800);
    }

    #[test]
    fn never_tracks_more_than_max_tracked_addresses() {
        let (mut g, now) = guard();
        for i in 0..20_000u64 {
            g.allow(&format!("ip{i}"));
            if i % 500 == 0 {
                now.fetch_add(1000, Ordering::Relaxed);
            }
        }
        assert!(g.tracked() <= 1024);
    }

    #[test]
    fn refills_over_time() {
        let (mut g, now) = guard();
        for _ in 0..100 {
            g.allow("a");
        }
        assert!(!g.allow("a"));
        now.fetch_add(1000, Ordering::Relaxed);
        assert!(g.allow("a"));
    }
}
