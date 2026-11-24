//! A day the station did not listen to is not a day the birds were quiet.
//!
//! The trend queries build a zero-filled spine of days, which is right for a
//! day the station listened to and heard nothing, and wrong for a day it was
//! switched off: an outage became a run of zeros, the moving average fell
//! through it, the anomaly detector flagged each day of it `low`, and the
//! zeros stayed in the baseline the next thirty days were judged against.
//!
//! The station's `recording_effort` ledger says which days it listened to.
//! These gates seed that ledger through the same sync the application runs at
//! startup (`AnalyticsDb::sync_recording_effort`), so the table they read is
//! the one production reads, and assert on the numbers the Trends page shows.
//!
//! Each discrimination has its counterpart: a day that *was* listened to and
//! heard nothing is still a `low` anomaly, and a station with no ledger at all
//! is judged as it always was.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_timeseries::executor::TimeSeriesDb;
use birdnet_timeseries::types::params::{AnomalyParams, TrendParams};
use tempfile::TempDir;

const FULL_DAY: f64 = 86_400.0;

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

/// A store with `count(back)` Great Tit detections on each of the days
/// `1..=days` before today.
fn store(days: i64, count: impl Fn(i64) -> u32) -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("ts.duckdb")).expect("open");
    let mut rows = Vec::new();
    for back in 1..=days {
        let date = day(&db, back);
        for k in 0..count(back) {
            let time = format!("{:02}:{:02}:00", 6 + k / 60, k % 60);
            rows.push(format!(
                "('{date}','{time}','Parus major','Great Tit',0.9,epoch(TIMESTAMP '{date} {time}'))"
            ));
        }
    }
    if !rows.is_empty() {
        db.conn()
            .execute_batch(&format!(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc) VALUES {};",
                rows.join(",")
            ))
            .expect("seed detections");
    }
    (db, dir)
}

/// Write `seconds(back)` of effort for each of the days `1..=days` before today
/// (`None`: no row for that day) into a `SQLite` ledger of the station's own
/// schema, and mirror it the way the application does.
fn ledger(db: &AnalyticsDb, days: i64, seconds: impl Fn(i64) -> Option<f64>) -> usize {
    let sqlite = rusqlite::Connection::open_in_memory().expect("sqlite");
    sqlite
        .execute_batch(
            "CREATE TABLE recording_effort (
                date TEXT NOT NULL, source TEXT NOT NULL,
                seconds REAL NOT NULL DEFAULT 0, PRIMARY KEY (date, source));",
        )
        .expect("ledger schema");
    let mut written = 0;
    for back in 1..=days {
        if let Some(s) = seconds(back) {
            sqlite
                .execute(
                    "INSERT INTO recording_effort VALUES (?1, 'mic', ?2)",
                    rusqlite::params![day(db, back), s],
                )
                .expect("ledger row");
            written += 1;
        }
    }
    db.sync_recording_effort(&sqlite).expect("sync effort");
    written
}

/// Alternating 9 and 11 detections: a steady series with a little spread, so
/// a z-score is defined.
const fn steady(back: i64) -> u32 {
    if back % 2 == 0 { 9 } else { 11 }
}

fn ts(db: &AnalyticsDb) -> TimeSeriesDb<'_> {
    TimeSeriesDb::new(db.conn()).expect("executor")
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
    let date = day(db, back);
    rows.iter().find(|r| r.date == date).map_or_else(
        || panic!("no anomaly row for {date}: {rows:?}"),
        |r| r.anomaly_flag.clone(),
    )
}

// ---------------------------------------------------------------------------
// UIC-03: an outage is not a "low" anomaly, and does not poison the baseline
// ---------------------------------------------------------------------------

/// UIC-03. A steady 40-day series with a three-day outage ten to twelve days
/// ago: the station recorded nothing and heard nothing. None of those days is
/// a `low` anomaly, and the day after the outage is `normal`.
#[test]
fn a_three_day_outage_is_not_a_low_anomaly() {
    let outage = 10..=12;
    let (db, _dir) = store(40, |b| if outage.contains(&b) { 0 } else { steady(b) });
    let written = ledger(&db, 40, |b| {
        Some(if outage.contains(&b) { 0.0 } else { FULL_DAY })
    });
    assert_eq!(written, 40, "the fixture wrote its ledger");

    let rows = anomalies(&db);
    for back in outage {
        assert_ne!(
            flag(&db, &rows, back),
            "low",
            "an outage day ({}) was flagged as a low anomaly: {rows:?}",
            day(&db, back)
        );
    }
    assert_eq!(
        flag(&db, &rows, 9),
        "normal",
        "the day after the outage was judged against a baseline holding it: {rows:?}"
    );
}

/// Counterpart: a day the station listened to all day and heard nothing is
/// still a `low` anomaly. The ledger excludes outages; it does not excuse
/// silence.
#[test]
fn a_listened_silent_day_is_still_low() {
    let (db, _dir) = store(40, |b| if b == 10 { 0 } else { steady(b) });
    ledger(&db, 40, |_| Some(FULL_DAY));
    let rows = anomalies(&db);
    assert_eq!(flag(&db, &rows, 10), "low", "{rows:?}");
}

