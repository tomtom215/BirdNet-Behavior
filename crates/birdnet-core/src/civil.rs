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

/// Whether `year` is a Gregorian leap year.
///
/// Exposed because the predicate was written out by hand in three more places
/// and `days_in_month` needs it here. Two of those copies stay, and both for a
/// reason worth recording:
///
/// * `birdnet-scheduler` depends on `serde` and nothing else — deliberately, so
///   the solar arithmetic stays a pure-computation crate. Taking
///   `birdnet-core` for two `const fn`s would pull ONNX Runtime, `symphonia`
///   and `rubato` into it. `tests/leap_year_agrees_with_the_scheduler.rs`
///   checks the two against each other instead, so they cannot drift.
/// * `src/capture/schedule.rs`'s copy is the *oracle* its own conversion is
///   checked against; its comment says so. An oracle that calls the
///   implementation under test proves nothing.
#[must_use]
pub const fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Days in `month` of `year`, `1..=12`.
///
/// An out-of-range month returns `30`, matching the caller this replaced: these
/// are calendar-rendering helpers, and a total function keeps a bad month a
/// short month rather than a panic in a page render.
#[must_use]
pub const fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 30,
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

/// The BirdNET "week of year" for a calendar month and day, always in `1..=48`.
///
/// BirdNET's metadata model — the geomodel this project feeds through
/// [`crate::inference::species_filter::SpeciesFilter`] — takes
/// `(latitude, longitude, week)` and was trained on a **48-week year**: four
/// weeks per month, with days 29–31 clamped into week 4 of their month. It is
/// not the ISO week and not the day-of-year ÷ 7; asking it about week 0 or
/// week 52 is asking about a point outside its input domain.
///
/// Both reference implementations agree on this arithmetic and on the clamp:
/// `tphakala/birdnet-go`'s `internal/inference/onnx/rangefilter.go`
/// (`CalculateWeek`, documented "Result is always in `[1, 48]`") records that
/// an un-clamped copy which produced week 49 for 29–31 December was a real
/// defect fed into a live range filter.
///
/// # Panics
///
/// Never. `month` outside `1..=12` and `day` outside `1..=31` are clamped
/// rather than rejected: this is the last step before a model input, and a
/// plausible week is a better failure than a panic in the capture path. Callers
/// that need to know a date was malformed should parse it with
/// [`parse_civil`] first, which rejects rather than guesses.
/// # Why the clamps are written this way
///
/// This was `if month < 1 { 1 } else if month > 12 { 12 }`, plus
/// `if w > 4 { 4 }`. Behaviourally identical to what is below — and all three
/// comparisons were **unkillable by any test**, because each clamped to the
/// value it compared against: `month < 1 => 1` and `month <= 1 => 1` are the
/// same function, as are `month > 12 => 12` and `month >= 12 => 12`, and
/// `w > 4 => 4` and `w >= 4 => 4`. No input distinguishes either pair, so
/// `cargo-mutants` reported three survivors on a file whose gate allows none,
/// and the repo's policy for that is to refactor until the boundary is
/// observable rather than to lift the threshold.
///
/// So: the lower clamps became `saturating_sub`, which has no comparison to
/// mutate, and each remaining boundary is now one past the value it produces
/// (`>= 13` yielding 11, `>= 5` yielding 4). Flipping either by one is then a
/// visible difference — `month >= 13` weakened to `> 13` returns week 49 for
/// 1 January of month 13, outside the model's domain, which is the defect the
/// clamp exists to prevent.
#[must_use]
pub const fn birdnet_week(month: u32, day: u32) -> u32 {
    // 0-based month. `saturating_sub` covers the `month == 0` case without a
    // comparison: 0 and 1 both give index 0, which is what clamping to 1 did.
    let month_index = if month >= 13 {
        11
    } else {
        month.saturating_sub(1)
    };
    let week_in_month = {
        // Days 29-31 (and anything larger) land in week 4 of their month.
        let w = day.saturating_sub(1) / 7 + 1;
        if w >= 5 { 4 } else { w }
    };
    month_index * 4 + week_in_month
}

