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

/// Days since 1970-01-01 for a proleptic-Gregorian `(year, month, day)`.
///
/// Hinnant's `days_from_civil`. Exposed because nine files carried their own
/// copy of this arithmetic — `146_097` appeared in ten places across
/// `birdnet-web`, `birdnet-timeseries`' tests and the binary. Checked verbatim
/// against each other over 200 years in both directions, all thirteen agreed,
/// so this consolidation fixes no live defect. It removes the *next* one: a
/// correct algorithm copied nine times is nine chances for a transcription slip
/// nobody notices, in code that decides which day a detection belongs to.
///
/// Pre-year-0 dates return `0` for the same reason [`unix_secs_from_civil`]
/// clamps: truncating division makes `era` wrong there, and nothing in this
/// project deals in dates that far back.
#[must_use]
pub const fn days_from_civil(year: u32, month: u32, day: u32) -> i64 {
    let y = year as i64 - (month <= 2) as i64;
    if y < 0 {
        return 0;
    }
    let era = y / 400;
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The `(year, month, day)` for a count of days since 1970-01-01 — the inverse
/// of [`days_from_civil`].
///
/// Hinnant's `civil_from_days`. Days before the epoch are clamped to
/// 1970-01-01 for the same reason [`civil_from_unix_secs`] clamps: a negative
/// value here only ever means a broken clock, and clamping keeps every
/// formatter total instead of making each caller handle it.
#[must_use]
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub const fn civil_from_days(days: i64) -> (u32, u32, u32) {
    // `is_negative()` rather than `days < 0`, and the difference is not style.
    // `if days < 0 { 0 } else { days }` has a mutant — `days <= 0` — that no
    // test can ever kill, because clamping zero to zero is what the unmutated
    // code already does. An unkillable mutant against a `max_missed: 0` gate is
    // a permanent red with no fix available, so the comparison is replaced by
    // the predicate it was spelling out. It also says the intent directly.
    let days = if days.is_negative() { 0 } else { days };
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y + 1 } else { y };
    (year as u32, month, day)
}

