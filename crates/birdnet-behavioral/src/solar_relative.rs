//! Time of day relative to each day's own sunrise (RES-17, UIR-18).
//!
//! A dawn analysis that buckets by clock hour, or measures from *today's*
//! sunrise, mixes season with behaviour: across a 60-day window near an
//! equinox at mid latitudes sunrise moves by more than an hour, and a daylight
//! saving change adds another. Here every instant is placed against the
//! sunrise of its own day ([`minutes_from_sunrise`]): the nearest sunrise to
//! the instant, from the sunrises of its UTC day and the two either side, so a
//! station far from Greenwich, whose sunrise falls on the previous UTC date,
//! is placed correctly.
//!
//! A day with no sunrise (polar day or night) places nothing, and is counted
//! ([`RelativeHistogram::no_sunrise`]); nothing is guessed from the clock.
//! What is counted is up to the caller; the analytic routes count events.

/// The method, by name.
pub const SUNRISE_RELATIVE_METHOD: &str = "minutes-from-own-day-sunrise-1";

/// Minutes from the sunrise nearest `instant` (Unix seconds), negative before it.
///
/// `sunrise(day)` gives the sunrise instant of UTC day `day` (days since
/// 1970-01-01), `None` when the sun does not rise that day. `None` when none
/// of the three days has a sunrise within twelve hours of the instant.
#[must_use]
pub fn minutes_from_sunrise(instant: i64, sunrise: impl Fn(i64) -> Option<i64>) -> Option<i64> {
    let day = instant.div_euclid(86_400);
    (day - 1..=day + 1)
        .filter_map(&sunrise)
        .map(|s| instant - s)
        .filter(|d| d.abs() <= 12 * 3600)
        .min_by_key(|d| d.abs())
        .map(|d| d.div_euclid(60))
}

/// Counts of instants by minutes from their own day's sunrise.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct RelativeHistogram {
    /// [`SUNRISE_RELATIVE_METHOD`].
    pub method: &'static str,
    /// Width of a bin, minutes.
    pub bin_minutes: i64,
    /// Start of the first bin, minutes from sunrise.
    pub from_minutes: i64,
    /// `counts[i]` covers `[from + i·bin, from + (i+1)·bin)`.
    pub counts: Vec<u64>,
    /// Instants placed outside the bins.
    pub outside: u64,
    /// Instants on a day without a sunrise near them.
    pub no_sunrise: u64,
}

/// The histogram of `instants` over `[from_minutes, to_minutes)` in bins of
/// `bin_minutes` (at least 1).
#[must_use]
pub fn histogram(
    instants: &[i64],
    sunrise: impl Fn(i64) -> Option<i64>,
    bin_minutes: i64,
    from_minutes: i64,
    to_minutes: i64,
) -> RelativeHistogram {
    let bin = bin_minutes.max(1);
    let bins = usize::try_from(((to_minutes - from_minutes).max(0) + bin - 1) / bin).unwrap_or(0);
    let mut out = RelativeHistogram {
        method: SUNRISE_RELATIVE_METHOD,
        bin_minutes: bin,
        from_minutes,
        counts: vec![0; bins],
        outside: 0,
        no_sunrise: 0,
    };
    for &instant in instants {
        match minutes_from_sunrise(instant, &sunrise) {
            None => out.no_sunrise += 1,
            Some(minutes) if minutes < from_minutes || minutes >= to_minutes => out.outside += 1,
            Some(minutes) => {
                let index = usize::try_from((minutes - from_minutes) / bin).unwrap_or(usize::MAX);
                match out.counts.get_mut(index) {
                    Some(count) => *count += 1,
                    None => out.outside += 1,
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_nearest_sunrise_is_taken_across_a_utc_date() {
        // Sunrise at 20:00 UTC each day (a station far east): an instant at
        // 19:50 UTC is ten minutes before; one at 00:10 UTC the next day is
        // 250 minutes after the previous day's sunrise.
        let sr = |d: i64| Some(d * 86_400 + 20 * 3600);
        assert_eq!(minutes_from_sunrise(19 * 3600 + 50 * 60, sr), Some(-10));
        assert_eq!(minutes_from_sunrise(86_400 + 10 * 60, sr), Some(250));
        assert_eq!(minutes_from_sunrise(0, |_| None), None);
    }
}
