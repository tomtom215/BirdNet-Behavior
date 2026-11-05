//! Whether each classifier is still answering.
//!
//! A loaded classifier that fails every window — a model file damaged after
//! it was loaded, an ONNX Runtime that cannot allocate, a mis-quantised model
//! that overflows on every input — used to show only in the journal: one
//! `failed to process file` line per segment, while every gauge stayed green
//! and the detection deadman waited its twenty-four hours. On a station
//! nobody reads the journal of, that is a lost day at best.
//!
//! This keeps, per classifier, the count of consecutive failed windows and
//! when it last answered, so the station-health notifier can say which model
//! stopped and since when. The policy is birdnet-go's (`e853776`): a model is
//! **failing** after [`FAILING_AFTER`] consecutive failed windows, and is
//! well again at its first good one.
//!
//! Two things fail a window:
//!
//! * the classifier returned an error ([`ErrorClass::Inference`]);
//! * it answered, but with a score that is not a finite number
//!   ([`ErrorClass::NonFiniteOutput`]). Such scores are never detections
//!   ([`super::model::compute_confidence`] maps them to zero), so a model
//!   doing this on every window is silent rather than broken-looking, which
//!   is exactly the case this module exists for.

/// Consecutive failed windows after which a classifier is reported failing.
///
/// birdnet-go's default. At the usual three-second window with no overlap
/// that is thirty seconds of audio — far more than a single transient
/// allocation failure, far less than a dawn chorus.
pub const FAILING_AFTER: u32 = 10;

/// Why a window failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorClass {
    /// The classifier returned an error.
    Inference,
    /// The classifier answered with a NaN or infinite score.
    NonFiniteOutput,
}

impl ErrorClass {
    /// The stable name used in metrics labels and API fields; birdnet-go's
    /// `errorClass` values.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inference => "inference_error",
            Self::NonFiniteOutput => "non_finite_output",
        }
    }
}

/// One classifier's record of its recent windows.
///
/// Times are seconds since the Unix epoch, supplied by the caller so the
/// policy is testable without a clock.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelHealth {
    /// Failed windows since the last good one.
    pub consecutive_failures: u32,
    /// Windows run since the classifier was loaded, good or not.
    pub windows: u64,
    /// When it last ran a window.
    pub last_window_at: Option<u64>,
    /// When it last answered a window cleanly.
    pub last_success_at: Option<u64>,
    /// Why the most recent failed window failed; cleared by a good one.
    pub error_class: Option<ErrorClass>,
    /// Microseconds the classifier spent inside `predict_chunk`, summed over
    /// the windows it answered. Wall-clock time on this machine, not a
    /// benchmark: it includes whatever else the board was doing.
    pub answered_micros: u64,
    /// How many windows `answered_micros` is the sum over.
    pub timed_windows: u64,
}

impl ModelHealth {
    /// Record a window that answered cleanly.
    pub const fn succeeded(&mut self, now: u64) {
        self.windows = self.windows.saturating_add(1);
        self.last_window_at = Some(now);
        self.last_success_at = Some(now);
        self.consecutive_failures = 0;
        self.error_class = None;
    }

    /// Record a window that failed, and why.
    pub const fn failed(&mut self, class: ErrorClass, now: u64) {
        self.windows = self.windows.saturating_add(1);
        self.last_window_at = Some(now);
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.error_class = Some(class);
    }

    /// Add one answered window's inference time to the running total.
    ///
    /// Only windows that answered are timed: a window that errored stops early,
    /// and averaging its few microseconds in would make a model that fails
    /// often look fast.
    pub fn record_inference_time(&mut self, took: std::time::Duration) {
        let micros = u64::try_from(took.as_micros()).unwrap_or(u64::MAX);
        self.answered_micros = self.answered_micros.saturating_add(micros);
        self.timed_windows = self.timed_windows.saturating_add(1);
    }

    /// The mean time per answered window since the classifier was loaded, in
    /// microseconds; `None` before it has answered one.
    #[must_use]
    pub const fn mean_inference_micros(&self) -> Option<u64> {
        match self.answered_micros.checked_div(self.timed_windows) {
            Some(mean) => Some(mean),
            None => None,
        }
    }

    /// Whether the classifier has failed enough windows in a row to say so.
    #[must_use]
    pub const fn is_failing(&self) -> bool {
        self.consecutive_failures >= FAILING_AFTER
    }
}

/// Seconds since the Unix epoch, now.
#[must_use]
pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_is_failing_at_the_threshold_and_not_before() {
        let mut h = ModelHealth::default();
        for i in 0..FAILING_AFTER - 1 {
            h.failed(ErrorClass::Inference, u64::from(i));
            assert!(!h.is_failing(), "failing after only {} windows", i + 1);
        }
        h.failed(ErrorClass::Inference, 100);
        assert!(h.is_failing());
        assert_eq!(h.consecutive_failures, FAILING_AFTER);
        assert_eq!(h.windows, u64::from(FAILING_AFTER));
        assert_eq!(h.last_window_at, Some(100));
        assert_eq!(h.last_success_at, None);
        assert_eq!(h.error_class, Some(ErrorClass::Inference));
    }

    #[test]
    fn one_good_window_ends_it_and_keeps_the_count_of_windows() {
        let mut h = ModelHealth::default();
        h.succeeded(5);
        for i in 0..FAILING_AFTER + 3 {
            h.failed(ErrorClass::NonFiniteOutput, 10 + u64::from(i));
        }
        assert!(h.is_failing());
        assert_eq!(h.last_success_at, Some(5), "a failure is not a success");
        h.succeeded(50);
        assert!(!h.is_failing());
        assert_eq!(h.consecutive_failures, 0);
        assert_eq!(h.error_class, None);
        assert_eq!(h.last_success_at, Some(50));
        assert_eq!(h.windows, u64::from(FAILING_AFTER) + 5);
    }

    #[test]
    fn a_failure_between_good_windows_does_not_accumulate() {
        let mut h = ModelHealth::default();
        for i in 0..FAILING_AFTER * 3 {
            if i % 2 == 0 {
                h.failed(ErrorClass::Inference, u64::from(i));
            } else {
                h.succeeded(u64::from(i));
            }
            assert!(!h.is_failing());
        }
    }

    #[test]
    fn error_class_names_are_upstreams() {
        assert_eq!(ErrorClass::Inference.as_str(), "inference_error");
        assert_eq!(ErrorClass::NonFiniteOutput.as_str(), "non_finite_output");
    }
    #[test]
    fn a_mean_window_time_exists_only_once_a_window_has_answered() {
        use std::time::Duration;
        let mut h = ModelHealth::default();
        assert_eq!(h.mean_inference_micros(), None, "no window yet: no number");
        h.record_inference_time(Duration::from_millis(120));
        h.record_inference_time(Duration::from_millis(80));
        assert_eq!(h.timed_windows, 2);
        assert_eq!(h.mean_inference_micros(), Some(100_000));
    }

    #[test]
    fn failing_a_window_neither_adds_to_the_time_nor_counts_as_timed() {
        use std::time::Duration;
        let mut h = ModelHealth::default();
        h.record_inference_time(Duration::from_millis(100));
        h.failed(ErrorClass::Inference, 5);
        assert_eq!((h.answered_micros, h.timed_windows), (100_000, 1));
        assert_eq!(h.windows, 1, "the failed window is still a window run");
    }
}
