//! Insight queries: who opens the dawn chorus, who has just arrived, and who
//! has gone quiet.
//!
//! Each is a counterpart of one of birdnet-go's `/api/v2/insights/*` answers,
//! computed the same way so an API client gets the same shape of answer from
//! either station. All three read `detections_analytic`, which already leaves
//! out detections a reviewer rejected and, when the operator asked, imported
//! ones.
//!
//! "Today" is a parameter rather than SQLite's `date('now')`: the callers pass
//! the station's local date, and the tests pin it.

use rusqlite::Connection;

use crate::sqlite::connection::DbError;

/// The dawn window: a species' first detection of the morning counts when it
/// falls in `[DAWN_START_HOUR, DAWN_END_HOUR)` local time — upstream's 04:00 to
/// 10:00.
pub const DAWN_START_HOUR: u32 = 4;
/// End of the dawn window (exclusive).
pub const DAWN_END_HOUR: u32 = 10;

/// One species' place in the dawn chorus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DawnFirstSong {
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Mean, over the mornings it was heard, of its first dawn detection, in
    /// seconds after local midnight.
    pub avg_first_secs: u32,
    /// Its earliest first dawn detection in the period, in seconds after
    /// local midnight.
    pub earliest_secs: u32,
    /// Mornings it was heard in the dawn window.
    pub days_observed: u32,
}

