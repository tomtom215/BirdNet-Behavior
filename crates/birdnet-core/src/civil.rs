//! Civil (calendar) time from a Unix timestamp, without a date crate.
//!
//! The workspace carries no `chrono`/`time` dependency and forbids `unsafe`, so
//! `localtime_r` is out of reach. Every place that needs to turn "seconds since
//! the epoch" into a year/month/day therefore hand-rolls Howard Hinnant's
//! `civil_from_days`. This module is the one implementation of that arithmetic
//! that capture code shares: the capture supervisor's schedule gate evaluates it
//! for the recording window, and the segment writer formats it into the
//! `YYYY-MM-DD-birdnet-…-HH:MM:SS` filenames every downstream consumer parses.
//! Those two must never disagree about what day it is, which is exactly what a
//! second copy would eventually let them do.
//!
//! # This module knows nothing about time zones
//!
//! [`civil_from_unix_secs`] converts a *count of seconds* into a civil date. Add
//! the local UTC offset to the timestamp before calling it and you get local
//! civil time; don't, and you get UTC. Deciding which is right — and learning
//! the machine's offset — belongs to the caller.

/// A broken-down civil date and time-of-day.
///
/// Produced by [`civil_from_unix_secs`]. Fields are calendar values, not
/// indices: `month` is `1..=12`, `day` is `1..=31`, `hour` is `0..=23`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CivilTime {
    /// Proleptic Gregorian year (e.g. `2026`).
    pub year: u32,
    /// Month of year, `1..=12`.
    pub month: u32,
    /// Day of month, `1..=31`.
    pub day: u32,
    /// Hour of day, `0..=23`.
    pub hour: u32,
    /// Minute of hour, `0..=59`.
    pub minute: u32,
    /// Second of minute, `0..=59`. Leap seconds are not represented — Unix
    /// time doesn't carry them.
    pub second: u32,
}

impl CivilTime {
    /// Minutes since midnight (`hour * 60 + minute`), the form the recording
    /// schedule and per-source quiet windows compare against.
    #[must_use]
    pub const fn minute_of_day(&self) -> u32 {
        self.hour * 60 + self.minute
    }
}

