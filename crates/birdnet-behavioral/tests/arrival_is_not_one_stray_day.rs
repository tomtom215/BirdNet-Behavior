//! RES-19: an arrival date is not moved to January by one stray detection, and
//! an arrival that follows an outage says so.
//!
//! `phenology_timing_sql` reported only `MIN` and `MAX` of the detection date,
//! so a single false positive in January became the species' arrival for the
//! year; and nothing said whether the station had been listening in the days
//! before an arrival, so "first heard on 4 April" after a month with the
//! microphone unplugged read as a date.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_behavioral::phenology::{PhenologyParams, timing};
use tempfile::TempDir;

/// Every day from `start` for `days` days, as `YYYY-MM-DD` strings, computed
/// by `DuckDB` so the fixture does no calendar arithmetic of its own.
fn days(db: &AnalyticsDb, start: &str, days: i64) -> Vec<String> {
    let mut stmt = db
        .conn()
        .prepare(&format!(
            "SELECT CAST(DATE '{start}' + CAST(i AS INTEGER) AS VARCHAR) FROM range({days}) t(i) ORDER BY i"
        ))
        .expect("prepare days");
    stmt.query_map([], |r| r.get::<_, String>(0))
        .expect("days")
        .map(|r| r.expect("day"))
        .collect()
}

/// Swallow (2025): one stray detection on 2 January, then daily from 1 April
/// to 30 June. Blackbird (2025): daily from 1 May to 30 June.
/// Effort (2025): an hour a day from 1 January to 28 February and from 1 April
/// to 30 June — an outage through March.
/// Wren (2024): daily through May 2024, before any effort was recorded.
fn store() -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    let mut rows = vec![("2025-01-02".to_owned(), "Hirundo rustica", "Barn Swallow")];
    rows.extend(
        days(&db, "2025-04-01", 91)
            .into_iter()
            .map(|d| (d, "Hirundo rustica", "Barn Swallow")),
    );
    rows.extend(
        days(&db, "2025-05-01", 61)
            .into_iter()
            .map(|d| (d, "Turdus merula", "Eurasian Blackbird")),
    );
    rows.extend(
        days(&db, "2024-05-01", 31)
            .into_iter()
            .map(|d| (d, "Troglodytes troglodytes", "Eurasian Wren")),
    );
    let values: Vec<String> = rows
        .iter()
        .map(|(d, sci, com)| {
            format!(
                "('{d}','06:00:00','{sci}','{com}',0.9,'{com}-{d}.wav',\
                  epoch(TIMESTAMP '{d} 06:00:00'))"
            )
        })
        .collect();
    let mut effort = days(&db, "2025-01-01", 59);
    effort.extend(days(&db, "2025-04-01", 91));
    let effort: Vec<String> = effort
        .iter()
        .map(|d| format!("('{d}','local',3600)"))
        .collect();
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, detected_at_utc)
             VALUES {};
             CREATE TABLE IF NOT EXISTS recording_effort (date TEXT, source TEXT, seconds DOUBLE);
             INSERT INTO recording_effort VALUES {};",
            values.join(","),
            effort.join(",")
        ))
        .expect("seed");
    (db, dir)
}

/// One row of the timing query.
#[derive(Debug)]
struct Timing {
    species: String,
    year: i64,
    first_detection: String,
    arrival: Option<String>,
    departure: Option<String>,
    gap_before: Option<bool>,
}

fn timings(db: &AnalyticsDb) -> Vec<Timing> {
    let params = PhenologyParams {
        min_detections: 1,
        ..PhenologyParams::default()
    };
    let sql = timing::phenology_timing_sql(&params);
    let mut stmt = db.conn().prepare(&sql).expect("prepare");
    stmt.query_map([], |r| {
        Ok(Timing {
            species: r.get("species")?,
            year: r.get("year")?,
            first_detection: r.get("first_detection")?,
            // Read leniently so the gate reports what a reader is told about
            // the arrival, not that a column is missing.
            arrival: r.get::<_, Option<String>>("arrival_q05").ok().flatten(),
            departure: r.get::<_, Option<String>>("departure_q95").ok().flatten(),
            gap_before: r
                .get::<_, Option<bool>>("effort_gap_before_arrival")
                .ok()
                .flatten(),
        })
    })
    .expect("query")
    .map(|r| r.expect("row"))
    .collect()
}

