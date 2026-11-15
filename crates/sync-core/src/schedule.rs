//! When to attempt a sync round.
//!
//! Battery is a product requirement, not an optimisation. A phone that is
//! alone should check for family members rarely; a phone that just made a
//! change, or just found a peer, should try again soon. This is a simple
//! exponential backoff between those two poles.

use beacon_protocol::UnixMillis;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScheduleConfig {
    /// Fastest cadence: right after a local change or a successful exchange.
    pub min_interval_ms: u64,
    /// Slowest cadence while nobody is around.
    pub max_interval_ms: u64,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            min_interval_ms: 30_000,
            max_interval_ms: 15 * 60_000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SyncSchedule {
    config: ScheduleConfig,
    interval_ms: u64,
    next_at: UnixMillis,
}

impl SyncSchedule {
    pub fn new(config: ScheduleConfig, now: UnixMillis) -> Self {
        Self {
            config,
            interval_ms: config.min_interval_ms,
            next_at: now,
        }
    }

    pub fn is_due(&self, now: UnixMillis) -> bool {
        now >= self.next_at
    }

    pub fn next_at(&self) -> UnixMillis {
        self.next_at
    }

    /// Something worth sharing happened locally: come back quickly.
    pub fn on_local_change(&mut self, now: UnixMillis) {
        self.interval_ms = self.config.min_interval_ms;
        self.next_at = self.next_at.min(now + self.interval_ms);
    }

    /// Record the result of a round.
    pub fn on_round_finished(&mut self, now: UnixMillis, exchanged_anything: bool) {
        self.interval_ms = if exchanged_anything {
            self.config.min_interval_ms
        } else {
            (self.interval_ms.saturating_mul(2)).min(self.config.max_interval_ms)
        };
        self.next_at = now + self.interval_ms;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: ScheduleConfig = ScheduleConfig {
        min_interval_ms: 1_000,
        max_interval_ms: 8_000,
    };

    #[test]
    fn backs_off_while_alone_and_caps() {
        let mut s = SyncSchedule::new(CONFIG, 0);
        assert!(s.is_due(0));

        let mut now = 0;
        let mut intervals = vec![];
        for _ in 0..5 {
            s.on_round_finished(now, false);
            intervals.push(s.next_at() - now);
            now = s.next_at();
        }
        assert_eq!(intervals, vec![2_000, 4_000, 8_000, 8_000, 8_000]);
    }

    #[test]
    fn resets_after_activity() {
        let mut s = SyncSchedule::new(CONFIG, 0);
        s.on_round_finished(0, false);
        s.on_round_finished(2_000, false);
        assert_eq!(s.next_at(), 6_000);

        s.on_local_change(3_000);
        assert_eq!(s.next_at(), 4_000);

        s.on_round_finished(4_000, true);
        assert_eq!(s.next_at(), 5_000);
    }
}