/// Convert a Unix timestamp (seconds since 1970-01-01 00:00:00) into a
/// [`CivilTime`].
///
/// Pure — it reads no clock and no time-zone data — so the date arithmetic is
/// directly property-testable. Implements Howard Hinnant's `civil_from_days`:
/// <http://howardhinnant.github.io/date_algorithms.html>.
///
/// Timestamps before the epoch saturate to `1970-01-01 00:00:00`. Nothing in
/// this project deals in pre-1970 audio, and a negative timestamp only ever
/// arises from a wildly wrong clock or a nonsensical UTC offset; clamping keeps
/// the filename formatter total instead of making every caller handle a case
/// that means "the clock is broken" either way.
// Every cast below is bounded by the algorithm: `doe` < 146_097, the
// time-of-day terms are < 86_400, and the year is >= 1970 because `secs` is
// clamped to 0 on entry.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn civil_from_unix_secs(secs: i64) -> CivilTime {
    let secs = secs.max(0);
    let days = secs / 86_400;
    let time_of_day = secs % 86_400;
    let hour = (time_of_day / 3_600) as u32;
    let minute = ((time_of_day % 3_600) / 60) as u32;
    let second = (time_of_day % 60) as u32;

    // Shift the epoch to 0000-03-01 so leap days land at the end of the cycle.
    // `days` is non-negative here, so Hinnant's proleptic negative-era branch is
    // unreachable and the division can be done directly.
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = i64::from(yoe) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };

    CivilTime {
        year: year as u32,
        month,
        day,
        hour,
        minute,
        second,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> CivilTime {
        civil_from_unix_secs(secs)
    }

    #[test]
    fn epoch_is_1970_01_01() {
        assert_eq!(
            at(0),
            CivilTime {
                year: 1970,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0
            }
        );
    }

    #[test]
    fn pre_epoch_saturates_to_the_epoch() {
        // A wildly wrong clock (or a nonsensical offset) must not wrap the
        // calendar round to year 2^32 — it clamps.
        assert_eq!(at(-1), at(0));
        assert_eq!(at(i64::MIN), at(0));
    }

    #[test]
    fn known_timestamps_round_trip() {
        // 2024-01-01 00:00:00 UTC
        assert_eq!(
            at(1_704_067_200),
            CivilTime {
                year: 2024,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0
            }
        );
        // 2026-08-12 14:03:15 UTC
        assert_eq!(
            at(1_786_543_395),
            CivilTime {
                year: 2026,
                month: 8,
                day: 12,
                hour: 14,
                minute: 3,
                second: 15
            }
        );
    }

    #[test]
    fn leap_day_is_a_real_day() {
        // 2024-02-29 12:00:00 UTC — the case an off-by-one in the era
        // arithmetic gets wrong first.
        let t = at(1_709_208_000);
        assert_eq!((t.year, t.month, t.day), (2024, 2, 29));
        assert_eq!(t.hour, 12);
        // …and 2100 is NOT a leap year (divisible by 100, not by 400):
        // 2100-02-28 23:59:59 + 1s must be 2100-03-01.
        let feb_28_2100_end = 4_107_542_399;
        assert_eq!(
            (at(feb_28_2100_end).month, at(feb_28_2100_end).day),
            (2, 28)
        );
        assert_eq!(
            (
                at(feb_28_2100_end + 1).month,
                at(feb_28_2100_end + 1).day,
                at(feb_28_2100_end + 1).year
            ),
            (3, 1, 2100)
        );
    }

    /// The last day of a 400-year era — the one date the `- doe / 146_096`
    /// correction term exists for.
    ///
    /// Hinnant's algorithm counts days from 0000-03-01, so an era runs
    /// 1600-03-01 → 2000-02-29 and the next 2000-03-01 → 2400-02-29. Only on
    /// that final day is `doe / 146_096` non-zero, so *every* other timestamp
    /// gives the same answer whether the term is subtracted or added. Without
    /// this case, 2000-02-29 silently reads as 2000-03-01 — a whole day of
    /// recordings filed under the wrong date, once every four centuries, and
    /// nothing else in the suite would notice.
    #[test]
    fn the_last_day_of_an_era_is_not_the_first_day_of_the_next() {
        // 2000-02-29 00:00:00 UTC and midday.
        for (ts, hour) in [(951_782_400_i64, 0), (951_825_600, 12)] {
            let t = at(ts);
            assert_eq!(
                (t.year, t.month, t.day, t.hour),
                (2000, 2, 29, hour),
                "the era's last day must not roll into the next era"
            );
        }
        // The day either side, to pin that this is a boundary and not an
        // off-by-one that shifted the whole calendar.
        let before = at(951_782_400 - 86_400);
        assert_eq!((before.year, before.month, before.day), (2000, 2, 28));
        let after = at(951_782_400 + 86_400);
        assert_eq!((after.year, after.month, after.day), (2000, 3, 1));
    }

    #[test]
    fn time_of_day_components_are_independent() {
        // 23:59:59 is the boundary a `/ 60` vs `% 60` mix-up destroys.
        let t = at(1_704_067_200 + 23 * 3600 + 59 * 60 + 59);
        assert_eq!((t.hour, t.minute, t.second), (23, 59, 59));
        assert_eq!((t.year, t.month, t.day), (2024, 1, 1));
        // One second later rolls the date.
        let t = at(1_704_067_200 + 86_400);
        assert_eq!((t.year, t.month, t.day), (2024, 1, 2));
        assert_eq!((t.hour, t.minute, t.second), (0, 0, 0));
    }

    #[test]
    fn minute_of_day_matches_hour_and_minute() {
        assert_eq!(at(0).minute_of_day(), 0);
        let t = at(6 * 3600 + 30 * 60);
        assert_eq!(t.minute_of_day(), 390);
        let t = at(23 * 3600 + 59 * 60 + 59);
        assert_eq!(t.minute_of_day(), 1439);
    }

    /// Walking a whole year one hour at a time must produce a strictly
    /// monotonic sequence of civil timestamps with no gaps or repeats — the
    /// cheapest way to catch an era/day-of-year boundary error anywhere in the
    /// cycle, including the 2024 leap day.
    #[test]
    fn one_year_of_hours_is_strictly_increasing() {
        let start = 1_704_067_200_i64; // 2024-01-01 UTC
        let mut prev = at(start);
        for h in 1..=(366 * 24) {
            let t = at(start + h * 3600);
            let key = |c: &CivilTime| (c.year, c.month, c.day, c.hour, c.minute, c.second);
            assert!(
                key(&t) > key(&prev),
                "civil time went backwards at hour {h}: {prev:?} -> {t:?}"
            );
            assert!(
                t.month >= 1 && t.month <= 12,
                "bad month at hour {h}: {t:?}"
            );
            assert!(t.day >= 1 && t.day <= 31, "bad day at hour {h}: {t:?}");
            prev = t;
        }
        // 366 days later (2024 is a leap year) we are back at 2025-01-01.
        assert_eq!(
            (prev.year, prev.month, prev.day, prev.hour),
            (2025, 1, 1, 0)
        );
    }
}
