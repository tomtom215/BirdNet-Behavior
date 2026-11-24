//! An arrival date is the start of a run of detection days, and it says whether
//! the station was listening before it.
//!
//! `phenology_timing_sql` reported `MIN` and `MAX` of the detection date and
//! nothing else, so one false positive on 2 January moved a summer migrant's
//! "arrival" three months early, and an arrival on the day the station came
//! back from a month-long outage read exactly like one the station had been
//! waiting for. The raw first and last detections are still reported, under
//! their own names; the arrival and departure are now separate columns.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_behavioral::phenology::{PhenologyParams, timing};
use tempfile::TempDir;

/// Every day from `from` for `days` days, as `YYYY-MM-DD`, computed by DuckDB
/// so the fixture does not carry its own calendar.
fn days(db: &AnalyticsDb, from: &str, n: u32) -> Vec<String> {
    let mut stmt = db
        .conn()
        .prepare(&format!(
            "SELECT CAST(DATE '{from}' + CAST(i AS INTEGER) AS VARCHAR) FROM range({n}) t(i) ORDER BY i"
        ))
        .expect("prepare");
    stmt.query_map([], |r| r.get(0))
        .expect("query")
        .map(|r| r.expect("row"))
        .collect()
}

fn detect(db: &AnalyticsDb, species: &str, dates: &[String]) {
    let values: Vec<String> = dates
        .iter()
        .map(|d| {
            format!(
                "('{d}','06:00:00','Sci name','{species}',0.9,\
                  epoch(TIMESTAMP '{d} 06:00:00'))"
            )
        })
        .collect();
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
             VALUES {};",
            values.join(",")
        ))
        .expect("seed detections");
}

fn listen(db: &AnalyticsDb, dates: &[String]) {
    let values: Vec<String> = dates
        .iter()
        .map(|d| format!("('{d}','mic-a',3600)"))
        .collect();
    db.conn()
        .execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS recording_effort (date TEXT, source TEXT, seconds DOUBLE);
             INSERT INTO recording_effort VALUES {};",
            values.join(",")
        ))
        .expect("seed effort");
}

#[derive(Debug)]
struct Row {
    first_detection: String,
    last_detection: String,
    arrival: Option<String>,
    departure: Option<String>,
    effort_days_before: Option<i64>,
    gap_before: Option<bool>,
    effort_days_after: Option<i64>,
    gap_after: Option<bool>,
}

fn row(db: &AnalyticsDb, species: &str) -> Row {
    let params = PhenologyParams {
        species: Some(species.into()),
        year_start: Some(2026),
        year_end: Some(2026),
        min_detections: 1,
        ..PhenologyParams::default()
    };
    let sql = timing::phenology_timing_sql(&params);
    let mut stmt = db.conn().prepare(&sql).expect("prepare");
    let rows: Vec<Row> = stmt
        .query_map([], |r| {
            Ok(Row {
                first_detection: r.get("first_detection")?,
                last_detection: r.get("last_detection")?,
                // Read leniently so that a query without these columns still
                // shows the row it returned; the first three tests demand
                // `Some` of each, so a missing column fails there.
                arrival: r.get("arrival_date").ok().flatten(),
                departure: r.get("departure_date").ok().flatten(),
                effort_days_before: r.get("effort_days_before_arrival").ok().flatten(),
                gap_before: r.get("effort_gap_before_arrival").ok().flatten(),
                effort_days_after: r.get("effort_days_after_departure").ok().flatten(),
                gap_after: r.get("effort_gap_after_departure").ok().flatten(),
            })
        })
        .expect("query")
        .map(|r| r.expect("row"))
        .collect();
    assert_eq!(rows.len(), 1, "one species, one year: {rows:?}");
    rows.into_iter().next().expect("one row")
}

fn open() -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    (db, dir)
}