// ---------------------------------------------------------------------------
// RES-14: a week without effort has no average and no anomaly
// ---------------------------------------------------------------------------

/// RES-14. Seven days of zero effort in the middle of a 40-day series: the
/// moving average is NULL on each of them, and none is a `low` anomaly.
#[test]
fn a_week_without_effort_has_no_average_and_no_low_anomaly() {
    let off = 15..=21;
    let (db, _dir) = store(40, |b| if off.contains(&b) { 0 } else { steady(b) });
    ledger(&db, 40, |b| {
        Some(if off.contains(&b) { 0.0 } else { FULL_DAY })
    });

    let trend = ts(&db)
        .moving_average(&TrendParams {
            window_days: 7,
            from_date: Some("CURRENT_DATE - INTERVAL 40 DAYS".into()),
            to_date: None,
            species: None,
        })
        .expect("moving_average");
    let avg = |back: i64| {
        let date = day(&db, back);
        trend
            .iter()
            .find(|r| r.date == date)
            .unwrap_or_else(|| panic!("no trend row for {date}: {trend:?}"))
            .moving_avg_detections
    };
    for back in off.clone() {
        assert_eq!(
            avg(back),
            None,
            "a day the station did not listen to ({}) has a moving average: {trend:?}",
            day(&db, back)
        );
    }
    // Counterpart: a week of listened days on either side still averages.
    assert_eq!(avg(22).map(f64::round), Some(10.0), "{trend:?}");
    assert_eq!(avg(1).map(f64::round), Some(10.0), "{trend:?}");

    let rows = anomalies(&db);
    for back in off {
        assert_ne!(
            flag(&db, &rows, back),
            "low",
            "a day without effort ({}) was flagged as a low anomaly: {rows:?}",
            day(&db, back)
        );
    }
}

/// Counterpart to both gates: a station with no ledger table, and one whose
/// ledger is empty, judges an outage as it always did — a `low` day with an
/// average through it — rather than turning every day into NULL.
#[test]
fn without_a_ledger_every_day_is_judged_as_before() {
    for with_empty_table in [false, true] {
        let (db, _dir) = store(40, |b| if b == 10 { 0 } else { steady(b) });
        if with_empty_table {
            assert_eq!(ledger(&db, 40, |_| None), 0);
        }
        let rows = anomalies(&db);
        assert_eq!(
            flag(&db, &rows, 10),
            "low",
            "empty table: {with_empty_table}: {rows:?}"
        );
        assert!(
            rows.iter().all(|r| r.listened && r.effort_hours.is_none()),
            "{rows:?}"
        );
        let trend = ts(&db)
            .moving_average(&TrendParams {
                window_days: 7,
                from_date: Some("CURRENT_DATE - INTERVAL 40 DAYS".into()),
                to_date: None,
                species: None,
            })
            .expect("moving_average");
        let at10 = trend.iter().find(|r| r.date == day(&db, 10)).expect("row");
        assert!(at10.moving_avg_detections.is_some(), "{trend:?}");
    }
}

/// A date before the ledger's first row is unknown, not unlistened: history
/// that predates the ledger is judged as it always was, and the days the
/// ledger covers are judged by it.
#[test]
fn days_before_the_ledger_are_unknown_not_unlistened() {
    let (db, _dir) = store(40, |b| if b == 30 || b == 5 { 0 } else { steady(b) });
    // Ledger from 20 days ago; day 5 has a zero-second row.
    ledger(&db, 20, |b| Some(if b == 5 { 0.0 } else { FULL_DAY }));
    let rows = anomalies(&db);
    assert_eq!(flag(&db, &rows, 30), "low", "{rows:?}");
    assert_eq!(flag(&db, &rows, 5), "unlistened", "{rows:?}");
    let at = |back: i64| rows.iter().find(|r| r.date == day(&db, back)).expect("row");
    assert_eq!(at(30).effort_hours, None);
    assert_eq!(at(5).effort_hours, Some(0.0));
    assert!(at(5).z_score.is_none(), "an unlistened day is not scored");
}

/// A day missing from the ledger after its first row had no effort credited:
/// it is a day the station did not listen to.
#[test]
fn a_day_missing_from_the_ledger_is_not_listened() {
    let (db, _dir) = store(40, |b| if b == 10 { 0 } else { steady(b) });
    ledger(&db, 40, |b| (b != 10).then_some(FULL_DAY));
    let rows = anomalies(&db);
    assert_eq!(flag(&db, &rows, 10), "unlistened", "{rows:?}");
}

