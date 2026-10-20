//! Say a repeating failure once, then now and again, and say when it stops.
//!
//! The background loops and the per-detection path read the database over and
//! over. When a read fails the loop must not carry on as though it had read
//! zero (that is the defect this exists to make visible), but a warning per
//! pass is its own fault: a locked database for an hour would write a line
//! every few seconds and bury everything else in the journal. [`RepeatWarning`]
//! is the rule the callers share — warn on the first failure of a run, repeat
//! at most once per `every` while it lasts, and report the recovery so an
//! operator reading the log can tell the fault from its end.

use std::time::{Duration, Instant};

/// A throttle for one repeating failure. One per thing that can fail; it
/// remembers nothing else.
#[derive(Debug, Clone)]
pub struct RepeatWarning {
    /// Least time between two warnings about the same run of failures.
    every: Duration,
    /// When the last warning of the current run was due, `None` when the last
    /// pass succeeded (or none has failed yet).
    warned_at: Option<Instant>,
    /// Failures since the last warning, so the next one can say how many were
    /// passed over.
    skipped: u32,
}

impl RepeatWarning {
    /// A throttle that warns at most once per `every` while a failure lasts.
    #[must_use]
    pub const fn new(every: Duration) -> Self {
        Self {
            every,
            warned_at: None,
            skipped: 0,
        }
    }

    /// A pass failed at `now`. `Some(n)` means *log it now*, `n` being the
    /// failures since the last warning that were not logged (zero for the
    /// first of a run); `None` means stay quiet.
    pub fn failed(&mut self, now: Instant) -> Option<u32> {
        match self.warned_at {
            Some(last) if now.saturating_duration_since(last) < self.every => {
                self.skipped = self.skipped.saturating_add(1);
                None
            }
            _ => {
                self.warned_at = Some(now);
                Some(std::mem::take(&mut self.skipped))
            }
        }
    }

    /// A pass succeeded. `true` when it ends a run of failures that was
    /// logged, so the caller can say the fault has cleared; `false` when
    /// nothing was ever said.
    pub const fn succeeded(&mut self) -> bool {
        self.skipped = 0;
        self.warned_at.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::RepeatWarning;
    use std::time::{Duration, Instant};

    const EVERY: Duration = Duration::from_secs(300);

    #[test]
    fn the_first_failure_is_logged_and_a_burst_after_it_is_not() {
        let t0 = Instant::now();
        let mut w = RepeatWarning::new(EVERY);
        assert_eq!(w.failed(t0), Some(0), "the first failure must be heard");
        for s in [1, 2, 60, 299] {
            assert_eq!(
                w.failed(t0 + Duration::from_secs(s)),
                None,
                "a repeat at +{s}s was logged"
            );
        }
    }

    #[test]
    fn it_repeats_once_the_interval_has_passed_and_counts_what_it_skipped() {
        // Counterpart: a throttle that never spoke again would hide a fault
        // that lasts for days behind one line from the first minute.
        let t0 = Instant::now();
        let mut w = RepeatWarning::new(EVERY);
        assert_eq!(w.failed(t0), Some(0));
        assert_eq!(w.failed(t0 + Duration::from_secs(10)), None);
        assert_eq!(w.failed(t0 + Duration::from_secs(20)), None);
        assert_eq!(
            w.failed(t0 + EVERY),
            Some(2),
            "the reminder must say how many failures it passed over"
        );
        assert_eq!(w.failed(t0 + EVERY + Duration::from_secs(1)), None);
    }

    #[test]
    fn a_recovery_is_reported_once_and_only_after_a_failure_was_logged() {
        let t0 = Instant::now();
        let mut w = RepeatWarning::new(EVERY);
        assert!(!w.succeeded(), "a healthy loop announced a recovery");
        assert_eq!(w.failed(t0), Some(0));
        assert!(w.succeeded());
        assert!(!w.succeeded(), "the recovery was reported twice");
    }

    #[test]
    fn a_failure_after_a_recovery_is_a_new_run_and_is_logged_at_once() {
        let t0 = Instant::now();
        let mut w = RepeatWarning::new(EVERY);
        assert_eq!(w.failed(t0), Some(0));
        assert_eq!(w.failed(t0 + Duration::from_secs(1)), None);
        assert!(w.succeeded());
        assert_eq!(
            w.failed(t0 + Duration::from_secs(2)),
            Some(0),
            "a second fault inside the first one's interval went unheard"
        );
    }
}