/// The BirdNET week for a `YYYY-MM-DD` date string, or `None` if it does not parse.
///
/// The date a station files a detection under is the date in its *recording's*
/// filename, not "now": a backlog drained three days after a power cut must be
/// scored against the season it was recorded in, not the season it was
/// analysed in. See [`birdnet_week`] for the arithmetic and why the domain
/// matters.
#[must_use]
pub fn birdnet_week_from_date(date: &str) -> Option<u32> {
    // `parse_civil` wants a time as well and validates both halves; midnight
    // is a legitimate time of day, so it is the right filler for a date-only
    // input and cannot make a bad date parse.
    let civil = parse_civil(date, "00:00:00")?;
    Some(birdnet_week(civil.month, civil.day))
}

/// Unix-time floor below which the system clock is not trusted for anything
/// that depends on knowing the date.
///
/// A Raspberry Pi has no battery-backed RTC, so before NTP lands it commonly
/// reports the epoch or a stale build-time value. `2024-01-01T00:00:00Z` is
/// safely before this project's deployment era and far above any unset-clock
/// reading, so a value below it means "time is not trustworthy yet".
///
/// # Why this lives here
///
/// There used to be two of these, **1 461 days apart**: the capture
/// supervisor's, at this value, and `--doctor`'s, at `2020-01-01`, under a
/// comment claiming it *"mirrors the capture supervisor's"*. It did not. For
/// any reading between them the doctor printed
/// `[ PASS ] System clock — set to a plausible current time` while the
/// supervisor treated the same reading as untrustworthy and disabled the
/// recording schedule and every quiet window. An operator reading the
/// diagnostic was told the opposite of what the station was doing.
///
/// One constant, in the module that already owns the calendar arithmetic both
/// of them use.
pub const CLOCK_PLAUSIBLE_FLOOR_SECS: u64 = 1_704_067_200;

/// Whether a Unix timestamp is late enough to be a real current time.
///
/// Pure, so the boundary is testable without a clock. A `false` result means
/// the caller should not make a dated decision: the capture supervisor fails
/// *open* on it (keep recording rather than trust a bogus date for solar
/// scheduling), and every destructive retention job refuses to run.
///
/// This is a **floor, not a range**. A clock reading far in the *future* is
/// also wrong and is not caught here, because catching it needs a reference
/// this function does not have — see `docs/UNATTENDED_DEPLOYMENT_AUDIT.md`
/// (NT-4). What the floor does cover is the common case on this hardware: an
/// RTC-less board that boots at the epoch and stays there until the network
/// comes back, which on a field station may be never.
#[must_use]
pub const fn clock_looks_plausible(secs: u64) -> bool {
    secs >= CLOCK_PLAUSIBLE_FLOOR_SECS
}

/// Whether a `YYYY-MM-DD` date could be a real date this station recorded on.
///
/// Applies [`clock_looks_plausible`] to the date's own midnight, so it answers
/// the same question about a recording's filename that the capture supervisor
/// asks about the system clock.
///
/// A date that does not parse is **not** plausible. On the live capture path
/// this cannot happen — `detection::pipeline::process_file` refuses a file
/// whose name does not parse — so the `None` branch is the belt to that
/// braces, and it fails closed because a detection that names no day cannot be
/// filed under one.
///
/// # Why this exists
///
/// A Raspberry Pi has no battery-backed RTC. Before NTP lands it reads the
/// epoch, the capture tee stamps that into the segment filename, and the
/// detection's `Date` and `Time` are parsed straight back out of that filename.
/// Nothing checked. Every detection produced before the clock was set was
/// stored as `1970-01-01`, where it stays: `species_summary` files it under
/// hour 00 for ever, `MIN(Date)` makes every species touched in that window
/// "first seen 1970", the history calendar acquires a 56-year span, and
/// `detected_at_utc` of about zero sorts it before everything. Retention then
/// reclaims the audio, because it is older than any cutoff — so the evidence
/// goes and the poisoned rows stay.
#[must_use]
pub fn date_looks_plausible(date: &str) -> bool {
    let Some(civil) = parse_civil(date, "00:00:00") else {
        return false;
    };
    u64::try_from(unix_secs_from_civil(&civil)).is_ok_and(clock_looks_plausible)
}

#[cfg(test)]
mod tests {
    use super::{days_in_month, is_leap_year};

