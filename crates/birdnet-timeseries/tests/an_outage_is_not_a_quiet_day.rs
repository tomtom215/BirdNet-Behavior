//! A day the station was not listening is not a day it heard nothing.
//!
//! The recording-effort ledger (`recording_effort`, credited per local date and
//! source, and copied into the analytics store by
//! `AnalyticsDb::sync_recording_effort`) is the station's account of when it was
//! listening. Before these gates, nothing in this crate read it: a week of
//! downtime was a week of zeros, which dragged the moving average down, was
//! flagged `low` by the anomaly detector and then depressed the baseline the next
//! month was judged against, read as a year-over-year decline, and was listed as
//! a quiet day.
//!
//! Each gate has its counterpart — the same detections with the ledger saying the
//! station listened throughout — so a query that simply stopped reporting cannot
//! pass for an effort-aware one.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_timeseries::executor::TimeSeriesDb;
use birdnet_timeseries::types::params::{AnomalyParams, TrendParams, WeeklyParams};
use tempfile::TempDir;

/// A full day of listening from one source.
const FULL_DAY: f64 = 86_400.0;

fn open() -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("ts.duckdb")).expect("open");
    (db, dir)
}

/// Days since the epoch of `CURRENT_DATE`, as the engine under test sees it.
fn today_days(db: &AnalyticsDb) -> i64 {
    let t: String = db
        .conn()
        .query_row("SELECT CAST(CURRENT_DATE AS VARCHAR)", [], |r| r.get(0))
        .expect("current date");
    let p: Vec<u32> = t.split('-').map(|x| x.parse().unwrap()).collect();
    birdnet_core::civil::days_from_civil(p[0], p[1], p[2])
}

