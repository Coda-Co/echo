//! Adaptive sync interval: fast while things are changing, slower when quiet.

use std::time::{Duration, Instant};

pub const NETWORK_BACKOFF: Duration = Duration::from_secs(60);
const QUIET_AFTER: Duration = Duration::from_secs(10 * 60);
const IDLE_AFTER: Duration = Duration::from_secs(60 * 60);
const QUIET_INTERVAL: Duration = Duration::from_secs(15);
const IDLE_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct AdaptiveInterval {
    last_activity: Instant,
    network_backoff: bool,
}

impl AdaptiveInterval {
    pub fn new(now: Instant) -> Self {
        Self { last_activity: now, network_backoff: false }
    }

    /// Any change, branch switch, window focus or wake resets to the base interval.
    pub fn reset(&mut self, now: Instant) {
        self.last_activity = now;
    }

    pub fn set_network_backoff(&mut self, on: bool) {
        self.network_backoff = on;
    }

    pub fn network_backoff(&self) -> bool {
        self.network_backoff
    }

    /// Interval to wait before the next tick.
    pub fn current(&self, base: Duration, now: Instant) -> Duration {
        if self.network_backoff {
            return NETWORK_BACKOFF.max(base);
        }
        let quiet = now.saturating_duration_since(self.last_activity);
        if quiet >= IDLE_AFTER {
            IDLE_INTERVAL.max(base)
        } else if quiet >= QUIET_AFTER {
            QUIET_INTERVAL.max(base)
        } else {
            base
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_down_and_resets() {
        let t0 = Instant::now();
        let base = Duration::from_secs(4);
        let mut iv = AdaptiveInterval::new(t0);
        assert_eq!(iv.current(base, t0), base);
        assert_eq!(iv.current(base, t0 + Duration::from_secs(599)), base);
        assert_eq!(iv.current(base, t0 + Duration::from_secs(600)), Duration::from_secs(15));
        assert_eq!(iv.current(base, t0 + Duration::from_secs(3600)), Duration::from_secs(30));
        iv.reset(t0 + Duration::from_secs(3600));
        assert_eq!(iv.current(base, t0 + Duration::from_secs(3601)), base);
    }

    #[test]
    fn network_backoff_overrides() {
        let t0 = Instant::now();
        let mut iv = AdaptiveInterval::new(t0);
        iv.set_network_backoff(true);
        assert_eq!(iv.current(Duration::from_secs(4), t0), Duration::from_secs(60));
        iv.set_network_backoff(false);
        assert_eq!(iv.current(Duration::from_secs(4), t0), Duration::from_secs(4));
    }

    #[test]
    fn base_larger_than_tier_wins() {
        let t0 = Instant::now();
        let iv = AdaptiveInterval::new(t0);
        assert_eq!(iv.current(Duration::from_secs(20), t0 + Duration::from_secs(700)), Duration::from_secs(20));
    }
}