    /// All three Gregorian rules, in this crate's own test suite.
    ///
    /// `tests/leap_year_agrees_with_the_scheduler.rs` already drives this
    /// predicate against `birdnet-scheduler`'s private copy over six centuries
    /// — but that test lives in the workspace-root binary crate, because it is
    /// the only one depending on both. `cargo mutants --package birdnet-core`
    /// does not run it, so from the mutation gate's point of view these two
    /// functions arrived with no coverage at all: **11 mutants, 11 missed**.
    ///
    /// CI named only four of them, because that shard tested four — the gate
    /// splits this file three ways. Fixing the four it printed would have left
    /// the other shards red, which is why the number to work from is the one a
    /// local `cargo mutants --package birdnet-core --in-diff` reports, not the
    /// one in the failing job:
    ///
    /// ```text
    /// replace is_leap_year -> bool with true / with false
    /// replace || with && / && with || in is_leap_year
    /// delete ! in is_leap_year
    /// replace days_in_month -> u32 with 0 / with 1
    /// delete match arm 1 | 3 | 5 | 7 | 8 | 10 | 12
    /// delete match arm 2
    /// replace match guard is_leap_year(year) with true / with false
    /// ```
    ///
    /// Coverage a gate cannot see is not coverage. Every case below is a year
    /// or month where some mutant disagrees with the real function — 2000 kills
    /// `|| → &&`, 2023 kills `&& → ||`, 1900 kills the deleted `!`, and
    /// February in both a leap and a common year kills both guard mutants and
    /// the deleted arm. None of them is a convenient round number.
    #[test]
    fn every_gregorian_leap_rule_is_pinned() {
        // Divisible by 4 → leap. Deleting the `!` makes this false, because
        // 2024 is *not* divisible by 100.
        assert!(is_leap_year(2024), "2024 is a leap year");
        assert!(is_leap_year(1996), "1996 is a leap year");

        // Divisible by 100 → not leap. Deleting the `!` makes this true, which
        // is the century mistake the rule exists to prevent.
        assert!(!is_leap_year(1900), "1900 is not a leap year");
        assert!(!is_leap_year(2100), "2100 is not a leap year");

        // Divisible by 400 → leap after all.
        assert!(is_leap_year(2000), "2000 is a leap year");
        assert!(is_leap_year(2400), "2400 is a leap year");

        // Not divisible by 4 → not leap.
        assert!(!is_leap_year(2023), "2023 is not a leap year");
        assert!(!is_leap_year(2026), "2026 is not a leap year");
    }

    /// Every month length, including the arm a mutant can delete.
    ///
    /// The 31-day arm is the interesting one: deleting it drops those months
    /// through to the `_ => 30` catch-all, and a calendar renderer that thinks
    /// January has 30 days silently loses a cell. Asserting the 30-day months
    /// alone would not notice.
    #[test]
    fn every_month_length_is_pinned() {
        for month in [1, 3, 5, 7, 8, 10, 12] {
            assert_eq!(days_in_month(2023, month), 31, "month {month} has 31 days");
        }
        for month in [4, 6, 9, 11] {
            assert_eq!(days_in_month(2023, month), 30, "month {month} has 30 days");
        }

        // February follows the predicate above, both ways.
        assert_eq!(days_in_month(2024, 2), 29, "February 2024");
        assert_eq!(days_in_month(2023, 2), 28, "February 2023");
        assert_eq!(
            days_in_month(1900, 2),
            28,
            "February 1900 — the century rule"
        );
        assert_eq!(days_in_month(2000, 2), 29, "February 2000 — the /400 rule");

        // The documented total-function behaviour: an impossible month is a
        // short month, not a panic in a page render.
        assert_eq!(days_in_month(2023, 0), 30, "month 0 falls through");
        assert_eq!(days_in_month(2023, 13), 30, "month 13 falls through");
    }

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

#[cfg(test)]
mod birdnet_week_tests {
    use super::{birdnet_week, birdnet_week_from_date};

    /// The domain contract, swept over every date a Gregorian year can hold.
    ///
    /// The geomodel was trained on `1..=48`. Anything outside it is a question
    /// the model was never asked, and the answer it invents is not detectable
    /// downstream — which is exactly how a hardcoded `0` survived in the
    /// daemon's two call sites without a single failing test.
    #[test]
    fn every_day_of_every_month_lands_in_the_trained_domain() {
        for month in 1..=12 {
            for day in 1..=31 {
                let w = birdnet_week(month, day);
                assert!(
                    (1..=48).contains(&w),
                    "month {month} day {day} gave week {w}, outside the model's 1..=48 domain"
                );
            }
        }
    }

