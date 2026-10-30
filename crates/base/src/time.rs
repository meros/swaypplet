//! Aiming a timer at a minute boundary.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Past the boundary by this much, so whatever reads the clock when the
/// timer fires already sees the new minute.
const PAST: Duration = Duration::from_millis(50);

/// How long until `since` reaches its next whole minute, plus a margin
/// past it. `since` is any count that turns over each minute: the time
/// since the Unix epoch for the clock's minute (see [`now`]), or an age
/// for the minute an age label changes. Exactly on a boundary is a whole
/// minute away: the label for that minute is already up.
pub fn until_next_minute(since: Duration) -> Duration {
    const MINUTE: u128 = 60_000_000_000;
    let into = (since.as_nanos() % MINUTE) as u64;
    Duration::from_nanos(MINUTE as u64 - into) + PAST
}

/// The wall clock as time since the Unix epoch, zero before it. Local time
/// turns over its minutes at the same instants, since every zone's offset
/// is whole minutes.
pub fn now() -> Duration {
    since_epoch(SystemTime::now())
}

/// `at` as time since the Unix epoch, zero before it.
pub fn since_epoch(at: SystemTime) -> Duration {
    at.duration_since(UNIX_EPOCH).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timer_lands_just_past_the_next_minute() {
        let at = |ms: u64| until_next_minute(Duration::from_millis(ms));
        assert_eq!(at(0), Duration::from_millis(60_050));
        assert_eq!(at(10_500), Duration::from_millis(49_550));
        assert_eq!(at(59_999), Duration::from_millis(51));
        assert_eq!(at(120_000), Duration::from_millis(60_050));
        // A clock reading in 2026, well past what an f64 of seconds keeps.
        assert_eq!(
            until_next_minute(Duration::from_secs(1_793_400_000 + 45)),
            Duration::from_millis(15_050)
        );
    }

    #[test]
    fn before_the_epoch_is_zero() {
        assert_eq!(
            since_epoch(UNIX_EPOCH - Duration::from_secs(5)),
            Duration::ZERO
        );
        assert_eq!(
            since_epoch(UNIX_EPOCH + Duration::from_secs(5)),
            Duration::from_secs(5)
        );
    }
}
