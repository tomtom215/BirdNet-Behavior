//! Per-window inference time, with its spread (audit MMP-13).
//!
//! [`bench`] times a classifier on one window, many times, after a warm-up,
//! and [`TimingSummary`] reports the distribution: a median alone hides the
//! slow tail a Pi under thermal throttling produces, and a mean alone hides
//! everything. The numbers describe the machine they ran on, under the load
//! it had; a run on a development machine says nothing about a Pi. **No Pi
//! measurement has been made with this.**

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::inference::classifier::Classifier;
use crate::inference::model::InferenceError;

/// The distribution of a set of durations, in milliseconds.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimingSummary {
    /// How many were timed.
    pub n: usize,
    /// Fastest.
    pub min_ms: f64,
    /// 10th percentile.
    pub p10_ms: f64,
    /// Median.
    pub median_ms: f64,
    /// 90th percentile.
    pub p90_ms: f64,
    /// 99th percentile.
    pub p99_ms: f64,
    /// Slowest.
    pub max_ms: f64,
    /// Arithmetic mean.
    pub mean_ms: f64,
    /// Interquartile range (p75 - p25).
    pub iqr_ms: f64,
}

impl TimingSummary {
    /// Summarise `samples`; `None` when there are none.
    ///
    /// Percentiles are nearest-rank on the sorted samples (rank
    /// `ceil(q * n)`), so every reported value is a time that was measured.
    #[must_use]
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    pub fn from_durations(samples: &[Duration]) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        let mut ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1e3).collect();
        ms.sort_by(f64::total_cmp);
        let n = ms.len();
        let rank = |q: f64| ms[((q * n as f64).ceil() as usize).clamp(1, n) - 1];
        Some(Self {
            n,
            min_ms: ms[0],
            p10_ms: rank(0.10),
            median_ms: rank(0.50),
            p90_ms: rank(0.90),
            p99_ms: rank(0.99),
            max_ms: ms[n - 1],
            mean_ms: ms.iter().sum::<f64>() / n as f64,
            iqr_ms: rank(0.75) - rank(0.25),
        })
    }
}

/// Run `classifier` on `window` `warmup` times untimed, then `runs` times
/// timed, and return each timed run's duration.
///
/// # Errors
///
/// The first [`InferenceError`]: a benchmark of a failing classifier would
/// time its error path.
pub fn bench(
    classifier: &mut dyn Classifier,
    window: &[f32],
    warmup: usize,
    runs: usize,
) -> Result<Vec<Duration>, InferenceError> {
    for _ in 0..warmup {
        classifier.infer(window)?;
    }
    let mut out = Vec::with_capacity(runs);
    for _ in 0..runs {
        let t = Instant::now();
        let scores = classifier.infer(window)?;
        out.push(t.elapsed());
        std::hint::black_box(scores);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_reports_measured_times_and_their_spread() {
        let samples: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
        let s = TimingSummary::from_durations(&samples).unwrap();
        assert_eq!(s.n, 100);
        assert!((s.min_ms - 1.0).abs() < 1e-9);
        assert!((s.p10_ms - 10.0).abs() < 1e-9);
        assert!((s.median_ms - 50.0).abs() < 1e-9);
        assert!((s.p90_ms - 90.0).abs() < 1e-9);
        assert!((s.p99_ms - 99.0).abs() < 1e-9);
        assert!((s.max_ms - 100.0).abs() < 1e-9);
        assert!((s.mean_ms - 50.5).abs() < 1e-9);
        assert!((s.iqr_ms - 50.0).abs() < 1e-9, "p75 75 - p25 25");
        assert_eq!(TimingSummary::from_durations(&[]), None);
    }

    #[test]
    fn one_sample_is_every_percentile() {
        let s = TimingSummary::from_durations(&[Duration::from_millis(7)]).unwrap();
        assert!((s.p10_ms - 7.0).abs() < 1e-9 && (s.p99_ms - 7.0).abs() < 1e-9);
        assert!(s.iqr_ms.abs() < 1e-12);
    }
}