/// The gate. One stray detection on 2 January, then a Chiffchaff every day
/// from 1 April. The station listened all year except 1 to 31 March.
///
/// The arrival is 1 April, not 2 January; and because the station had been off
/// for the fortnight before it, the arrival is flagged: the bird may have come
/// back during the outage.
#[test]
fn a_stray_detection_is_not_an_arrival_and_an_outage_is_flagged() {
    let (db, _dir) = open();
    let mut dates = vec!["2026-01-02".to_owned()];
    dates.extend(days(&db, "2026-04-01", 20));
    detect(&db, "Chiffchaff", &dates);
    let mut effort = days(&db, "2026-01-01", 59); // 1 Jan to 28 Feb
    effort.extend(days(&db, "2026-04-01", 61)); // 1 Apr to 31 May
    listen(&db, &effort);

    let r = row(&db, "Chiffchaff");
    // The raw extremes are still what they were.
    assert_eq!(r.first_detection, "2026-01-02", "{r:?}");
    assert_eq!(
        r.arrival.as_deref(),
        Some("2026-04-01"),
        "one detection on 2 January is not an arrival: {r:?}"
    );
    assert_eq!(r.effort_days_before, Some(0), "{r:?}");
    assert_eq!(
        r.gap_before,
        Some(true),
        "nothing was listened to in the fortnight before 1 April: {r:?}"
    );
    assert_eq!(r.departure.as_deref(), Some("2026-04-20"), "{r:?}");
    assert_eq!(r.effort_days_after, Some(14), "{r:?}");
    assert_eq!(r.gap_after, Some(false), "{r:?}");
}

/// The counterpart: a clean run, with the station listening throughout, gives
/// the arrival and departure that `MIN` and `MAX` gave, and no flag.
#[test]
fn a_clean_run_with_full_effort_keeps_the_first_and_last_day() {
    let (db, _dir) = open();
    detect(&db, "Chiffchaff", &days(&db, "2026-04-01", 20));
    listen(&db, &days(&db, "2026-03-01", 92)); // 1 Mar to 31 May

    let r = row(&db, "Chiffchaff");
    assert_eq!(
        r.arrival.as_deref(),
        Some(r.first_detection.as_str()),
        "{r:?}"
    );
    assert_eq!(r.arrival.as_deref(), Some("2026-04-01"), "{r:?}");
    assert_eq!(
        r.departure.as_deref(),
        Some(r.last_detection.as_str()),
        "{r:?}"
    );
    assert_eq!(r.effort_days_before, Some(14), "{r:?}");
    assert_eq!(r.gap_before, Some(false), "{r:?}");
    assert_eq!(r.gap_after, Some(false), "{r:?}");
}

/// A one-day outage inside the fortnight is still a gap — the bird could have
/// arrived that day — and the count says how much of the fortnight was heard.
#[test]
fn a_partial_fortnight_is_a_gap_with_its_count() {
    let (db, _dir) = open();
    detect(&db, "Chiffchaff", &days(&db, "2026-04-01", 20));
    let effort: Vec<String> = days(&db, "2026-03-01", 92)
        .into_iter()
        .filter(|d| d != "2026-03-25")
        .collect();
    listen(&db, &effort);

    let r = row(&db, "Chiffchaff");
    assert_eq!(r.arrival.as_deref(), Some("2026-04-01"), "{r:?}");
    assert_eq!(r.effort_days_before, Some(13), "{r:?}");
    assert_eq!(r.gap_before, Some(true), "{r:?}");
}

/// Detections too sparse to make a run give no arrival at all, rather than a
/// stray promoted to one; the raw first detection is still there.
#[test]
fn detections_that_never_make_a_run_have_no_arrival() {
    let (db, _dir) = open();
    let dates = vec![
        "2026-01-02".to_owned(),
        "2026-02-10".to_owned(),
        "2026-02-20".to_owned(),
    ];
    detect(&db, "Waxwing", &dates);
    listen(&db, &days(&db, "2026-01-01", 90));

    let r = row(&db, "Waxwing");
    assert_eq!(r.first_detection, "2026-01-02", "{r:?}");
    assert_eq!(r.arrival, None, "{r:?}");
    assert_eq!(r.departure, None, "{r:?}");
    assert_eq!(r.gap_before, None, "no arrival, so nothing to flag: {r:?}");
}