fn of<'a>(out: &'a [Timing], species: &str) -> &'a Timing {
    out.iter()
        .find(|t| t.species == species)
        .unwrap_or_else(|| panic!("{species}: {out:?}"))
}

/// The acceptance gate: one January detection plus a run from 1 April is an
/// April arrival, and the March outage before it is flagged.
#[test]
fn one_stray_detection_does_not_make_a_january_arrival() {
    let (db, _dir) = store();
    let out = timings(&db);
    let swallow = of(&out, "Barn Swallow");
    // Counterpart: the exact first detection is still reported as it was.
    assert_eq!(swallow.first_detection, "2025-01-02", "{swallow:?}");
    let arrival = swallow.arrival.as_deref().unwrap_or_else(|| {
        panic!(
            "no quantile arrival: the only arrival reported is first_detection = {}, \
             one stray January detection ahead of a run from 1 April: {swallow:?}",
            swallow.first_detection
        )
    });
    assert!(
        arrival.starts_with("2025-04-"),
        "the arrival must be in April, not on the stray day: {swallow:?}"
    );
    assert_eq!(
        swallow.gap_before,
        Some(true),
        "nothing was listened to through March, so the April arrival follows an \
         effort gap: {swallow:?}"
    );
    assert_eq!(
        swallow.departure.as_deref(),
        Some("2025-06-26"),
        "{swallow:?}"
    );
}

/// Counterparts: an arrival with listening on each of the 14 days before it is
/// not flagged, and an arrival before any effort was recorded is unknown
/// (NULL), not "no gap".
#[test]
fn a_listened_run_up_is_no_gap_and_an_unrecorded_one_is_unknown() {
    let (db, _dir) = store();
    let out = timings(&db);
    let blackbird = of(&out, "Eurasian Blackbird");
    assert!(
        blackbird
            .arrival
            .as_deref()
            .is_some_and(|a| a.starts_with("2025-05-")),
        "{blackbird:?}"
    );
    assert_eq!(blackbird.gap_before, Some(false), "{blackbird:?}");
    let wren = of(&out, "Eurasian Wren");
    assert_eq!(wren.year, 2024, "{wren:?}");
    assert!(wren.arrival.is_some(), "{wren:?}");
    assert_eq!(
        wren.gap_before, None,
        "no effort was recorded on or before 2024, so the station cannot say: {wren:?}"
    );
}

/// N26-05: the run-up is judged by the one listened-day rule (half the
/// station's typical day, `site_effort::LISTENED_DAY` in `birdnet-db`), not by
/// "any audio at all". A station that analysed an hour a day and then, for the
/// eleven days before an arrival, a minute a day (an SD card full, a stream
/// dropping) did not listen to those days; the arrival after them follows a
/// gap. Counterpart: the same arrival after full days is not flagged (the
/// Blackbird of [`store`]).
#[test]
fn a_run_up_of_minute_long_days_is_a_gap() {
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    let detections: Vec<String> = days(&db, "2025-05-01", 61)
        .iter()
        .map(|d| {
            format!(
                "('{d}','06:00:00','Turdus merula','Eurasian Blackbird',0.9,'b-{d}.wav',\
                  epoch(TIMESTAMP '{d} 06:00:00'))"
            )
        })
        .collect();
    let effort: Vec<String> = days(&db, "2025-01-01", 181)
        .iter()
        .map(|d| {
            let short = ("2025-04-20".."2025-05-01").contains(&d.as_str());
            format!("('{d}','local',{})", if short { 60 } else { 3600 })
        })
        .collect();
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, detected_at_utc)
             VALUES {};
             CREATE TABLE IF NOT EXISTS recording_effort (date TEXT, source TEXT, seconds DOUBLE);
             INSERT INTO recording_effort VALUES {};",
            detections.join(","),
            effort.join(",")
        ))
        .expect("seed");
    let out = timings(&db);
    let blackbird = of(&out, "Eurasian Blackbird");
    assert_eq!(
        blackbird.gap_before,
        Some(true),
        "eleven days of a minute each, against a typical hour, are not listened days: \
         {blackbird:?}"
    );
}
