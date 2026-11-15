//! Lamport clock.
//!
//! Gives every local event a counter that is guaranteed to be larger than any
//! event this device has already seen, from itself or from peers. That yields a
//! causal order for edits without trusting phone wall clocks.

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LamportClock {
    current: u64,
}

impl LamportClock {
    pub fn starting_at(value: u64) -> Self {
        Self { current: value }
    }

    pub fn current(&self) -> u64 {
        self.current
    }

    /// Advance for a new local event and return its timestamp.
    pub fn tick(&mut self) -> u64 {
        self.current += 1;
        self.current
    }

    /// Fold in a timestamp seen on a remote envelope.
    pub fn observe(&mut self, remote: u64) {
        self.current = self.current.max(remote);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_events_are_strictly_increasing() {
        let mut clock = LamportClock::default();
        assert_eq!(clock.tick(), 1);
        assert_eq!(clock.tick(), 2);
    }

    #[test]
    fn observing_a_later_remote_event_moves_the_clock_forward() {
        let mut clock = LamportClock::starting_at(3);
        clock.observe(10);
        assert_eq!(clock.tick(), 11);

        clock.observe(2);
        assert_eq!(clock.tick(), 12);
    }
}