/// The date `back` days before today, `YYYY-MM-DD`.
fn day(db: &AnalyticsDb, back: i64) -> String {
    let (y, m, d) = birdnet_core::civil::civil_from_days(today_days(db) - back);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `n` Great Tit detections on the day `back` days ago, a minute apart from 06:00.
fn detections(db: &AnalyticsDb, back: i64, n: u32) {
    if n == 0 {
        return;
    }
    let date = day(db, back);
    let values: Vec<String> = (0..n)
        .map(|k| {
            let t = format!("{:02}:{:02}:00", 6 + k / 60, k % 60);
            format!("('{date}','{t}','Parus major','Great Tit',0.9,epoch(TIMESTAMP '{date} {t}'))")
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

/// Write `(back, seconds)` effort rows into a station `SQLite` ledger and copy
/// it into the analytics store the way the station does at startup.
fn effort(db: &AnalyticsDb, rows: &[(i64, f64)]) {
    let sqlite = rusqlite::Connection::open_in_memory().expect("sqlite");
    sqlite
        .execute_batch(
            "CREATE TABLE recording_effort (
                date TEXT NOT NULL, source TEXT NOT NULL, seconds REAL NOT NULL,
                PRIMARY KEY (date, source));",
        )
        .expect("ledger");
    for (back, seconds) in rows {
        sqlite
            .execute(
                "INSERT INTO recording_effort VALUES (?1, 'mic', ?2)",
                rusqlite::params![day(db, *back), seconds],
            )
            .expect("effort row");
    }
    db.sync_recording_effort(&sqlite).expect("mirror effort");
}

fn ts(db: &AnalyticsDb) -> TimeSeriesDb<'_> {
    TimeSeriesDb::new(db.conn()).expect("executor")
}

// ---------------------------------------------------------------------------
// Moving average (RES-14)
// ---------------------------------------------------------------------------

/// Thirty days of ten detections, with a week in the middle (10–16 days ago)
/// when the station was off: no detections and no effort.
fn steady_month_with_a_dark_week(db: &AnalyticsDb, ledger_says_off: bool) {
    let dark = 10..=16;
    for back in 1..=30 {
        if !dark.contains(&back) {
            detections(db, back, 10);
        }
    }
    let rows: Vec<(i64, f64)> = (1..=30)
        .filter(|b| !(ledger_says_off && dark.contains(b)))
        .map(|b| (b, FULL_DAY))
        .collect();
    effort(db, &rows);
}

fn trend(db: &AnalyticsDb) -> Vec<birdnet_timeseries::types::results::TrendRow> {
    ts(db)
        .moving_average(&TrendParams {
            window_days: 7,
            from_date: Some("CURRENT_DATE - INTERVAL 40 DAYS".into()),
            to_date: None,
            species: None,
        })
        .expect("moving_average")
}

/// No average is computed over, or across, a day the station was not listening:
/// the dark week and the six days whose windows reach into it are `NULL`, and
/// the averages on either side are the steady ten.
#[test]
fn a_dark_week_has_no_moving_average() {
    let (db, _dir) = open();
    steady_month_with_a_dark_week(&db, true);
    let trend = trend(&db);
    let avg = |back: i64| {
        trend
            .iter()
            .find(|r| r.date == day(&db, back))
            .unwrap_or_else(|| panic!("no row {back} days back: {trend:?}"))
            .moving_avg_detections
    };
    for back in 4..=16 {
        assert_eq!(
            avg(back),
            None,
            "{back} days back: a window touching the dark week has no average: {trend:?}"
        );
    }
    for back in (1..=3).chain(17..=24) {
        assert_eq!(avg(back), Some(10.0), "{back} days back: {trend:?}");
    }
}

/// Counterpart: the same detections on a station whose ledger says it listened
/// all month — the dark week is a genuine silence, and is averaged as zeros.
#[test]
fn a_listened_silent_week_is_still_averaged_as_zeros() {
    let (db, _dir) = open();
    steady_month_with_a_dark_week(&db, false);
    let trend = trend(&db);
    let yesterday = trend.iter().find(|r| r.date == day(&db, 1)).expect("row");
    assert_eq!(yesterday.moving_avg_detections, Some(10.0), "{trend:?}");
    let in_the_week = trend.iter().find(|r| r.date == day(&db, 10)).expect("row");
    assert_eq!(in_the_week.moving_avg_detections, Some(0.0), "{trend:?}");
    assert!(
        trend
            .iter()
            .all(|r| r.date < day(&db, 24) || r.moving_avg_detections.is_some()),
        "every complete window is averaged: {trend:?}"
    );
}

// ---------------------------------------------------------------------------
// Anomalies (UIC-03)
// ---------------------------------------------------------------------------

/// Forty days alternating 9 and 11, a three-day outage 5–7 days ago (no
/// detections, and no effort unless `ledger_says_off` is false), then a genuinely
/// thin day of 3 detections 3 days ago.
fn steady_with_an_outage(db: &AnalyticsDb, ledger_says_off: bool) {
    let outage = 5..=7;
    for back in 1..=40 {
        let n = match back {
            b if outage.contains(&b) => 0,
            3 => 3,
            b if b % 2 == 0 => 9,
            _ => 11,
        };
        detections(db, back, n);
    }
    let rows: Vec<(i64, f64)> = (1..=40)
        .filter(|b| !(ledger_says_off && outage.contains(b)))
        .map(|b| (b, FULL_DAY))
        .collect();
    effort(db, &rows);
}

fn anomalies(db: &AnalyticsDb) -> Vec<birdnet_timeseries::types::results::AnomalyRow> {
    ts(db)
        .anomalies(&AnomalyParams {
            z_threshold: 2.0,
            window_days: 30,
            lookback_days: 60,
        })
        .expect("anomalies")
}

fn flag(
    db: &AnalyticsDb,
    rows: &[birdnet_timeseries::types::results::AnomalyRow],
    back: i64,
) -> String {
    rows.iter()
        .find(|r| r.date == day(db, back))
        .unwrap_or_else(|| panic!("no row {back} days back: {rows:?}"))
        .anomaly_flag
        .clone()
}

/// An outage is neither flagged nor part of the baseline: the outage days are
/// not `low`, the day after is `normal`, and the thin day after that is judged
/// against the steady days alone — and so is `low`, where three zeros in its
/// baseline had widened the deviation enough to wave it through.
#[test]
fn an_outage_is_not_an_anomaly_and_not_a_baseline() {
    let (db, _dir) = open();
    steady_with_an_outage(&db, true);
    let rows = anomalies(&db);
    for back in 5..=7 {
        assert_ne!(
            flag(&db, &rows, back),
            "low",
            "{back} days back was an outage, not a quiet day: {rows:?}"
        );
    }
    assert_eq!(flag(&db, &rows, 4), "normal", "the day after: {rows:?}");
    assert_eq!(
        flag(&db, &rows, 3),
        "low",
        "3 against a baseline of 9s and 11s, without the outage in it: {rows:?}"
    );
}

/// Counterpart: the same detections with the ledger saying the station listened
/// through those three days. Then they were genuinely silent, and are flagged.
#[test]
fn a_listened_silent_day_is_still_low() {
    let (db, _dir) = open();
    steady_with_an_outage(&db, false);
    let rows = anomalies(&db);
    for back in 5..=7 {
        assert_eq!(flag(&db, &rows, back), "low", "{back} days back: {rows:?}");
    }
}

/// A day the station listened for a fraction of its usual time is labelled as
/// such rather than judged, and the outage days carry a label of their own.
#[test]
fn partial_and_absent_effort_are_labelled() {
    let (db, _dir) = open();
    for back in 1..=40 {
        let n = match back {
            4 => 0,
            b if b % 2 == 0 => 9,
            _ => 11,
        };
        detections(&db, back, n);
    }
    // Two hours of a usual full day, 2 days back; nothing at all, 4 days back.
    let rows: Vec<(i64, f64)> = (1..=40)
        .filter(|b| *b != 4)
        .map(|b| (b, if b == 2 { 7_200.0 } else { FULL_DAY }))
        .collect();
    effort(&db, &rows);
    let out = anomalies(&db);
    assert_eq!(flag(&db, &out, 2), "partial", "{out:?}");
    assert_eq!(flag(&db, &out, 4), "not_listening", "{out:?}");
    assert_eq!(flag(&db, &out, 1), "normal", "{out:?}");
}

// ---------------------------------------------------------------------------
// Quiet days
// ---------------------------------------------------------------------------

/// An outage is not a quiet day; a day the station listened through and heard
/// little is.
#[test]
fn quiet_days_are_days_the_station_listened() {
    let (db, _dir) = open();
    for back in 1..=10 {
        let n = match back {
            3 => 0, // outage
            6 => 1, // listened, quiet
            _ => 20,
        };
        detections(&db, back, n);
    }
    let rows: Vec<(i64, f64)> = (1..=10)
        .filter(|b| *b != 3)
        .map(|b| (b, FULL_DAY))
        .collect();
    effort(&db, &rows);
    let quiet: Vec<String> = ts(&db)
        .quiet_days(5, 30)
        .expect("quiet_days")
        .into_iter()
        .map(|r| r.window_start)
        .collect();
    assert_eq!(quiet, [day(&db, 6)], "the outage is not quiet: {quiet:?}");
}

// ---------------------------------------------------------------------------
// Year over year
// ---------------------------------------------------------------------------

/// One detection a day for 800 days, except a three-day outage in this year's
/// week three weeks ago (no detections, and no effort when `ledger_says_off`).
/// The ledger covers the last 60 days only, as a station that upgraded recently.
fn two_years_with_an_outage(db: &AnalyticsDb, ledger_says_off: bool) -> String {
    let monday: String = db
        .conn()
        .query_row(
            "SELECT CAST((date_trunc('week', CURRENT_DATE) - INTERVAL 3 WEEKS)::DATE AS VARCHAR)",
            [],
            |r| r.get(0),
        )
        .expect("monday");
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
             SELECT strftime(d, '%Y-%m-%d'), '06:00:00', 'Turdus merula', 'Eurasian Blackbird',
                    0.9, epoch(d + INTERVAL 6 HOUR)
               FROM range(CAST(CURRENT_DATE - INTERVAL 800 DAY AS TIMESTAMP),
                          CAST(CURRENT_DATE + INTERVAL 1 DAY AS TIMESTAMP),
                          INTERVAL 1 DAY) t(d)
              WHERE d NOT BETWEEN TIMESTAMP '{monday}' AND TIMESTAMP '{monday}' + INTERVAL 2 DAY"
        ))
        .expect("seed");
    let off: Vec<String> = (0..3)
        .map(|k| {
            db.conn()
                .query_row(
                    &format!("SELECT CAST(DATE '{monday}' + INTERVAL {k} DAY AS DATE)::VARCHAR"),
                    [],
                    |r| r.get(0),
                )
                .expect("date")
        })
        .collect();
    let rows: Vec<(i64, f64)> = (0..=60)
        .filter(|b| !(ledger_says_off && off.contains(&day(db, *b))))
        .map(|b| (b, FULL_DAY))
        .collect();
    effort(db, &rows);
    monday
}