/// Each species' typical first song of the morning over the `period_days`
/// days up to and including `today`, for species heard at dawn on at least
/// `min_days` of them, earliest riser first.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn dawn_first_songs(
    conn: &Connection,
    today: &str,
    period_days: u32,
    min_days: u32,
) -> Result<Vec<DawnFirstSong>, DbError> {
    let mut stmt = conn.prepare(
        "WITH mornings AS (
             SELECT Sci_Name, MAX(Com_Name) AS com, Date,
                    MIN(CAST(substr(Time, 1, 2) AS INTEGER) * 3600
                        + CAST(substr(Time, 4, 2) AS INTEGER) * 60
                        + CAST(substr(Time, 7, 2) AS INTEGER)) AS first_secs
             FROM detections_analytic
             WHERE Date > date(?1, ?2) AND Date <= ?1
               AND length(Time) >= 8
               AND CAST(substr(Time, 1, 2) AS INTEGER) >= ?3
               AND CAST(substr(Time, 1, 2) AS INTEGER) < ?4
             GROUP BY Sci_Name, Date
         )
         SELECT Sci_Name, MAX(com), CAST(AVG(first_secs) AS INTEGER),
                MIN(first_secs), COUNT(*) AS days
         FROM mornings
         GROUP BY Sci_Name
         HAVING days >= ?5
         ORDER BY AVG(first_secs), Sci_Name",
    )?;
    let back = format!("-{period_days} days");
    let rows = stmt
        .query_map(
            rusqlite::params![today, back, DAWN_START_HOUR, DAWN_END_HOUR, min_days],
            |row| {
                Ok(DawnFirstSong {
                    sci_name: row.get(0)?,
                    com_name: row.get(1)?,
                    avg_first_secs: row.get(2)?,
                    earliest_secs: row.get(3)?,
                    days_observed: row.get(4)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// A species first heard at this station recently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewArrival {
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Date of its first-ever detection here (`YYYY-MM-DD`).
    pub first_detected: String,
    /// Detections of it so far.
    pub detection_count: i64,
}

/// Species whose first-ever detection falls in the `recent_days` days up to
/// and including `today`, newest first.
///
/// A row with an empty `Date` (an import whose source had none) is not a
/// first detection: it would sort before every real date and hide the
/// species' arrival for good.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn new_arrivals(
    conn: &Connection,
    today: &str,
    recent_days: u32,
) -> Result<Vec<NewArrival>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Sci_Name, MAX(Com_Name), MIN(NULLIF(Date, '')) AS first, COUNT(*)
         FROM detections_analytic
         WHERE Date <= ?1
         GROUP BY Sci_Name
         HAVING first > date(?1, ?2)
         ORDER BY first DESC, Sci_Name",
    )?;
    let back = format!("-{recent_days} days");
    let rows = stmt
        .query_map(rusqlite::params![today, back], |row| {
            Ok(NewArrival {
                sci_name: row.get(0)?,
                com_name: row.get(1)?,
                first_detected: row.get(2)?,
                detection_count: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// A species heard often enough before that its silence means something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoneQuiet {
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Date of its last detection (`YYYY-MM-DD`).
    pub last_detected: String,
    /// Whole days from that date to `today`.
    pub days_since: i64,
    /// Detections of it in all.
    pub total_detections: i64,
}

/// Species with at least `min_total` detections and none in the
/// `recent_days` days up to and including `today`, most recently heard first.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn gone_quiet(
    conn: &Connection,
    today: &str,
    recent_days: u32,
    min_total: u32,
) -> Result<Vec<GoneQuiet>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Sci_Name, MAX(Com_Name), MAX(Date) AS last,
                CAST(julianday(?1) - julianday(MAX(Date)) AS INTEGER), COUNT(*) AS n
         FROM detections_analytic
         WHERE Date <= ?1
         GROUP BY Sci_Name
         HAVING n >= ?3 AND last <= date(?1, ?2)
         ORDER BY last DESC, Sci_Name",
    )?;
    let back = format!("-{recent_days} days");
    let rows = stmt
        .query_map(rusqlite::params![today, back, min_total], |row| {
            Ok(GoneQuiet {
                sci_name: row.get(0)?,
                com_name: row.get(1)?,
                last_detected: row.get(2)?,
                days_since: row.get(3)?,
                total_detections: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// A species this station has heard around this date in earlier years.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedSpecies {
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Earlier years in which it was heard within the window of this date.
    pub years_seen: u32,
    /// Its latest detection inside any of those windows (`YYYY-MM-DD`).
    pub last_seen_date: String,
}

/// Species heard within `window_days` days either side of `today`'s date in
/// any of the `max_years` years before it, the most regular first; and how
/// many of those years have any detection in the window at all.
///
/// A window is `date(today, '-N years', '∓window_days days')`, so one that
/// crosses New Year or a month end is still one window, and a species heard
/// on both sides of a New Year counts that year once.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn expected_today(
    conn: &Connection,
    today: &str,
    window_days: u32,
    max_years: u32,
) -> Result<(Vec<ExpectedSpecies>, u32), DbError> {
    const WINDOWS: &str = "WITH RECURSIVE yrs(n) AS (
             SELECT 1 UNION ALL SELECT n + 1 FROM yrs WHERE n < ?3
         ),
         windows AS (
             SELECT n,
                    date(?1, '-' || n || ' years', '-' || ?2 || ' days') AS lo,
                    date(?1, '-' || n || ' years', '+' || ?2 || ' days') AS hi
             FROM yrs
         ),
         heard AS (
             SELECT w.n, d.Sci_Name, d.Com_Name, d.Date
             FROM windows w
             JOIN detections_analytic d ON d.Date BETWEEN w.lo AND w.hi
         )";
    let mut stmt = conn.prepare(&format!(
        "{WINDOWS}
         SELECT Sci_Name, MAX(Com_Name), COUNT(DISTINCT n) AS years, MAX(Date) AS last
         FROM heard
         GROUP BY Sci_Name
         ORDER BY years DESC, last DESC, Sci_Name"
    ))?;
    let params = rusqlite::params![today, window_days, max_years];
    let species = stmt
        .query_map(params, |row| {
            Ok(ExpectedSpecies {
                sci_name: row.get(0)?,
                com_name: row.get(1)?,
                years_seen: row.get(2)?,
                last_seen_date: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let years_of_data: u32 = conn.query_row(
        &format!("{WINDOWS} SELECT COUNT(DISTINCT n) FROM heard"),
        params,
        |row| row.get(0),
    )?;
    Ok((species, years_of_data))
}

/// The dashboard's headline numbers, as birdnet-go's `/dashboard/kpis`
/// computes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DashboardKpis {
    /// Distinct species ever detected.
    pub lifetime_species: i64,
    /// Detections dated `today`.
    pub today_detections: i64,
    /// The busiest day from the same date a year ago up to and including
    /// `today`, and its count; `None` with no detections in that span. Ties
    /// go to the more recent day.
    pub best_day: Option<(String, i64)>,
    /// Consecutive days with a detection, ending `today`; 0 when `today` has
    /// none.
    pub streak_days: i64,
    /// The first day of that streak, when there is one.
    pub streak_start: Option<String>,
}

/// The dashboard's headline numbers as of `today`.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn dashboard_kpis(conn: &Connection, today: &str) -> Result<DashboardKpis, DbError> {
    let (lifetime_species, today_detections): (i64, i64) = conn.query_row(
        "SELECT COUNT(DISTINCT Sci_Name),
                COALESCE(SUM(CASE WHEN Date = ?1 THEN 1 ELSE 0 END), 0)
         FROM detections_analytic
         WHERE Date <= ?1",
        [today],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let best_day = conn
        .query_row(
            "SELECT Date, COUNT(*) AS n FROM detections_analytic
             WHERE Date >= date(?1, '-1 year') AND Date <= ?1
             GROUP BY Date ORDER BY n DESC, Date DESC LIMIT 1",
            [today],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            e => Err(e),
        })?;
    // Distinct detection dates, newest first: a date belongs to the streak
    // ending today exactly when its distance back from today equals its row
    // number, which holds until the first gap and never after it.
    let (streak_days, streak_start): (i64, Option<String>) = conn.query_row(
        "SELECT COUNT(*), MIN(Date) FROM (
             SELECT Date,
                    CAST(julianday(?1) - julianday(Date) AS INTEGER) AS back,
                    ROW_NUMBER() OVER (ORDER BY Date DESC) - 1 AS rn
             FROM (SELECT DISTINCT Date FROM detections_analytic WHERE Date <= ?1)
         ) WHERE back = rn",
        [today],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(DashboardKpis {
        lifetime_species,
        today_detections,
        best_day,
        streak_days,
        streak_start,
    })
}

/// A species heard often lately, but never with much confidence.
#[derive(Debug, Clone, PartialEq)]
pub struct PhantomSpecies {
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Detections of it in the period.
    pub detection_count: i64,
    /// Their mean confidence.
    pub avg_confidence: f64,
    /// Their highest confidence.
    pub max_confidence: f64,
}

/// Species with at least `min_detections` detections in the `period_days`
/// days up to and including `today` whose mean confidence is below
/// `max_avg_confidence`, the least confident first.
///
/// birdnet-go's phantom-species insight: a cue to look for a noise source the
/// model keeps mistaking for a bird.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn phantom_species(
    conn: &Connection,
    today: &str,
    period_days: u32,
    min_detections: u32,
    max_avg_confidence: f64,
) -> Result<Vec<PhantomSpecies>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Sci_Name, MAX(Com_Name), COUNT(*) AS n,
                AVG(Confidence) AS mean, MAX(Confidence)
         FROM detections_analytic
         WHERE Date > date(?1, '-' || ?2 || ' days') AND Date <= ?1
         GROUP BY Sci_Name
         HAVING n >= ?3 AND mean < ?4
         ORDER BY mean, Sci_Name",
    )?;
    let rows = stmt
        .query_map(
            rusqlite::params![today, period_days, min_detections, max_avg_confidence],
            |row| {
                Ok(PhantomSpecies {
                    sci_name: row.get(0)?,
                    com_name: row.get(1)?,
                    detection_count: row.get(2)?,
                    avg_confidence: row.get(3)?,
                    max_confidence: row.get(4)?,
                })
            },
        )?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// One species' figures over a date range.
#[derive(Debug, Clone, PartialEq)]
pub struct SpeciesRangeSummary {
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Detections in the range.
    pub count: i64,
    /// Their mean confidence.
    pub avg_confidence: f64,
    /// Their highest confidence.
    pub max_confidence: f64,
    /// The earliest of them: `Date`, `Time` and `detected_at_utc`. `None`
    /// when none of them has a date and time that name an instant.
    pub first: Option<(String, String, i64)>,
    /// The latest of them, likewise.
    pub last: Option<(String, String, i64)>,
}

/// Per-species count, mean and highest confidence, and first and last
/// detection, over `start..=end` (either end open when `None`), the most
/// detected first.
///
/// First and last are by instant (`detected_at_utc`), so the repeated autumn
/// hour orders correctly. A row without one — its `Date`/`Time` name no point
/// in time — counts, but cannot be first or last.
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn species_range_summary(
    conn: &Connection,
    start: Option<&str>,
    end: Option<&str>,
) -> Result<Vec<SpeciesRangeSummary>, DbError> {
    // Each of `firsts` and `lasts` holds a single MIN or MAX, so SQLite takes
    // the bare `Date`, `Time` from the row that supplied it. Both skip a NULL
    // instant, and a species with none at all gets a NULL `utc`: no place.
    // A `Date`, `Time`, instant triple from a LEFT JOIN: all NULL or none.
    fn placed(
        row: &rusqlite::Row<'_>,
        at: usize,
    ) -> rusqlite::Result<Option<(String, String, i64)>> {
        let utc: Option<i64> = row.get(at + 2)?;
        utc.map(|utc| Ok((row.get(at)?, row.get(at + 1)?, utc)))
            .transpose()
    }
    let mut stmt = conn.prepare(
        "WITH d AS (
             SELECT * FROM detections_analytic
             WHERE (?1 IS NULL OR Date >= ?1) AND (?2 IS NULL OR Date <= ?2)
         ),
         stats AS (
             SELECT Sci_Name, MAX(Com_Name) AS com, COUNT(*) AS n,
                    AVG(Confidence) AS mean, MAX(Confidence) AS top
             FROM d GROUP BY Sci_Name
         ),
         firsts AS (
             SELECT Sci_Name, MIN(detected_at_utc) AS utc, Date, Time
             FROM d GROUP BY Sci_Name
         ),
         lasts AS (
             SELECT Sci_Name, MAX(detected_at_utc) AS utc, Date, Time
             FROM d GROUP BY Sci_Name
         )
         SELECT s.Sci_Name, s.com, s.n, s.mean, s.top,
                f.Date, f.Time, f.utc, l.Date, l.Time, l.utc
         FROM stats s
         LEFT JOIN firsts f ON f.Sci_Name = s.Sci_Name
         LEFT JOIN lasts l ON l.Sci_Name = s.Sci_Name
         ORDER BY s.n DESC, s.Sci_Name",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![start, end], |row| {
            Ok(SpeciesRangeSummary {
                sci_name: row.get(0)?,
                com_name: row.get(1)?,
                count: row.get(2)?,
                avg_confidence: row.get(3)?,
                max_confidence: row.get(4)?,
                first: placed(row, 5)?,
                last: placed(row, 8)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// A species whose first detection here falls in a date range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstHeard {
    /// Scientific name.
    pub sci_name: String,
    /// Common name.
    pub com_name: String,
    /// Date of its first-ever detection here (`YYYY-MM-DD`).
    pub first_date: String,
    /// Its detections within the range.
    pub count_in_period: i64,
}

/// Species whose first-ever detection falls in `start..=end`, newest first,
/// a page of `limit` from `offset`: birdnet-go's new-species list.
///
/// Rows with an empty `Date` are not first detections, as in
/// [`new_arrivals`].
///
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn first_heard_between(
    conn: &Connection,
    start: &str,
    end: &str,
    limit: u32,
    offset: u32,
) -> Result<Vec<FirstHeard>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Sci_Name, MAX(Com_Name), MIN(NULLIF(Date, '')) AS first,
                SUM(Date BETWEEN ?1 AND ?2)
         FROM detections_analytic
         GROUP BY Sci_Name
         HAVING first BETWEEN ?1 AND ?2
         ORDER BY first DESC, Sci_Name
         LIMIT ?3 OFFSET ?4",
    )?;
    let rows = stmt
        .query_map(rusqlite::params![start, end, limit, offset], |row| {
            Ok(FirstHeard {
                sci_name: row.get(0)?,
                com_name: row.get(1)?,
                first_date: row.get(2)?,
                count_in_period: row.get(3)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station(rows: &[(&str, &str, &str)]) -> Connection {
        let conn = Connection::open_in_memory().expect("db");
        crate::migration::migrate(&conn).expect("migrate");
        for (date, time, sci) in rows {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES (?1, ?2, ?3, ?3 || ' common', 0.9)",
                rusqlite::params![date, time, sci],
            )
            .expect("insert");
        }
        conn
    }

    /// Upstream's statistic: the average and earliest of each morning's first
    /// dawn detection, over species heard on at least `min_days` mornings,
    /// earliest riser first. Song after the window and outside the period is
    /// not counted.
    #[test]
    fn the_dawn_chorus_is_each_species_typical_first_song() {
        let conn = station(&[
            // Robin: first songs 04:30 and 05:30 → average 05:00, earliest 04:30.
            ("2026-10-10", "04:30:00", "Erithacus rubecula"),
            ("2026-10-10", "06:00:00", "Erithacus rubecula"),
            ("2026-10-11", "05:30:00", "Erithacus rubecula"),
            // After the window: not a first song.
            ("2026-10-12", "11:00:00", "Erithacus rubecula"),
            // Wren: 05:10 twice → after the robin.
            ("2026-10-10", "05:10:00", "Troglodytes troglodytes"),
            ("2026-10-11", "05:10:00", "Troglodytes troglodytes"),
            // Blackbird: one morning only — below min_days.
            ("2026-10-11", "04:00:00", "Turdus merula"),
            // Outside the 7-day period: does not count for the wren.
            ("2026-09-01", "04:00:00", "Troglodytes troglodytes"),
        ]);
        let songs = dawn_first_songs(&conn, "2026-10-12", 7, 2).expect("query");
        let names: Vec<&str> = songs.iter().map(|s| s.sci_name.as_str()).collect();
        assert_eq!(names, ["Erithacus rubecula", "Troglodytes troglodytes"]);
        assert_eq!(songs[0].avg_first_secs, 5 * 3600);
        assert_eq!(songs[0].earliest_secs, 4 * 3600 + 30 * 60);
        assert_eq!(songs[0].days_observed, 2);
        assert_eq!(songs[1].earliest_secs, 5 * 3600 + 10 * 60);

        // min_days 1 lets the blackbird in, and it opens the chorus.
        let all = dawn_first_songs(&conn, "2026-10-12", 7, 1).expect("query");
        assert_eq!(all[0].sci_name, "Turdus merula");
    }

    /// A first detection inside the window is an arrival; one on its edge or
    /// before is not. Silence is counted from the last detection, and a
    /// species heard too seldom to miss is left out.
    #[test]
    fn arrivals_and_silences_are_counted_from_today() {
        let mut rows = vec![
            // First heard 3 days ago: an arrival.
            ("2026-10-09", "07:00:00", "Fringilla montifringilla"),
            // First heard exactly 14 days ago: on the edge, not an arrival.
            ("2026-09-28", "07:00:00", "Regulus regulus"),
            // Heard five times, last 20 days ago: gone quiet.
            ("2026-09-22", "07:00:00", "Sylvia atricapilla"),
        ];
        for d in ["2026-09-18", "2026-09-19", "2026-09-20", "2026-09-21"] {
            rows.push((d, "07:00:00", "Sylvia atricapilla"));
        }
        // Heard four times, long ago: too seldom to be missed.
        for d in ["2026-08-01", "2026-08-02", "2026-08-03", "2026-08-04"] {
            rows.push((d, "07:00:00", "Hirundo rustica"));
        }
        let conn = station(&rows);
        let arrived = new_arrivals(&conn, "2026-10-12", 14).expect("query");
        assert_eq!(arrived.len(), 1, "{arrived:?}");
        assert_eq!(arrived[0].sci_name, "Fringilla montifringilla");
        assert_eq!(arrived[0].first_detected, "2026-10-09");

        let quiet = gone_quiet(&conn, "2026-10-12", 14, 5).expect("query");
        assert_eq!(quiet.len(), 1, "{quiet:?}");
        assert_eq!(quiet[0].sci_name, "Sylvia atricapilla");
        assert_eq!(quiet[0].days_since, 20);
        assert_eq!(quiet[0].total_detections, 5);
    }

    /// A detection a reviewer rejected is not evidence of a first song, an
    /// arrival or a presence: the queries read `detections_analytic`.
    #[test]
    fn a_rejected_detection_counts_for_nothing() {
        let conn = station(&[("2026-10-10", "04:30:00", "Turdus merula")]);
        conn.execute("UPDATE detections SET review_verdict = 'rejected'", [])
            .expect("reject");
        assert!(
            dawn_first_songs(&conn, "2026-10-12", 7, 1)
                .expect("query")
                .is_empty()
        );
        assert!(
            new_arrivals(&conn, "2026-10-12", 14)
                .expect("query")
                .is_empty()
        );
    }

    /// Upstream's expected-today: the ±3-day window around this date in each
    /// earlier year. A window that crosses New Year is one window, and this
    /// year's own detections do not count — they are not "expected", they
    /// are here.
    #[test]
    fn expected_species_come_from_this_date_in_earlier_years() {
        let conn = station(&[
            // Heard around 12 October in two earlier years.
            ("2025-10-10", "07:00:00", "Turdus iliacus"),
            ("2024-10-14", "07:00:00", "Turdus iliacus"),
            // One earlier year, at the window's edge.
            ("2023-10-15", "07:00:00", "Turdus pilaris"),
            // Just outside the window: not expected.
            ("2025-10-16", "07:00:00", "Hirundo rustica"),
            // This year: here, not expected.
            ("2026-10-11", "07:00:00", "Sturnus vulgaris"),
            // Eleven years back: beyond max_years.
            ("2015-10-12", "07:00:00", "Ciconia ciconia"),
        ]);
        let (species, years) = expected_today(&conn, "2026-10-12", 3, 10).expect("query");
        let names: Vec<&str> = species.iter().map(|s| s.sci_name.as_str()).collect();
        assert_eq!(names, ["Turdus iliacus", "Turdus pilaris"]);
        assert_eq!(species[0].years_seen, 2);
        assert_eq!(species[0].last_seen_date, "2025-10-10");
        assert_eq!(years, 3, "2025, 2024 and 2023 have data in the window");

        // Across New Year: 30 December and 2 January of the same winter are
        // one window, counted once, for a date of 1 January.
        let conn = station(&[
            ("2025-12-30", "07:00:00", "Bombycilla garrulus"),
            ("2026-01-02", "07:00:00", "Bombycilla garrulus"),
        ]);
        let (species, years) = expected_today(&conn, "2027-01-01", 3, 10).expect("query");
        assert_eq!(species.len(), 1, "{species:?}");
        assert_eq!(species[0].years_seen, 1);
        assert_eq!(years, 1);
    }

    /// A streak counts back from today and stops at the first gap; a day
    /// without detections today is no streak at all, however long the run
    /// before it. The best day is the busiest since this date last year.
    #[test]
    fn kpis_count_the_streak_back_from_today() {
        let mut rows = vec![
            ("2026-10-12", "07:00:00", "Turdus merula"),
            ("2026-10-11", "07:00:00", "Turdus merula"),
            ("2026-10-10", "07:00:00", "Erithacus rubecula"),
            // A gap on the 9th: the 8th is not in the streak.
            ("2026-10-08", "07:00:00", "Turdus merula"),
            // The busiest day of the year, three detections.
            ("2026-03-01", "07:00:00", "Turdus merula"),
            ("2026-03-01", "08:00:00", "Turdus merula"),
            ("2026-03-01", "09:00:00", "Parus major"),
        ];
        // Busier, but more than a year ago.
        for t in ["01", "02", "03", "04", "05"] {
            rows.push(("2025-10-01", t, "Sitta europaea"));
        }
        let conn = station(&rows);
        let k = dashboard_kpis(&conn, "2026-10-12").expect("query");
        assert_eq!(k.best_day, Some(("2026-03-01".to_owned(), 3)));
        // As busy as 1 March, and exactly a year back: inside the span, as
        // upstream counts it, but the less recent of the two.
        for t in ["01", "02", "03"] {
            rows.push(("2025-10-12", t, "Sitta europaea"));
        }
        let k = dashboard_kpis(&station(&rows), "2026-10-12").expect("query");
        assert_eq!(k.best_day, Some(("2026-03-01".to_owned(), 3)));
        // One more and it is the best day: the boundary is inclusive.
        rows.push(("2025-10-12", "04", "Sitta europaea"));
        let conn = station(&rows);
        let k = dashboard_kpis(&conn, "2026-10-12").expect("query");
        assert_eq!(k.streak_days, 3);
        assert_eq!(k.streak_start.as_deref(), Some("2026-10-10"));
        assert_eq!(k.today_detections, 1);
        assert_eq!(k.lifetime_species, 4);
        assert_eq!(k.best_day, Some(("2025-10-12".to_owned(), 4)));

        // As of the 11th, the 12th is ahead of today and does not shift the
        // count back.
        let k = dashboard_kpis(&conn, "2026-10-11").expect("query");
        assert_eq!(k.streak_days, 2);
        assert_eq!(k.streak_start.as_deref(), Some("2026-10-10"));

        // Nothing yet today: no streak, though yesterday ended a run.
        let k = dashboard_kpis(&conn, "2026-10-13").expect("query");
        assert_eq!(k.streak_days, 0);
        assert_eq!(k.streak_start, None);
        assert_eq!(k.today_detections, 0);

        // An empty station: zeros and no best day, not an error.
        let k = dashboard_kpis(&station(&[]), "2026-10-12").expect("query");
        assert_eq!(
            (k.lifetime_species, k.streak_days, k.best_day),
            (0, 0, None)
        );
    }
    /// Upstream's phantom-species rule: enough detections in the period, a
    /// mean confidence under the threshold, the least confident first. A
    /// species confident on average, too rare, heard only outside the period,
    /// or rejected by a reviewer is not one.
    #[test]
    fn phantoms_are_frequent_and_unconfident_in_the_period() {
        let conn = station(&[]);
        let rows: &[(&str, &str, f64)] = &[
            // Three low-confidence detections inside the 30 days.
            ("2026-10-12", "Strix aluco", 0.40),
            ("2026-10-01", "Strix aluco", 0.50),
            ("2026-09-13", "Strix aluco", 0.30),
            // Less unconfident, so listed second.
            ("2026-10-12", "Bubo bubo", 0.50),
            ("2026-10-11", "Bubo bubo", 0.55),
            ("2026-10-10", "Bubo bubo", 0.59),
            // A mean of exactly the threshold is not under it.
            ("2026-10-12", "Asio otus", 0.60),
            ("2026-10-11", "Asio otus", 0.60),
            ("2026-10-10", "Asio otus", 0.60),
            // Two in the period and one the day before it: too few.
            ("2026-10-12", "Tyto alba", 0.20),
            ("2026-10-11", "Tyto alba", 0.20),
            ("2026-09-12", "Tyto alba", 0.20),
            // Tomorrow is not in the period either.
            ("2026-10-13", "Tyto alba", 0.20),
            // Rejected by a reviewer below: already dealt with.
            ("2026-10-12", "Otus scops", 0.20),
            ("2026-10-11", "Otus scops", 0.20),
            ("2026-10-10", "Otus scops", 0.20),
        ];
        for (date, sci, conf) in rows {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES (?1, '03:00:00', ?2, ?2 || ' common', ?3)",
                rusqlite::params![date, sci, conf],
            )
            .expect("insert");
        }
        conn.execute(
            "UPDATE detections SET review_verdict = 'rejected' WHERE Sci_Name = 'Otus scops'",
            [],
        )
        .expect("reject");
        let p = phantom_species(&conn, "2026-10-12", 30, 3, 0.6).expect("query");
        let names: Vec<&str> = p.iter().map(|s| s.sci_name.as_str()).collect();
        assert_eq!(names, ["Strix aluco", "Bubo bubo"]);
        assert_eq!(p[0].detection_count, 3);
        assert!((p[0].avg_confidence - 0.4).abs() < 1e-9, "{p:?}");
        assert!((p[0].max_confidence - 0.5).abs() < 1e-9, "{p:?}");
    }

    /// Upstream's species summary over a range: count, mean and top
    /// confidence, first and last — by instant, which the wall clock cannot
    /// give in the repeated autumn hour — the most detected first.
    #[test]
    fn a_range_summary_counts_and_places_each_species() {
        let conn = station(&[]);
        // 2026-10-25 01:00Z is the first 01:00 of the night the clocks go
        // back in Europe/London: 01:45 BST is 00:45Z, before 01:15 GMT.
        let t = 1_792_890_000; // 2026-10-25T01:00:00Z
        let rows: &[(&str, &str, &str, f64, Option<i64>)] = &[
            ("2026-10-25", "01:45:00", "Strix aluco", 0.5, Some(t - 900)),
            ("2026-10-25", "01:15:00", "Strix aluco", 0.9, Some(t + 900)),
            ("2026-10-20", "22:00:00", "Strix aluco", 0.7, None),
            // Outside the range on either side.
            ("2026-10-19", "22:00:00", "Strix aluco", 0.1, None),
            ("2026-10-26", "22:00:00", "Strix aluco", 0.1, None),
            ("2026-10-22", "07:00:00", "Turdus merula", 0.8, None),
            // Rejected by a reviewer below: not counted.
            ("2026-10-23", "07:00:00", "Turdus merula", 0.8, None),
        ];
        for (date, time, sci, conf, utc) in rows {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
                 VALUES (?1, ?2, ?3, ?3 || ' common', ?4, ?5)",
                rusqlite::params![date, time, sci, conf, utc],
            )
            .expect("insert");
        }
        conn.execute(
            "UPDATE detections SET review_verdict = 'rejected' WHERE Date = '2026-10-23'",
            [],
        )
        .expect("reject");
        let s =
            species_range_summary(&conn, Some("2026-10-20"), Some("2026-10-25")).expect("query");
        assert_eq!(
            s[1].count, 1,
            "the rejected blackbird is not counted: {s:?}"
        );
        let names: Vec<&str> = s.iter().map(|r| r.sci_name.as_str()).collect();
        assert_eq!(names, ["Strix aluco", "Turdus merula"]);
        let owl = &s[0];
        assert_eq!(owl.count, 3);
        assert!((owl.avg_confidence - 0.7).abs() < 1e-9, "{owl:?}");
        assert!((owl.max_confidence - 0.9).abs() < 1e-9, "{owl:?}");
        assert_eq!(owl.first.as_ref().map(|f| f.0.as_str()), Some("2026-10-20"));
        assert_eq!(
            owl.last,
            Some(("2026-10-25".to_owned(), "01:15:00".to_owned(), t + 900)),
            "the later instant, though the earlier wall clock"
        );

        // On the night itself, the first is 01:45 summer time: the earlier
        // instant, though the later wall clock.
        let s =
            species_range_summary(&conn, Some("2026-10-25"), Some("2026-10-25")).expect("query");
        assert_eq!(
            s[0].first,
            Some(("2026-10-25".to_owned(), "01:45:00".to_owned(), t - 900))
        );

        // Open at either end.
        let s = species_range_summary(&conn, None, Some("2026-10-19")).expect("query");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].count, 1);
        let s = species_range_summary(&conn, Some("2026-10-26"), None).expect("query");
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].count, 1);
        let s = species_range_summary(&conn, None, None).expect("query");
        assert_eq!(s[0].count, 5);

        // A row whose date names no instant counts, but is never first or
        // last: there is nothing to place it by.
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('', '', 'Bubo bubo', 'Eagle-Owl', 0.4)",
            [],
        )
        .expect("insert");
        let s = species_range_summary(&conn, None, None).expect("query");
        let bubo = s
            .iter()
            .find(|r| r.sci_name == "Bubo bubo")
            .expect("listed");
        assert_eq!((bubo.count, &bubo.first, &bubo.last), (1, &None, &None));
    }
    /// An import with no date is not a first detection: counted as one, it
    /// would sort before every real date and hide the species' arrival.
    #[test]
    fn an_undated_row_does_not_hide_an_arrival() {
        let conn = station(&[
            ("2026-10-10", "07:00:00", "Fringilla montifringilla"),
            ("", "", "Fringilla montifringilla"),
        ]);
        let arrivals = new_arrivals(&conn, "2026-10-12", 14).expect("query");
        assert_eq!(arrivals.len(), 1, "{arrivals:?}");
        assert_eq!(arrivals[0].first_detected, "2026-10-10");
        let firsts = first_heard_between(&conn, "2026-10-01", "2026-10-12", 100, 0).expect("query");
        assert_eq!(firsts.len(), 1, "{firsts:?}");
    }

    /// Upstream's new-species list: first-ever detection inside the range,
    /// the count within the range only, newest first, paged.
    #[test]
    fn first_heard_lists_species_new_in_the_range() {
        let conn = station(&[
            // First heard on the 5th; also on the 20th, after the range.
            ("2026-10-05", "07:00:00", "Turdus iliacus"),
            ("2026-10-06", "07:00:00", "Turdus iliacus"),
            ("2026-10-20", "07:00:00", "Turdus iliacus"),
            // First heard on the 10th, the range's last day.
            ("2026-10-10", "07:00:00", "Turdus pilaris"),
            // First heard on the 1st, the range's first day.
            ("2026-10-01", "07:00:00", "Bombycilla garrulus"),
            // Heard in the range, but first heard before it.
            ("2026-09-30", "07:00:00", "Turdus merula"),
            ("2026-10-07", "07:00:00", "Turdus merula"),
            // First heard after the range.
            ("2026-10-11", "07:00:00", "Regulus regulus"),
        ]);
        let f = first_heard_between(&conn, "2026-10-01", "2026-10-10", 100, 0).expect("query");
        let names: Vec<&str> = f.iter().map(|r| r.sci_name.as_str()).collect();
        assert_eq!(
            names,
            ["Turdus pilaris", "Turdus iliacus", "Bombycilla garrulus"]
        );
        assert_eq!(f[1].first_date, "2026-10-05");
        assert_eq!(f[1].count_in_period, 2, "the 20th is outside the range");

        let page = first_heard_between(&conn, "2026-10-01", "2026-10-10", 1, 1).expect("query");
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].sci_name, "Turdus iliacus");

        conn.execute(
            "UPDATE detections SET review_verdict = 'rejected' WHERE Sci_Name = 'Turdus pilaris'",
            [],
        )
        .expect("reject");
        let f = first_heard_between(&conn, "2026-10-01", "2026-10-10", 100, 0).expect("query");
        assert_eq!(f.len(), 2, "a rejected detection is no first: {f:?}");
    }
}