    /// The four boundaries of the arithmetic, named individually so a mutant
    /// that breaks one is not masked by the other three.
    #[test]
    fn the_corners_of_the_forty_eight_week_year() {
        assert_eq!(birdnet_week(1, 1), 1, "the first day of the year is week 1");
        assert_eq!(birdnet_week(1, 7), 1, "day 7 is still the first week");
        assert_eq!(birdnet_week(1, 8), 2, "day 8 opens the second week");
        assert_eq!(
            birdnet_week(12, 31),
            48,
            "the last day of the year is week 48"
        );
    }

    /// Days 29-31 clamp into week 4 rather than opening a week 5.
    ///
    /// This is the case `birdnet-go` records as a live defect in its own
    /// history: an un-clamped copy returned 49 for 29-31 December and fed it
    /// to the range filter.
    #[test]
    fn the_long_tail_of_a_month_clamps_into_week_four() {
        for day in 29..=31 {
            assert_eq!(
                birdnet_week(12, day),
                48,
                "December {day} must clamp to week 48, not open a 49th"
            );
            assert_eq!(
                birdnet_week(1, day),
                4,
                "January {day} must clamp to week 4"
            );
        }
        // And the day before the clamp is genuinely week 4 too, so the clamp is
        // not doing the work of the formula.
        assert_eq!(birdnet_week(1, 22), 4);
    }

    /// Each month opens a new block of four weeks. Without this a mutant that
    /// drops the `(month - 1) * 4` term still passes every within-month case.
    #[test]
    fn each_month_opens_the_next_block_of_four() {
        for month in 1..=12 {
            assert_eq!(
                birdnet_week(month, 1),
                (month - 1) * 4 + 1,
                "the first day of month {month}"
            );
        }
        // Adjacent months must differ, which a constant-returning mutant cannot do.
        assert_ne!(birdnet_week(1, 15), birdnet_week(7, 15));
    }

    /// A malformed month or day is clamped, not panicked on: this runs in the
    /// capture path.
    #[test]
    fn out_of_range_inputs_are_clamped_into_the_domain() {
        assert_eq!(birdnet_week(0, 1), 1);
        assert_eq!(birdnet_week(13, 31), 48);
        assert_eq!(birdnet_week(1, 0), 1);
        assert_eq!(birdnet_week(6, 99), 24);
    }

    #[test]
    fn each_clamp_boundary_is_observable() {
        // These two comparisons replaced three that no test could ever have
        // pinned: the old code clamped to the value it compared against, so
        // `month > 12 => 12` and `month >= 12 => 12` were the same function.
        // A mutation gate reported them as survivors on a file that allows
        // none. The boundaries below are deliberately one past their result,
        // which is what makes a one-step flip visible — so this test exists to
        // keep that property, not merely to check two numbers.

        // `month >= 13 => index 11`. Weakened to `> 13`, month 13 would index
        // 12 and produce week 49 or more — outside the model's 1..=48 domain,
        // which is the defect the clamp is for.
        assert_eq!(birdnet_week(12, 1), 45, "month 12 is not clamped");
        assert_eq!(birdnet_week(13, 1), 45, "month 13 clamps onto month 12");
        assert_eq!(birdnet_week(13, 31), 48, "and cannot exceed 48");

        // `w >= 5 => 4`. Weakened to `> 5`, day 29 would give week 5 of its
        // month and push the year to 49 weeks.
        assert_eq!(birdnet_week(1, 28), 4, "day 28 reaches week 4 on its own");
        assert_eq!(birdnet_week(1, 29), 4, "and day 29 is clamped into it");
        assert_eq!(
            birdnet_week(12, 29),
            48,
            "the case birdnet-go recorded as a real defect: 29 December must \
             not be week 49"
        );

        // The lower ends have no comparison left to flip — `saturating_sub`
        // handles them — but they must still answer as they did before.
        assert_eq!(birdnet_week(0, 0), 1);
        assert_eq!(birdnet_week(0, 31), 4, "a zero month keeps the day's week");
    }

    #[test]
    fn a_date_string_resolves_to_its_week() {
        assert_eq!(birdnet_week_from_date("2026-01-05"), Some(1));
        assert_eq!(birdnet_week_from_date("2026-07-05"), Some(25));
        assert_eq!(birdnet_week_from_date("2026-12-31"), Some(48));
        // A leap day is an ordinary day to this arithmetic.
        assert_eq!(birdnet_week_from_date("2028-02-29"), Some(8));
    }