/// A week with three days of outage is compared with last year over the four
/// days it listened, not read as a decline of three.
#[test]
fn an_outage_week_is_compared_over_the_days_listened() {
    let (db, _dir) = open();
    let monday = two_years_with_an_outage(&db, true);
    let rows = ts(&db)
        .year_over_year(&WeeklyParams { lookback_weeks: 8 })
        .expect("yoy");
    let week = rows
        .iter()
        .find(|r| r.week_start == monday)
        .unwrap_or_else(|| panic!("the outage week is missing: {rows:?}"));
    assert_eq!(week.yoy_delta, Some(0), "{week:?}");
    assert_eq!(week.current_year_count, 4, "{week:?}");
    assert_eq!(week.prior_year_count, Some(4), "{week:?}");
    assert_eq!(week.compared_days, 4, "{week:?}");
}

/// Counterpart: if the ledger says the station listened through those three
/// days, the silence is real and the week is down three on last year.
#[test]
fn a_listened_silent_stretch_is_still_a_decline() {
    let (db, _dir) = open();
    let monday = two_years_with_an_outage(&db, false);
    let rows = ts(&db)
        .year_over_year(&WeeklyParams { lookback_weeks: 8 })
        .expect("yoy");
    let week = rows.iter().find(|r| r.week_start == monday).expect("week");
    assert_eq!(week.yoy_delta, Some(-3), "{week:?}");
    assert_eq!(week.compared_days, 7, "{week:?}");
    for r in rows.iter().filter(|r| r.week_start != monday) {
        assert_eq!(r.yoy_delta, Some(0), "{r:?}");
    }
}
