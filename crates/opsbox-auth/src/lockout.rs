//! Sign-in lockout policy (docs/BACKEND_SPEC.md section 6.2): 5 failures in
//! 15 minutes, per user name and per address, then an exponential backoff.

/// Failures are counted over this many seconds.
pub const WINDOW_SECS: i64 = 15 * 60;
/// The failure that starts the lockout.
pub const THRESHOLD: u32 = 5;
const FIRST_LOCK_SECS: i64 = 30;
const MAX_LOCK_SECS: i64 = WINDOW_SECS;

/// How long a lock lasts after `failures` failures in the window: none below the
/// threshold, then 30 s doubling with each further failure, capped at 15 minutes.
pub fn lock_secs(failures: u32) -> i64 {
    if failures < THRESHOLD {
        return 0;
    }
    let doublings = (failures - THRESHOLD).min(10);
    (FIRST_LOCK_SECS << doublings).min(MAX_LOCK_SECS)
}

/// Seconds still to wait, if locked. `newest` is the time of the latest failure.
pub fn remaining(failures: u32, newest: Option<i64>, now: i64) -> Option<i64> {
    let newest = newest?;
    let left = newest + lock_secs(failures) - now;
    (lock_secs(failures) > 0 && left > 0).then_some(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_happens_below_the_threshold() {
        for n in 0..THRESHOLD {
            assert_eq!(lock_secs(n), 0);
            assert_eq!(remaining(n, Some(1000), 1000), None);
        }
    }

    #[test]
    fn backoff_doubles_and_is_capped() {
        assert_eq!(lock_secs(5), 30);
        assert_eq!(lock_secs(6), 60);
        assert_eq!(lock_secs(7), 120);
        assert_eq!(lock_secs(9), 480);
        assert_eq!(lock_secs(10), 900);
        assert_eq!(lock_secs(11), 900);
        assert_eq!(lock_secs(10_000), 900, "no overflow however many failures");
    }

    #[test]
    fn remaining_counts_down_from_the_newest_failure() {
        assert_eq!(remaining(5, Some(1000), 1000), Some(30));
        assert_eq!(remaining(5, Some(1000), 1029), Some(1));
        assert_eq!(remaining(5, Some(1000), 1030), None);
        assert_eq!(remaining(6, Some(1000), 1030), Some(30));
        assert_eq!(remaining(5, None, 1000), None);
    }
}