    /// Unparseable dates are `None`, never a guessed week. The database's
    /// `Date` column is free-form `TEXT` and an imported history holds values
    /// that name no point in time.
    #[test]
    fn an_unparseable_date_has_no_week() {
        for bad in [
            "",
            "not-a-date",
            "2026-13-01",
            "2026-01-32",
            "26-01-01",
            "2026-1-1",
        ] {
            assert_eq!(birdnet_week_from_date(bad), None, "{bad} must not parse");
        }
    }
}

#[cfg(test)]
mod clock_floor_tests {
    use super::{CLOCK_PLAUSIBLE_FLOOR_SECS, civil_from_unix_secs, clock_looks_plausible};

    /// The constant must be the date its documentation claims. A floor whose
    /// comment and value disagree is how the two divergent copies survived.
    #[test]
    fn the_floor_is_the_date_it_says_it_is() {
        let t = civil_from_unix_secs(i64::try_from(CLOCK_PLAUSIBLE_FLOOR_SECS).expect("fits"));
        assert_eq!((t.year, t.month, t.day), (2024, 1, 1));
        assert_eq!((t.hour, t.minute, t.second), (0, 0, 0));
    }

    /// Both sides of the boundary, because a gate that only asserts the
    /// rejecting side passes just as well against a predicate that rejects
    /// everything.
    #[test]
    fn the_boundary_is_inclusive_and_discriminating() {
        assert!(
            !clock_looks_plausible(0),
            "the epoch is not a plausible now"
        );
        assert!(!clock_looks_plausible(CLOCK_PLAUSIBLE_FLOOR_SECS - 1));
        assert!(clock_looks_plausible(CLOCK_PLAUSIBLE_FLOOR_SECS));
        assert!(clock_looks_plausible(CLOCK_PLAUSIBLE_FLOOR_SECS + 1));
        // A reading from this project's actual deployment era.
        assert!(clock_looks_plausible(1_788_480_000));
    }

    /// The years the two old constants disagreed about.
    ///
    /// `--doctor`'s floor was 2020-01-01 and the capture supervisor's was
    /// 2024-01-01, so for any reading in these four years the diagnostic said
    /// the clock was fine while the supervisor disabled the schedule. With one
    /// constant there is no such window, and this sweeps it to say so.
    #[test]
    fn the_four_years_the_two_old_floors_disagreed_about_are_all_implausible() {
        const OLD_DOCTOR_FLOOR: u64 = 1_577_836_800; // 2020-01-01
        const { assert!(OLD_DOCTOR_FLOOR < CLOCK_PLAUSIBLE_FLOOR_SECS) };
        let mut secs = OLD_DOCTOR_FLOOR;
        while secs < CLOCK_PLAUSIBLE_FLOOR_SECS {
            assert!(
                !clock_looks_plausible(secs),
                "{secs} was plausible to the doctor and implausible to the \
                 supervisor; one constant must answer once"
            );
            secs += 86_400 * 7;
        }
    }
}

#[cfg(test)]
mod date_plausibility_tests {
    use super::date_looks_plausible;

    #[test]
    fn the_epoch_and_everything_near_it_is_implausible() {
        for d in ["1970-01-01", "1970-01-02", "1999-12-31", "2023-12-31"] {
            assert!(!date_looks_plausible(d), "{d} must not be filed");
        }
    }

    #[test]
    fn a_real_recording_date_is_plausible() {
        for d in ["2024-01-01", "2026-05-19", "2030-12-31"] {
            assert!(
                date_looks_plausible(d),
                "{d} is a date a station records on"
            );
        }
    }

    /// A date that names no day cannot be filed under one, and fails closed.
    #[test]
    fn an_unparseable_date_is_implausible() {
        for d in ["", "not-a-date", "2026-13-01", "2026-01-32", "26-01-01"] {
            assert!(!date_looks_plausible(d), "{d} must not parse");
        }
    }

    /// The boundary, both sides, so a predicate that rejects everything cannot
    /// pass the tests above.
    #[test]
    fn the_boundary_is_the_floors_own_day() {
        assert!(!date_looks_plausible("2023-12-31"));
        assert!(date_looks_plausible("2024-01-01"));
    }
}
