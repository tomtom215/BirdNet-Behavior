//! First-of-the-year, first-of-the-season, and returning-after-an-absence.
//!
//! # The unit birders actually keep score in
//!
//! A life list is the obvious thing to build from a detection table and the
//! least interesting: a species enters it once and never again, so after a
//! year at one site almost nothing new happens. What a person actually watches
//! for is *first of the year* and *first of the spring* — and a station is
//! uniquely good at catching those, because it is listening at 04:40 when
//! nobody is awake.
//!
//! # The four flags
//!
//! For a species detected on a given date:
//!
//! * **new ever** — nothing before this date. The life list.
//! * **new this year** — nothing since the tracking year began. The year list.
//! * **new this season** — nothing since the season began. The phenology
//!   signal this project's analytics already exist to measure.
//! * **returning after an absence** — there *is* something before, and the gap
//!   is at least the configured number of days. A resident is not returning;
//!   a bird absent since last autumn is.
//!
//! They overlap by design and are not mutually exclusive: a first-ever
//! detection is also a first of the year and of the season, and a caller
//! showing one badge should pick the strongest rather than expecting the
//! flags to have chosen for it.
//!
//! # Where the windows come from
//!
//! The caller passes them in as dates. The season boundary is hemisphere- and
//! latitude-dependent and lives in `birdnet_core::season`, which this crate
//! does not depend on; the tracking year's start is an operator setting whose
//! default (1 January) is itself a northern convention. Neither belongs in
//! SQL, and computing them here would mean a second copy of rules that already
//! have a home.

use rusqlite::{Connection, params};

use crate::sqlite::DbError;

/// The windows a status is computed against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackingWindows<'a> {
    /// First date of the current tracking year, `YYYY-MM-DD`.
    pub year_start: &'a str,
    /// First date of the current season, `YYYY-MM-DD`.
    ///
    /// May be in the previous calendar year — a northern winter starts in
    /// December — which is exactly why this is a date and not a month number.
    pub season_start: &'a str,
    /// Days of silence after which a detection counts as a return.
    ///
    /// `0` disables the flag rather than making every detection a return.
    pub absence_days: u32,
}

/// What is notable about one species on one date.
///
/// Four booleans, which clippy dislikes and which is right here anyway: they
/// are not a state machine with four bits, they are four independent claims
/// that genuinely overlap. A first-ever detection is *also* a first of the
/// year and of the season, and a caller filtering for "anything new this
/// season" wants that to be true. Collapsing them into an enum would force a
/// precedence at the point of measurement rather than at the point of display,
/// where [`SpeciesStatus::headline`] puts it.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SpeciesStatus {
    /// Nothing recorded before this date.
    pub new_ever: bool,
    /// Nothing recorded since the tracking year began.
    pub new_this_year: bool,
    /// Nothing recorded since the season began.
    pub new_this_season: bool,
    /// Recorded before, but not within [`TrackingWindows::absence_days`].
    pub returning_after_absence: bool,
    /// Days since the previous detection, or `None` if there was none.
    pub days_since_previous: Option<i64>,
}

impl SpeciesStatus {
    /// Whether anything about this species is worth a badge.
    #[must_use]
    pub const fn is_notable(&self) -> bool {
        self.new_ever || self.new_this_year || self.new_this_season || self.returning_after_absence
    }

    /// The single strongest claim, for a UI with room for one badge.
    ///
    /// Ordered by how rare the event is, not by how the flags are declared:
    /// a first-ever detection is also a first of the year and of the season,
    /// and showing "new this season" for a lifer would be true and useless.
    #[must_use]
    pub const fn headline(&self) -> Option<&'static str> {
        if self.new_ever {
            Some("first ever")
        } else if self.returning_after_absence {
            Some("returning")
        } else if self.new_this_year {
            Some("first this year")
        } else if self.new_this_season {
            Some("first this season")
        } else {
            None
        }
    }
}

/// One species' status on `on_date`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedSpecies {
    /// Scientific name.
    pub sci_name: String,
    /// Common name, as recorded.
    pub com_name: String,
    /// The flags.
    pub status: SpeciesStatus,
}

