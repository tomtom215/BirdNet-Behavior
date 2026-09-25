//! Query parameter types for time-series endpoints, and the bounds they are
//! held to.

// The handlers that read these exist only with analytics compiled in.
#![cfg_attr(not(feature = "analytics"), allow(dead_code))]

use serde::Deserialize;

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct HourlyQuery {
    pub(super) days: Option<u32>,
    pub(super) species: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct DailyQuery {
    pub(super) days: Option<u32>,
    pub(super) species: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct WeeklyQuery {
    pub(super) weeks: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct TrendQuery {
    pub(super) window: Option<u32>,
    pub(super) from: Option<String>,
    pub(super) to: Option<String>,
    pub(super) species: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct AnomalyQuery {
    pub(super) z: Option<f64>,
    pub(super) window: Option<u32>,
    pub(super) days: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct DiversityQuery {
    pub(super) days: Option<u32>,
    pub(super) shannon: Option<bool>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct AccumulationQuery {
    pub(super) from: Option<String>,
    pub(super) to: Option<String>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct PeakQuery {
    pub(super) window: Option<u32>,
    pub(super) hop: Option<u32>,
    pub(super) days: Option<u32>,
    pub(super) limit: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct SessionQuery {
    pub(super) gap: Option<u32>,
    pub(super) date: Option<String>,
    pub(super) days: Option<u32>,
    pub(super) limit: Option<u32>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
pub(super) struct GapsQuery {
    pub(super) date: Option<String>,
    pub(super) threshold: Option<u32>,
    pub(super) days: Option<u32>,
}

// -- Clamps --
//
// Every endpoint here is public on an open station and runs against the one
// analytics connection, behind a mutex every other analytics read waits on.
// Nothing bounded the numbers a caller could send: measured on a 4-core x86
// dev box against a year of fixture data,
// `peak-windows?window=1440&hop=1&days=365` took 139.8 s, and the dashboard's
// own analytics stalled behind it. The ceilings sit well above anything the
// pages ask for.

/// Longest look-back any endpoint accepts: ten years of station history.
pub(super) const MAX_LOOKBACK_DAYS: u32 = 3_660;
/// Longest look-back in weeks, the same ten years.
pub(super) const MAX_LOOKBACK_WEEKS: u32 = 530;
/// Most rows any endpoint returns.
pub(super) const MAX_LIMIT: u32 = 1_000;
/// Widest smoothing / baseline window, in days.
pub(super) const MAX_WINDOW_DAYS: u32 = 365;
/// Longest silence that still counts as a gap or a session break: a day.
pub(super) const MAX_GAP_MINUTES: u32 = 1_440;

/// Peak windows are the expensive query: `days × 1440 / hop` windows, each
/// range-joined to every detection it covers. The page asks for one day at a
/// 15-minute window and a 5-minute hop.
pub(super) const PEAK_MAX_DAYS: u32 = 31;
/// Shortest hop, and so the densest grid of windows, peak-windows accepts.
pub(super) const PEAK_MIN_HOP: u32 = 5;
/// Widest peak window, in minutes.
pub(super) const PEAK_MAX_WINDOW: u32 = 240;
/// Most peak windows returned.
pub(super) const PEAK_MAX_LIMIT: u32 = 100;

/// `value` (or `default`) held to `1..=max`.
pub(super) fn bounded(value: Option<u32>, default: u32, max: u32) -> u32 {
    value.unwrap_or(default).clamp(1, max)
}

/// The peak-window parameters, held to the bounds above: the window between
/// the minimum hop and [`PEAK_MAX_WINDOW`], the hop between [`PEAK_MIN_HOP`]
/// and the window, the look-back to [`PEAK_MAX_DAYS`].
pub(super) fn peak_bounds(q: &PeakQuery) -> (u32, u32, u32, u32) {
    let window = q.window.unwrap_or(15).clamp(PEAK_MIN_HOP, PEAK_MAX_WINDOW);
    let hop = q.hop.unwrap_or(5).clamp(PEAK_MIN_HOP, window);
    let days = bounded(q.days, 1, PEAK_MAX_DAYS);
    let limit = bounded(q.limit, 10, PEAK_MAX_LIMIT);
    (window, hop, days, limit)
}

/// The anomaly z threshold: finite, and between 0.5 and 10. It is formatted
/// into the query, where `NaN` or `inf` fail the statement.
pub(super) fn z_threshold(z: Option<f64>) -> f64 {
    z.filter(|v| v.is_finite()).unwrap_or(2.0).clamp(0.5, 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_peak_request_cannot_ask_for_a_year_of_one_minute_windows() {
        let q = PeakQuery {
            window: Some(1_000_000),
            hop: Some(0),
            days: Some(100_000),
            limit: Some(u32::MAX),
        };
        assert_eq!(
            peak_bounds(&q),
            (PEAK_MAX_WINDOW, PEAK_MIN_HOP, PEAK_MAX_DAYS, PEAK_MAX_LIMIT)
        );
        // The page's own request passes through untouched.
        let page = PeakQuery {
            window: Some(15),
            hop: Some(5),
            days: Some(1),
            limit: Some(10),
        };
        assert_eq!(peak_bounds(&page), (15, 5, 1, 10));
        // A hop wider than its window would leave gaps between windows.
        let wide = PeakQuery {
            window: Some(10),
            hop: Some(60),
            days: None,
            limit: None,
        };
        assert_eq!(peak_bounds(&wide).1, 10);
    }

    #[test]
    fn a_z_threshold_is_finite_and_in_range() {
        assert!((z_threshold(Some(f64::NAN)) - 2.0).abs() < f64::EPSILON);
        assert!((z_threshold(Some(f64::INFINITY)) - 2.0).abs() < f64::EPSILON);
        assert!((z_threshold(Some(0.0)) - 0.5).abs() < f64::EPSILON);
        assert!((z_threshold(Some(3.0)) - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn zero_and_huge_are_held_to_the_range() {
        assert_eq!(bounded(Some(0), 7, MAX_LOOKBACK_DAYS), 1);
        assert_eq!(
            bounded(Some(u32::MAX), 7, MAX_LOOKBACK_DAYS),
            MAX_LOOKBACK_DAYS
        );
        assert_eq!(bounded(None, 7, MAX_LOOKBACK_DAYS), 7);
    }
}
