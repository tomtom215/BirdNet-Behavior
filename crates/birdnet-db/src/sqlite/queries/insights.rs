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
/// # Errors
///
/// Returns `DbError` on query failure.
pub fn new_arrivals(
    conn: &Connection,
    today: &str,
    recent_days: u32,
) -> Result<Vec<NewArrival>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT Sci_Name, MAX(Com_Name), MIN(Date) AS first, COUNT(*)
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
}