/// Format a count of days since 1970-01-01 as `YYYY-MM-DD`.
///
/// The shape every date column in this project stores, and the reason several
/// of the copies this replaces existed at all.
#[must_use]
pub fn date_string_from_days(days: i64) -> String {
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
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
/// Implements Hinnant's `days_from_civil`. Unlike the forward direction, this
/// does **not** clamp at 1970: a pre-epoch civil date returns the correct
/// negative value (`1969-01-01` → `-31_536_000`). Round-tripping through
/// [`civil_from_unix_secs`] still lands on the epoch for anything pre-1970,
/// because *that* direction clamps — nothing here deals in pre-1970 audio, and
/// a date that far out means a broken clock either way.
///
/// The `y < 0` guard below is arithmetic, not policy. `y` is the year shifted
/// back one for January and February, so the only way to reach a negative is
/// year 0 in those two months. Rust's integer division truncates toward zero,
/// so `era = y / 400` would yield `0` where Hinnant's algorithm needs `-1`, and
/// the result would be quietly wrong rather than obviously so. `y == 0` — year
/// 0 from March, or year 1 in January — is a value the algorithm handles
/// correctly and must not be caught by it.
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

/// The instant a **local** wall-clock `date`/`time` names, given the UTC offset
/// in force at that moment, in seconds since the Unix epoch.
///
/// # Why the offset is a parameter and not looked up
///
/// A station's `Date`/`Time` pair is local wall clock with no offset recorded,
/// which is not a point in time: one local hour repeats every autumn and one
/// never happens every spring. Converting it needs an offset, and *which*
/// offset depends on what the caller knows:
///
/// * A detection being written **now** knows the offset in force, because the
///   capture supervisor is maintaining it. `local − offset` is then exact, in
///   both passes of the repeated hour: the first is written while the offset is
///   still +2 and the second while it is +1, so the two land an hour apart —
///   which is what actually happened.
/// * **History** nobody was there for has no such knowledge, and is better
///   converted through a tz database for its own date, which is what migration
///   32's backfill does in SQL.
///
/// Taking the offset as an argument keeps this pure — no clock, no database,
/// testable at any instant — and keeps the choice of offset where the knowledge
/// is.
///
/// Returns `None` when `date`/`time` do not name a civil time at all.
#[must_use]
pub fn unix_secs_from_local(date: &str, time: &str, offset_secs: i64) -> Option<i64> {
    parse_civil(date, time).map(|c| unix_secs_from_civil(&c) - offset_secs)
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
    /// The day-level primitives must agree with the second-level ones, and with
    /// each other, across every day this project can see.
    ///
    /// Nine files carried their own copy of this arithmetic before it was
    /// exposed here; they were checked verbatim against each other over 200
    /// years and all agreed, so nothing was broken. This gate is what stops the
    /// tenth copy — or a transcription slip in the consolidation itself — being
    /// the one that differs. A wrong day here moves a detection to the wrong
    /// date, which is the kind of error that is invisible on a dashboard and
    /// fatal in a dataset.
    #[test]
    fn the_day_and_second_primitives_agree_over_two_centuries() {
        // 1970-01-01 .. 2170-01-01
        for days in 0..73_050i64 {
            let (y, m, d) = civil_from_days(days);
            let via_secs = civil_from_unix_secs(days * 86_400);
            assert_eq!(
                (y, m, d),
                (via_secs.year, via_secs.month, via_secs.day),
                "day {days}"
            );
            assert_eq!(days_from_civil(y, m, d), days, "round trip at day {days}");
            assert_eq!(
                date_string_from_days(days),
                format!("{y:04}-{m:02}-{d:02}"),
                "day {days}"
            );
        }
    }

    /// The clamps, which are the only place the two directions deliberately
    /// disagree with pure Hinnant.
    #[test]
    fn pre_epoch_days_clamp_rather_than_wrap() {
        assert_eq!(civil_from_days(-1), (1970, 1, 1));
        assert_eq!(civil_from_days(i64::MIN / 2), (1970, 1, 1));
        // Forward: year 0 in Jan/Feb is the only way to reach a negative `y`.
        assert_eq!(days_from_civil(0, 1, 1), 0);
        // …and a real date is untouched by that guard.
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
    }

    /// `y == 0` is on the *computing* side of the guard, not the clamping side.
    ///
    /// Found by mutation testing: `if y < 0` survived being changed to
    /// `if y <= 0`, because every other gate in this file works in 1970–2170
    /// and never produces a shifted year of zero. The test above does not catch
    /// it either — it asserts the clamp returns `0`, and `0` is also the honest
    /// answer for 1970-01-01, so "clamped" and "computed" are indistinguishable
    /// there.
    ///
    /// Both ways of reaching `y == 0` are pinned, because they arrive by
    /// different routes: year 0 from March (no shift) and year 1 in Jan/Feb
    /// (shifted down by one). Neither may clamp.
    #[test]
    fn a_shifted_year_of_zero_still_computes_a_real_day_count() {
        assert_eq!(
            days_from_civil(0, 3, 1),
            -719_468,
            "year 0 March 1 is on the computing side of the `y < 0` guard"
        );
        assert_eq!(
            days_from_civil(1, 1, 1),
            -719_162,
            "year 1 January 1 shifts to y == 0, which is still not negative"
        );
        // The counterpart, so this is a boundary and not a blanket alarm: one
        // day earlier crosses into the shifted-negative region and clamps.
        assert_eq!(
            days_from_civil(0, 2, 29),
            0,
            "year 0 February shifts to y == -1 and must clamp"
        );
    }

    /// Leap-day handling, spelled out because it is the case every copy of this
    /// algorithm gets right or wrong together.
    #[test]
    fn leap_days_land_where_they_should() {
        for (y, m, d) in [
            (2024, 2, 29), // leap year
            (2000, 2, 29), // divisible by 400
            (2100, 3, 1),  // 2100 is NOT a leap year
            (2026, 12, 31),
        ] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m, d), "{y}-{m}-{d}");
        }
        // 2100 has no 29 February: 28 Feb and 1 Mar are consecutive.
        assert_eq!(
            days_from_civil(2100, 3, 1) - days_from_civil(2100, 2, 28),
            1
        );
        // 2024 does: they are two days apart.
        assert_eq!(
            days_from_civil(2024, 3, 1) - days_from_civil(2024, 2, 28),
            2
        );
    }

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

    /// The boundary of the `y < 0` guard, from both sides.
    ///
    /// `y` is the year shifted back one for January and February, so year 0 in
    /// those months is the only way to reach a negative — and it is the one
    /// input the era arithmetic cannot take, because Rust truncates `-1 / 400`
    /// toward zero where Hinnant's algorithm needs `-1`. Year 0 from March is
    /// the neighbour that must *not* be caught: `y == 0` computes correctly.
    ///
    /// Widening the guard to `<=` or narrowing it to `==` would swallow that
    /// neighbour and silently return the epoch for a date the algorithm
    /// handles. Both are mutations `cargo-mutants` generates against this line,
    /// and neither was caught until this test existed.
    #[test]
    fn only_a_negative_shifted_year_saturates() {
        let at_year_zero = |month| CivilTime {
            year: 0,
            month,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };

        // y == -1: the guard fires.
        assert_eq!(unix_secs_from_civil(&at_year_zero(1)), 0, "0000-01-01");
        assert_eq!(unix_secs_from_civil(&at_year_zero(2)), 0, "0000-02-01");

        // y == 0: it must not. Computed, not saturated.
        assert_eq!(
            unix_secs_from_civil(&at_year_zero(3)),
            -62_162_035_200,
            "0000-03-01 is a date the era arithmetic handles"
        );
    }

    /// A pre-epoch civil date converts to negative seconds, not to zero.
    ///
    /// Deliberately asymmetric with `pre_epoch_saturates_to_the_epoch` above,
    /// which pins the *forward* direction's clamp. Only that direction clamps.
    /// The doc comment on `unix_secs_from_civil` claimed both did, until the
    /// mutation gate sent someone to read the code.
    #[test]
    fn pre_epoch_civil_dates_return_negative_seconds() {
        let new_year_1969 = CivilTime {
            year: 1969,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 0,
        };
        assert_eq!(unix_secs_from_civil(&new_year_1969), -31_536_000);

        // …but a round trip lands on the epoch, because the forward direction
        // clamps. This is what any caller composing the two actually observes.
        assert_eq!(
            civil_from_unix_secs(unix_secs_from_civil(&new_year_1969)),
            civil_from_unix_secs(0)
        );
    }

    /// Every field-shape check, exercised one field at a time.
    ///
    /// `parse_civil` is three chained `||` checks over six fields, and a test
    /// that breaks two fields at once cannot tell those chains apart from a
    /// single `&&`. That is not a theoretical gap: the suite had cases for a
    /// malformed date *and* a malformed time together, and for a one-digit
    /// hour, and `cargo-mutants` still walked several of these operators
    /// unnoticed. Each case below leaves every other field valid, so exactly
    /// one sub-condition is doing the rejecting.
    #[test]
    fn every_field_shape_is_checked_on_its_own() {
        // Baseline: the case all the others are one mutation away from.
        assert!(parse_civil("2026-03-11", "08:30:00").is_some());

        // Split counts, one side at a time.
        assert!(parse_civil("2026-03", "08:30:00").is_none(), "date needs 3");
        assert!(parse_civil("2026-03-11", "08:30").is_none(), "time needs 3");

        // Date field widths, one field at a time.
        assert!(parse_civil("202-03-11", "08:30:00").is_none(), "year 4");
        assert!(parse_civil("2026-3-11", "08:30:00").is_none(), "month 2");
        assert!(parse_civil("2026-03-1", "08:30:00").is_none(), "day 2");

        // Time field widths, one field at a time.
        assert!(parse_civil("2026-03-11", "8:30:00").is_none(), "hour 2");
        assert!(parse_civil("2026-03-11", "08:3:00").is_none(), "minute 2");
        assert!(parse_civil("2026-03-11", "08:30:0").is_none(), "second 2");
    }

    /// The last valid value of each time field, and the first invalid one.
    ///
    /// `hour > 23`, `minute > 59` and `second > 59` are the three comparisons
    /// most easily written one off. Asserting only that `60` is rejected
    /// cannot distinguish `> 59` from `>= 59` or `== 59` — every one of those
    /// rejects 60. It takes the *accepted* boundary to pin the operator, which
    /// is why `second > 59` survived mutation until this test existed.
    #[test]
    fn time_field_bounds_accept_their_last_valid_value() {
        for (time, why) in [
            ("23:59:59", "23:59:59 is a real time"),
            ("00:00:00", "midnight is a real time"),
        ] {
            assert!(
                parse_civil("2026-03-11", time).is_some(),
                "{why} — rejecting it means a bound is off by one"
            );
        }

        // …and one past each, one field at a time.
        assert!(parse_civil("2026-03-11", "24:30:00").is_none(), "hour 24");
        assert!(parse_civil("2026-03-11", "08:60:00").is_none(), "minute 60");
        assert!(parse_civil("2026-03-11", "08:30:60").is_none(), "second 60");
    }

    /// The calendar-range check, from both sides of each bound.
    ///
    /// Same reasoning as the time bounds: `1..=12` and `1..=31` are only
    /// pinned by asserting that 1, 12 and 31 are *accepted*.
    #[test]
    fn month_and_day_bounds_accept_their_edges() {
        assert!(parse_civil("2026-01-01", "08:30:00").is_some(), "month 1");
        assert!(parse_civil("2026-12-31", "08:30:00").is_some(), "month 12");
        assert!(parse_civil("2026-00-11", "08:30:00").is_none(), "month 0");
        assert!(parse_civil("2026-13-11", "08:30:00").is_none(), "month 13");
        assert!(parse_civil("2026-03-00", "08:30:00").is_none(), "day 0");
        assert!(parse_civil("2026-03-32", "08:30:00").is_none(), "day 32");
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
    /// The property the whole `detected_at_utc` column rests on: two passes of
    /// the local hour daylight saving repeats have the *same* wall clock and
    /// must produce *different* instants, because the offset in force differs.
    ///
    /// Europe/Berlin, 2026-10-25: at 01:00 UTC the offset moves +2 -> +1, so
    /// local 02:30 happens once at 00:30Z and again at 01:30Z.
    #[test]
    fn the_repeated_hour_is_two_instants_when_the_offset_is_known() {
        let first =
            unix_secs_from_local("2026-10-25", "02:30:00", 2 * 3600).expect("valid civil time");
        let second =
            unix_secs_from_local("2026-10-25", "02:30:00", 3600).expect("valid civil time");
        assert_eq!(
            second - first,
            3600,
            "the two passes are an hour apart in real time"
        );
        // And they are the two real instants, not merely distinct.
        assert_eq!(first, 1_792_888_200, "00:30Z, the CEST reading");
        assert_eq!(second, 1_792_891_800, "01:30Z, the CET reading");
    }

    #[test]
    fn a_utc_station_gets_its_wall_clock_back_unchanged() {
        let t = unix_secs_from_local("2026-05-01", "06:00:00", 0).expect("valid");
        assert_eq!(
            civil_from_unix_secs(t).hour,
            6,
            "on UTC the instant and the wall clock agree"
        );
    }

    #[test]
    fn a_western_offset_moves_the_instant_later_not_earlier() {
        // UTC-5: local 06:00 is 11:00Z, so the instant is *larger*.
        let utc = unix_secs_from_local("2026-05-01", "06:00:00", 0).expect("valid");
        let est = unix_secs_from_local("2026-05-01", "06:00:00", -5 * 3600).expect("valid");
        assert_eq!(
            est - utc,
            5 * 3600,
            "sign of the offset must not be flipped"
        );
    }

    #[test]
    fn a_time_that_names_nothing_has_no_instant() {
        assert!(unix_secs_from_local("", "", 0).is_none());
        assert!(unix_secs_from_local("not-a-date", "25:99:99", 0).is_none());
    }
}