/// Every species detected on `on_date`, with its status.
///
/// One query for the whole date rather than one per species: the today page
/// wants all of them, and a per-species call in a loop is how a page that
/// renders forty species holds the writer lock for forty round trips.
///
/// Species are returned in common-name order, which is the order a list is
/// read in.
///
/// # Errors
///
/// Returns [`DbError`] on query failure.
pub fn statuses_for_date(
    conn: &Connection,
    on_date: &str,
    windows: TrackingWindows<'_>,
) -> Result<Vec<TrackedSpecies>, DbError> {
    // Dates are stored as `YYYY-MM-DD` text, so lexicographic comparison is
    // chronological and the indexes on `Date` are usable. `julianday` is used
    // only for the gap arithmetic, where a day count is genuinely wanted.
    let mut stmt = conn.prepare(
        "WITH seen AS (
             SELECT Sci_Name, MIN(Com_Name) AS Com_Name
               FROM detections_analytic
              WHERE Date = ?1
              GROUP BY Sci_Name
         )
         SELECT s.Sci_Name,
                s.Com_Name,
                (SELECT MIN(d.Date) FROM detections_analytic d
                  WHERE d.Sci_Name = s.Sci_Name)                          AS first_ever,
                (SELECT MIN(d.Date) FROM detections_analytic d
                  WHERE d.Sci_Name = s.Sci_Name AND d.Date >= ?2)         AS first_year,
                (SELECT MIN(d.Date) FROM detections_analytic d
                  WHERE d.Sci_Name = s.Sci_Name AND d.Date >= ?3)         AS first_season,
                (SELECT MAX(d.Date) FROM detections_analytic d
                  WHERE d.Sci_Name = s.Sci_Name AND d.Date < ?1)          AS previous
           FROM seen s
          ORDER BY s.Com_Name",
    )?;

    let rows = stmt
        .query_map(
            params![on_date, windows.year_start, windows.season_start],
            |r| {
                let sci_name: String = r.get(0)?;
                let com_name: String = r.get(1)?;
                let first_ever: Option<String> = r.get(2)?;
                let first_year: Option<String> = r.get(3)?;
                let first_season: Option<String> = r.get(4)?;
                let previous: Option<String> = r.get(5)?;
                Ok(TrackedSpecies {
                    sci_name,
                    com_name,
                    status: classify(
                        on_date,
                        first_ever.as_deref(),
                        first_year.as_deref(),
                        first_season.as_deref(),
                        previous.as_deref(),
                        windows,
                    ),
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Turn the four dates a query returns into flags.
///
/// Split out so the rules are testable without a database, and because the
/// interesting cases — a window that starts after the date, a species with no
/// history at all — are awkward to set up in SQL and trivial here.
fn classify(
    on_date: &str,
    first_ever: Option<&str>,
    first_year: Option<&str>,
    first_season: Option<&str>,
    previous: Option<&str>,
    windows: TrackingWindows<'_>,
) -> SpeciesStatus {
    let days_since_previous = previous.and_then(|p| days_between(p, on_date));

    // No guard here on the windows having begun, and the reason is worth
    // recording because the first draft had one.
    //
    // `first_year` is `MIN(Date) WHERE Date >= year_start`, so `first_year ==
    // on_date` already implies `on_date >= year_start`; the guard could never
    // change an answer, and a mutant that deleted it left every test green.
    // The comment justifying it described a failure the query cannot produce.
    // `a_window_starting_after_the_date_reports_nothing_new` pins the
    // invariant instead, which is the honest version of the same concern.
    SpeciesStatus {
        new_ever: first_ever == Some(on_date),
        new_this_year: first_year == Some(on_date),
        new_this_season: first_season == Some(on_date),
        returning_after_absence: windows.absence_days > 0
            && days_since_previous.is_some_and(|d| d >= i64::from(windows.absence_days)),
        days_since_previous,
    }
}

/// Whole days from `from` to `to`, both `YYYY-MM-DD`.
///
/// Returns `None` when either date does not parse, rather than a plausible
/// zero: a zero would make a malformed date look like "heard again today",
/// which is the reading least likely to be questioned.
fn days_between(from: &str, to: &str) -> Option<i64> {
    Some(days_from_ymd(to)? - days_from_ymd(from)?)
}

/// Days since the civil epoch for `YYYY-MM-DD`.
///
/// Howard Hinnant's `days_from_civil`, the same algorithm
/// `birdnet_core::civil` uses. Duplicated rather than depended on because
/// `birdnet-db` does not depend on `birdnet-core`, and
/// `the_two_civil_day_counts_agree` in the binary's tests pins them together.
fn days_from_ymd(date: &str) -> Option<i64> {
    let mut parts = date.split('-');
    let y: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next()?.parse().ok()?;
    let d: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let conn = Connection::open_in_memory().expect("open");
        crate::migration::migrate(&conn).expect("migrate");
        conn
    }

    /// Record one detection of `sci` on `date` at 06:00.
    fn seen(conn: &Connection, sci: &str, date: &str) {
        seen_at(conn, sci, date, "06:00:00");
    }

    /// Record one detection of `sci` on `date` at `time`.
    ///
    /// The time is a parameter because `idx_detections_unique` covers
    /// `(Date, Time, Sci_Name, File_Name, chunk_offset_secs)` — two detections
    /// of one species on one date have to differ somewhere, which is the
    /// schema being right rather than an inconvenience.
    fn seen_at(conn: &Connection, sci: &str, date: &str, time: &str) {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs)
             VALUES (?1, ?4, ?2, ?3, 0.9, 0.7, 1, 1.25, 0.0, 'x.wav', 0)",
            params![date, sci, sci, time],
        )
        .expect("insert");
    }

    fn windows<'a>(year: &'a str, season: &'a str, absence: u32) -> TrackingWindows<'a> {
        TrackingWindows {
            year_start: year,
            season_start: season,
            absence_days: absence,
        }
    }

    fn status(conn: &Connection, date: &str, w: TrackingWindows<'_>, sci: &str) -> SpeciesStatus {
        statuses_for_date(conn, date, w)
            .expect("query")
            .into_iter()
            .find(|s| s.sci_name == sci)
            .unwrap_or_else(|| panic!("{sci} not detected on {date}"))
            .status
    }

    /// A species never recorded before is new in every sense.
    #[test]
    fn a_first_ever_detection_sets_every_new_flag() {
        let conn = db();
        seen(&conn, "Strix aluco", "2026-04-10");
        let s = status(
            &conn,
            "2026-04-10",
            windows("2026-01-01", "2026-03-20", 30),
            "Strix aluco",
        );
        assert!(s.new_ever);
        assert!(s.new_this_year);
        assert!(s.new_this_season);
        assert!(
            !s.returning_after_absence,
            "there is nothing to return from"
        );
        assert_eq!(s.days_since_previous, None);
        assert_eq!(s.headline(), Some("first ever"));
    }

    /// A resident heard yesterday is new in no sense.
    ///
    /// The counterpart to every other test here: a classifier that returned
    /// all-true would pass most of them individually.
    #[test]
    fn a_resident_heard_yesterday_is_not_notable() {
        let conn = db();
        seen(&conn, "Turdus merula", "2026-04-09");
        seen(&conn, "Turdus merula", "2026-04-10");
        let s = status(
            &conn,
            "2026-04-10",
            windows("2026-01-01", "2026-03-20", 30),
            "Turdus merula",
        );
        assert!(!s.is_notable(), "a bird heard yesterday is not news: {s:?}");
        assert_eq!(s.days_since_previous, Some(1));
        assert_eq!(s.headline(), None);
    }

    /// First of the year, but not first ever and not first of the season.
    ///
    /// Recorded last November, then again in January: the year has turned but
    /// the northern winter has not.
    #[test]
    fn a_species_last_heard_last_year_is_new_this_year_only() {
        let conn = db();
        seen(&conn, "Fringilla montifringilla", "2025-11-15");
        seen(&conn, "Fringilla montifringilla", "2026-01-20");
        // Winter 2025 began on 21 December, so both dates are... not in it:
        // the November one predates it. Season start is therefore 2025-12-21.
        let s = status(
            &conn,
            "2026-01-20",
            windows("2026-01-01", "2025-12-21", 30),
            "Fringilla montifringilla",
        );
        assert!(!s.new_ever, "it was recorded in November");
        assert!(s.new_this_year, "the year turned between the two");
        assert!(
            s.new_this_season,
            "the November record predates the 21 December season start"
        );
        assert_eq!(s.days_since_previous, Some(66));
    }

    /// A season window that began *before* a species' last detection means it
    /// is not new this season, even though the calendar year turned.
    ///
    /// The discriminating half of the pair above: without it, "new this
    /// season" that simply mirrored "new this year" would pass.
    #[test]
    fn a_species_already_heard_this_season_is_not_new_this_season() {
        let conn = db();
        seen(&conn, "Fringilla montifringilla", "2025-12-28");
        seen(&conn, "Fringilla montifringilla", "2026-01-20");
        let s = status(
            &conn,
            "2026-01-20",
            windows("2026-01-01", "2025-12-21", 30),
            "Fringilla montifringilla",
        );
        assert!(
            s.new_this_year,
            "the calendar year still turned between the two"
        );
        assert!(
            !s.new_this_season,
            "it was already recorded on 28 December, inside this winter"
        );
        assert_eq!(s.headline(), Some("first this year"));
    }

    /// A bird back after a long gap is a return, not a first.
    #[test]
    fn a_long_gap_is_a_return() {
        let conn = db();
        seen(&conn, "Hirundo rustica", "2025-09-01");
        seen(&conn, "Hirundo rustica", "2026-04-12");
        let s = status(
            &conn,
            "2026-04-12",
            windows("2026-01-01", "2026-03-20", 30),
            "Hirundo rustica",
        );
        assert!(!s.new_ever);
        assert!(s.returning_after_absence);
        assert_eq!(s.days_since_previous, Some(223));
        assert_eq!(
            s.headline(),
            Some("returning"),
            "a return outranks 'first this year' for a bird that has been here before"
        );
    }

    /// The absence threshold is a boundary, and the boundary day counts.
    ///
    /// "Not heard for thirty days" must include a bird last heard exactly
    /// thirty days ago; the strict comparison that excludes it is the classic
    /// off-by-one and would make a monthly visitor never a return.
    #[test]
    fn the_absence_boundary_is_inclusive() {
        let conn = db();
        seen(&conn, "Hirundo rustica", "2026-03-13");
        seen(&conn, "Hirundo rustica", "2026-04-12");
        let w = windows("2026-01-01", "2026-03-20", 30);
        let s = status(&conn, "2026-04-12", w, "Hirundo rustica");
        assert_eq!(s.days_since_previous, Some(30));
        assert!(s.returning_after_absence, "exactly 30 days must count");

        // ...and 29 must not.
        let conn2 = db();
        seen(&conn2, "Hirundo rustica", "2026-03-14");
        seen(&conn2, "Hirundo rustica", "2026-04-12");
        let s2 = status(&conn2, "2026-04-12", w, "Hirundo rustica");
        assert_eq!(s2.days_since_previous, Some(29));
        assert!(!s2.returning_after_absence);
    }

    /// An absence threshold of zero disables the flag rather than firing it on
    /// everything.
    #[test]
    fn a_zero_absence_threshold_disables_the_return_flag() {
        let conn = db();
        seen(&conn, "Turdus merula", "2026-04-09");
        seen(&conn, "Turdus merula", "2026-04-10");
        let s = status(
            &conn,
            "2026-04-10",
            windows("2026-01-01", "2026-03-20", 0),
            "Turdus merula",
        );
        assert!(
            !s.returning_after_absence,
            "with the threshold off, a bird heard yesterday must not be a return"
        );
    }

    /// A rejected detection is not history.
    ///
    /// The query reads `detections_analytic`, which excludes anything a
    /// reviewer has rejected. A species whose only previous record was a
    /// mistake is genuinely new, and saying otherwise would make the review
    /// workflow pointless for exactly the species it matters most for.
    #[test]
    fn a_rejected_previous_detection_does_not_count_as_history() {
        let conn = db();
        seen(&conn, "Strix aluco", "2026-01-05");
        conn.execute(
            "UPDATE detections SET review_verdict = 'rejected' WHERE Date = '2026-01-05'",
            [],
        )
        .expect("reject");
        seen(&conn, "Strix aluco", "2026-04-10");

        let s = status(
            &conn,
            "2026-04-10",
            windows("2026-01-01", "2026-03-20", 30),
            "Strix aluco",
        );
        assert!(
            s.new_ever,
            "the January record was rejected, so April is the first real one"
        );
        assert_eq!(s.days_since_previous, None);
    }

    /// Species come back in common-name order, and every species detected on
    /// the date is present exactly once however many detections it has.
    #[test]
    fn a_date_lists_each_species_once_in_name_order() {
        let conn = db();
        seen_at(&conn, "Turdus merula", "2026-04-10", "06:00:00");
        seen_at(&conn, "Turdus merula", "2026-04-10", "07:30:00");
        seen(&conn, "Apus apus", "2026-04-10");
        let rows = statuses_for_date(&conn, "2026-04-10", windows("2026-01-01", "2026-03-20", 30))
            .expect("query");
        let names: Vec<&str> = rows.iter().map(|r| r.com_name.as_str()).collect();
        assert_eq!(names, vec!["Apus apus", "Turdus merula"]);
    }

    /// A date with nothing on it returns an empty list, not an error.
    #[test]
    fn a_quiet_date_returns_nothing() {
        let conn = db();
        seen(&conn, "Turdus merula", "2026-04-10");
        assert!(
            statuses_for_date(&conn, "2026-04-11", windows("2026-01-01", "2026-03-20", 30))
                .expect("query")
                .is_empty()
        );
    }

    // ── the pure classifier ─────────────────────────────────────────────

    /// Browsing an older date with today's windows reports nothing new.
    ///
    /// The real version of a concern the first draft answered with a guard in
    /// `classify`. That guard was unreachable — `MIN(Date) WHERE Date >=
    /// window` can only equal `on_date` when `on_date` is inside the window —
    /// and a mutant deleting it left every test green. The invariant is
    /// genuine and worth pinning; the guard was not.
    ///
    /// The usage this covers is a person opening last April's page while the
    /// station's current season is next winter's.
    #[test]
    fn a_window_starting_after_the_date_reports_nothing_new() {
        let conn = db();
        seen(&conn, "Hirundo rustica", "2026-04-10");
        let s = status(
            &conn,
            "2026-04-10",
            windows("2027-01-01", "2026-12-21", 30),
            "Hirundo rustica",
        );
        assert!(
            s.new_ever,
            "first-ever does not depend on a window and must still be true"
        );
        assert!(
            !s.new_this_year,
            "the tracking year begins after this date, so nothing can be new in it"
        );
        assert!(!s.new_this_season);
    }

    /// The headline picks the strongest claim, not the first flag set.
    #[test]
    fn the_headline_is_ordered_by_rarity() {
        let all = SpeciesStatus {
            new_ever: true,
            new_this_year: true,
            new_this_season: true,
            returning_after_absence: true,
            days_since_previous: Some(400),
        };
        assert_eq!(all.headline(), Some("first ever"));
        assert_eq!(
            SpeciesStatus {
                new_ever: false,
                ..all
            }
            .headline(),
            Some("returning")
        );
        assert_eq!(
            SpeciesStatus {
                new_ever: false,
                returning_after_absence: false,
                ..all
            }
            .headline(),
            Some("first this year")
        );
        assert_eq!(
            SpeciesStatus {
                new_ever: false,
                returning_after_absence: false,
                new_this_year: false,
                ..all
            }
            .headline(),
            Some("first this season")
        );
        assert_eq!(SpeciesStatus::default().headline(), None);
    }

    /// The civil day count agrees with `birdnet_core::civil` at the awkward
    /// dates.
    ///
    /// It is a second copy of Hinnant's algorithm, in a crate that cannot
    /// import the first. Leap years, century years, and the year-400 rule are
    /// where a transcription slips.
    #[test]
    fn the_day_count_handles_leap_and_century_years() {
        assert_eq!(days_between("2026-01-01", "2026-01-01"), Some(0));
        assert_eq!(days_between("2026-01-01", "2026-12-31"), Some(364));
        assert_eq!(
            days_between("2028-01-01", "2028-12-31"),
            Some(365),
            "2028 is a leap year"
        );
        assert_eq!(
            days_between("1900-02-28", "1900-03-01"),
            Some(1),
            "1900 is not a leap year: divisible by 100 but not 400"
        );
        assert_eq!(
            days_between("2000-02-28", "2000-03-01"),
            Some(2),
            "2000 is a leap year: divisible by 400"
        );
        assert_eq!(days_between("2026-04-12", "2026-03-13"), Some(-30));
        assert_eq!(days_between("not-a-date", "2026-01-01"), None);
        assert_eq!(days_between("2026-13-01", "2026-01-01"), None);
    }
}