/// A station on a recording schedule (`solar`, `fixed:`, night inhibit) that
/// records ten hours a day is judged against its own typical day, not a
/// 24-hour one: its ordinary days are listened to and judged, and only its
/// outage is left out.
#[test]
fn a_scheduled_station_is_judged_against_its_own_day() {
    const WINDOW: f64 = 10.0 * 3600.0;
    let off = 14..=16;
    let (db, _dir) = store(40, |b| if off.contains(&b) { 0 } else { steady(b) });
    ledger(&db, 40, |b| {
        Some(if off.contains(&b) { 0.0 } else { WINDOW })
    });
    let rows = anomalies(&db);
    for back in [5, 10, 20, 30] {
        assert_ne!(
            flag(&db, &rows, back),
            "unlistened",
            "a ten-hour day on a ten-hour schedule is a day listened to: {rows:?}"
        );
    }
    for back in off {
        assert_eq!(flag(&db, &rows, back), "unlistened", "{rows:?}");
    }
}

/// The floor is a line: a day just above it is judged and labelled partial,
/// one just below it is not judged; a full day is not partial.
#[test]
fn the_effort_floor_divides_judged_from_unjudged() {
    use birdnet_timeseries::queries::trend::{EFFORT_FLOOR_SHARE, FULL_DAY_SHARE};
    // Every other day is a whole 24 hours, so that is the typical day.
    const EFFORT_FLOOR_SECS: f64 = EFFORT_FLOOR_SHARE * FULL_DAY;
    const FULL_DAY_SECS: f64 = FULL_DAY_SHARE * FULL_DAY;
    let (db, _dir) = store(40, |b| match b {
        10 | 11 => 0,
        _ => steady(b),
    });
    ledger(&db, 40, |b| {
        Some(match b {
            10 => EFFORT_FLOOR_SECS,
            11 => EFFORT_FLOOR_SECS - 1.0,
            12 => FULL_DAY_SECS,
            _ => FULL_DAY,
        })
    });
    let rows = anomalies(&db);
    let at = |back: i64| rows.iter().find(|r| r.date == day(&db, back)).expect("row");
    assert_eq!(at(10).anomaly_flag, "low", "{rows:?}");
    assert!(at(10).listened && at(10).partial_effort, "{:?}", at(10));
    assert_eq!(at(10).effort_hours, Some(12.0));
    assert_eq!(at(11).anomaly_flag, "unlistened", "{rows:?}");
    assert!(!at(11).listened && !at(11).partial_effort, "{:?}", at(11));
    assert!(at(12).listened && !at(12).partial_effort, "{:?}", at(12));
}

/// Moving-average rows carry the same effort fields, and the average returns
/// once a full window of listened days is behind it again.
#[test]
fn trend_rows_carry_effort() {
    let off = 15..=21;
    let (db, _dir) = store(40, |b| if off.contains(&b) { 0 } else { steady(b) });
    ledger(&db, 40, |b| {
        Some(if off.contains(&b) { 0.0 } else { FULL_DAY })
    });
    let trend = ts(&db)
        .moving_average(&TrendParams {
            window_days: 7,
            from_date: Some("CURRENT_DATE - INTERVAL 40 DAYS".into()),
            to_date: None,
            species: None,
        })
        .expect("moving_average");
    let at = |back: i64| {
        trend
            .iter()
            .find(|r| r.date == day(&db, back))
            .expect("row")
    };
    assert!(
        !at(15).listened && at(15).effort_hours == Some(0.0),
        "{:?}",
        at(15)
    );
    assert!(
        at(14).listened && at(14).effort_hours == Some(24.0),
        "{:?}",
        at(14)
    );
    // Day 9's window (15..=9 is 9..=15) still holds an unlistened day; day 8's
    // (8..=14) does not.
    assert_eq!(at(9).moving_avg_detections, None, "{trend:?}");
    assert!(at(8).moving_avg_detections.is_some(), "{trend:?}");
}

/// Year over year: the same birds per listened day in both years compare as
/// equal, though the prior year lost three days of its week to an outage.
///
/// The raw `yoy_delta` still reads the outage as a difference; the
/// per-listened-day comparison does not.
#[test]
fn year_over_year_compares_per_listened_day() {
    use birdnet_timeseries::types::params::WeeklyParams;
    // 10 a day for 380 days, except three days of one week 52 weeks back
    // (364 days + 8..=10 days ago), when the station was off.
    let prior_outage = 372..=374;
    let (db, _dir) = store(380, |b| if prior_outage.contains(&b) { 0 } else { 10 });
    ledger(&db, 380, |b| {
        Some(if prior_outage.contains(&b) {
            0.0
        } else {
            FULL_DAY
        })
    });
    let rows = ts(&db)
        .year_over_year(&WeeklyParams { lookback_weeks: 4 })
        .expect("year_over_year");
    assert!(!rows.is_empty());
    let mut saw_outage_week = false;
    for r in &rows {
        assert_eq!(r.per_listened_day_delta, Some(0.0), "{r:?}");
        assert_eq!(r.current_per_listened_day, Some(10.0), "{r:?}");
        if r.prior_listened_days
            .zip(Some(r.current_listened_days))
            .is_some_and(|(p, c)| p < c)
        {
            saw_outage_week = true;
            assert!(
                r.yoy_delta.is_some_and(|d| d > 0),
                "raw still differs: {r:?}"
            );
        }
    }
    assert!(
        saw_outage_week,
        "the fixture put the outage in a compared week: {rows:?}"
    );
}
