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

/// Seconds since the Unix epoch for a civil date/time — the inverse of
/// [`civil_from_unix_secs`].
///
/// Implements Hinnant's `days_from_civil`. Years before 1970 saturate to the
/// epoch, mirroring the forward direction: nothing here deals in pre-1970
/// audio, and a date that far out means a broken clock either way.
#[must_use]
pub fn unix_secs_from_civil(t: &CivilTime) -> i64 {
    let y = i64::from(t.year) - i64::from(t.month <= 2);
    if y < 0 {
        return 0;
    }
    let era = y / 400;
    let yoe = y - era * 400;
    let m = i64::from(t.month);
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(t.day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    days * 86_400 + i64::from(t.hour) * 3_600 + i64::from(t.minute) * 60 + i64::from(t.second)
}

/// Parse a `YYYY-MM-DD` / `HH:MM:SS` pair into a [`CivilTime`].
///
/// Strict: both fields must be exactly the documented shape and all-digits
/// apart from the separators. `None` for anything else — `Date` and `Time` are
/// free-form `TEXT` in the database and a real station's history holds values
/// that name no point in time, so "unparseable" has to be representable rather
/// than guessed at.
#[must_use]
pub fn parse_civil(date: &str, time: &str) -> Option<CivilTime> {
    let d: Vec<&str> = date.split('-').collect();
    let t: Vec<&str> = time.split(':').collect();
    if d.len() != 3 || t.len() != 3 {
        return None;
    }
    if d[0].len() != 4 || d[1].len() != 2 || d[2].len() != 2 {
        return None;
    }
    if t[0].len() != 2 || t[1].len() != 2 || t[2].len() != 2 {
        return None;
    }
    let year: u32 = d[0].parse().ok()?;
    let month: u32 = d[1].parse().ok()?;
    let day: u32 = d[2].parse().ok()?;
    let hour: u32 = t[0].parse().ok()?;
    let minute: u32 = t[1].parse().ok()?;
    let second: u32 = t[2].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some(CivilTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    })
}

/// Add `offset_secs` to a `YYYY-MM-DD` / `HH:MM:SS` pair, returning the same
/// pair of strings shifted — rolling the date when the offset crosses midnight.
///
/// This is what turns a recording's start time into the time a *chunk within
/// it* was actually heard. BirdNET-Pi does the same thing in its `Detection`
/// constructor (`file_date + timedelta(seconds=self.start)`); doing it here
/// keeps a detection's timestamp meaning the same thing in both projects, and
/// keeps natively-recorded rows consistent with the BirdNET-Pi rows the
/// migration importer writes.
///
/// `None` when the input does not parse, so a caller can keep the original
/// rather than substitute an invented time.
#[must_use]
#[allow(clippy::cast_possible_truncation)]
pub fn shift_datetime(date: &str, time: &str, offset_secs: f64) -> Option<(String, String)> {
    if !offset_secs.is_finite() {
        return None;
    }
    let civil = parse_civil(date, time)?;
    let shifted = civil_from_unix_secs(unix_secs_from_civil(&civil) + offset_secs.trunc() as i64);
    Some((
        format!(
            "{:04}-{:02}-{:02}",
            shifted.year, shifted.month, shifted.day
        ),
        format!(
            "{:02}:{:02}:{:02}",
            shifted.hour, shifted.minute, shifted.second
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> CivilTime {
        civil_from_unix_secs(secs)
    }

    /// The two directions must agree, including across leap days.
    #[test]
    fn unix_secs_from_civil_round_trips() {
        for secs in [
            0_i64,
            1_771_000_000, // 2026-02-13
            1_772_323_200, // 2026-03-01, the day after February
            1_709_164_800, // 2024-02-29, a leap day
            1_735_689_599, // 2024-12-31 23:59:59
            4_102_444_800, // 2100-01-01, a non-leap century
        ] {
            let civil = civil_from_unix_secs(secs);
            assert_eq!(
                unix_secs_from_civil(&civil),
                secs,
                "round trip failed for {secs} ({civil:?})"
            );
        }
    }

    #[test]
    fn parse_civil_rejects_what_is_not_a_date() {
        assert!(parse_civil("2026-03-11", "08:30:00").is_some());
        // The shapes a station's history actually holds.
        assert!(parse_civil("", "").is_none());
        assert!(parse_civil("not-a-date", "25:99:99").is_none());
        assert!(
            parse_civil("2026-3-11", "08:30:00").is_none(),
            "month must be 2 digits"
        );
        assert!(
            parse_civil("2026-03-11", "8:30:00").is_none(),
            "hour must be 2 digits"
        );
        assert!(parse_civil("2026-13-11", "08:30:00").is_none(), "month 13");
        assert!(parse_civil("2026-03-11", "24:00:00").is_none(), "hour 24");
        assert!(parse_civil("2026-03-11", "08:60:00").is_none(), "minute 60");
    }

    /// Shifting a recording's start time by a chunk offset.
    ///
    /// The rollover case is the reason this is date arithmetic and not string
    /// manipulation: a segment that starts at 23:59:55 has chunks belonging to
    /// the *next day*, and a naive `HH:MM:SS + n` would write 24:00:04 — a time
    /// no engine will parse, on the wrong date.
    #[test]
    fn shift_datetime_adds_the_chunk_offset() {
        // The five chunks of one 15-second segment.
        let chunks: Vec<(String, String)> = [0.0, 3.0, 6.0, 9.0, 12.0]
            .iter()
            .map(|&o| shift_datetime("2026-03-11", "08:30:00", o).unwrap())
            .collect();
        assert_eq!(
            chunks,
            vec![
                ("2026-03-11".into(), "08:30:00".into()),
                ("2026-03-11".into(), "08:30:03".into()),
                ("2026-03-11".into(), "08:30:06".into()),
                ("2026-03-11".into(), "08:30:09".into()),
                ("2026-03-11".into(), "08:30:12".into()),
            ]
        );
    }

    #[test]
    fn shift_datetime_rolls_over_midnight() {
        assert_eq!(
            shift_datetime("2026-03-11", "23:59:55", 9.0).unwrap(),
            ("2026-03-12".into(), "00:00:04".into())
        );
        // And across a month, a year, and a leap day.
        assert_eq!(
            shift_datetime("2026-12-31", "23:59:58", 3.0).unwrap(),
            ("2027-01-01".into(), "00:00:01".into())
        );
        assert_eq!(
            shift_datetime("2024-02-28", "23:59:58", 3.0).unwrap(),
            ("2024-02-29".into(), "00:00:01".into())
        );
    }

    #[test]
    fn shift_datetime_keeps_unparseable_input_unparseable() {
        // Never invent a timestamp for a row that names no point in time.
        assert!(shift_datetime("", "", 3.0).is_none());
        assert!(shift_datetime("not-a-date", "25:99:99", 3.0).is_none());
        assert!(shift_datetime("2026-03-11", "08:30:00", f64::NAN).is_none());
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
