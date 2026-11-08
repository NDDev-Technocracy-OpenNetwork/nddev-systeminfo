//! Per-device baselines: hotplug and resets never subtract one device's
//! lifetime counter from another's. Read failures retain the elapsed interval
//! of the last successful observation rather than inflating the next rate.

use super::{delta, rate};
use std::time::Instant;

#[derive(Default)]
pub(super) struct Counter {
    previous: Option<(u64, u64, Instant)>,
}

impl Counter {
    pub(super) fn observe(&mut self, first: u64, second: u64, now: Instant) -> Option<(u64, u64)> {
        let (old_first, old_second, then) = self.previous.replace((first, second, now))?;
        let seconds = now.duration_since(then).as_secs_f64();
        Some((
            rate(delta(first, old_first), seconds),
            rate(delta(second, old_second), seconds),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn first_observation_and_counter_resets_are_not_lifetime_traffic() {
        let now = Instant::now();
        let mut counter = Counter::default();
        assert_eq!(counter.observe(1_000_000, 2_000_000, now), None);
        assert_eq!(
            counter.observe(3, 5, now + Duration::from_secs(1)),
            Some((0, 0))
        );
        assert_eq!(
            counter.observe(103, 205, now + Duration::from_secs(3)),
            Some((50, 100))
        );
    }

    #[test]
    fn an_unobserved_tick_does_not_shorten_the_next_interval() {
        let now = Instant::now();
        let mut counter = Counter::default();
        counter.observe(0, 0, now);
        assert_eq!(
            counter.observe(100, 200, now + Duration::from_secs(2)),
            Some((50, 100))
        );
    }
}
