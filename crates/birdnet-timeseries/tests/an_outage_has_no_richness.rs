//! A day the station did not listen to has no species count, no diversity,
//! and is not a quiet day (RES-14).
//!
//! Richness and Shannon diversity were grouped from the detections alone, and
//! `QuietDays` zero-filled a spine of days from the station's first
//! detection. So a day the station recorded a few minutes of reported one or
//! two species as that day's richness, and a day it recorded nothing was a
//! "quiet day" with zero detections — an outage read as a collapse.
//!
//! These gates seed the `recording_effort` ledger through the sync the
//! application runs (`AnalyticsDb::sync_recording_effort`), as
//! `an_outage_is_not_a_quiet_day.rs` does for the trend queries, and assert
//! on the rows the `/api/v2/timeseries/diversity` route serialises. Each has
//! its counterpart: a day that *was* listened to and heard nothing is zero
//! species and a quiet day, and a station with no ledger is judged as before.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_timeseries::executor::TimeSeriesDb;
use birdnet_timeseries::types::params::DiversityParams;
use tempfile::TempDir;

const FULL_DAY: f64 = 86_400.0;

/// The outage: seven days with no listening worth the name in the middle of
/// a 30-day series. On three of them the station was up for a quarter of an
/// hour and heard a bird or two.
const OFF: std::ops::RangeInclusive<i64> = 12..=18;

/// Up for fifteen minutes: well under half its usual day.
const fn outage_seconds(back: i64) -> f64 {
    if back % 2 == 0 { 900.0 } else { 0.0 }
}

/// The listened day that heard nothing.
const SILENT: i64 = 5;

fn today_days(db: &AnalyticsDb) -> i64 {
    let t: String = db
        .conn()
        .query_row("SELECT CAST(CURRENT_DATE AS VARCHAR)", [], |r| r.get(0))
        .expect("current date");
    let p: Vec<u32> = t.split('-').map(|x| x.parse().unwrap()).collect();
    birdnet_core::civil::days_from_civil(p[0], p[1], p[2])
}

fn day(db: &AnalyticsDb, back: i64) -> String {
    let (y, m, d) = birdnet_core::civil::civil_from_days(today_days(db) - back);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Detections on day `back`: two species on an ordinary day, one Great Tit
/// twice on the outage days the station was briefly up, none on `SILENT` or
/// on the outage days it was off.
fn detections_on(back: i64) -> Vec<(&'static str, &'static str, u32)> {
    if back == SILENT {
        Vec::new()
    } else if OFF.contains(&back) {
        if outage_seconds(back) > 0.0 {
            vec![("Parus major", "Great Tit", 2)]
        } else {
            Vec::new()
        }
    } else {
        vec![
            ("Parus major", "Great Tit", 9),
            ("Turdus merula", "Eurasian Blackbird", 4),
        ]
    }
}

fn store(days: i64) -> (AnalyticsDb, TempDir) {
    let dir = TempDir::new().expect("temp dir");
    let db = AnalyticsDb::open(&dir.path().join("ts.duckdb")).expect("open");
    let mut rows = Vec::new();
    for back in 1..=days {
        let date = day(&db, back);
        for (sci, com, n) in detections_on(back) {
            for k in 0..n {
                let time = format!("{:02}:{:02}:00", 6 + k / 60, k % 60);
                rows.push(format!(
                    "('{date}','{time}','{sci}','{com}',0.9,epoch(TIMESTAMP '{date} {time}'))"
                ));
            }
        }
    }
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc) VALUES {};",
            rows.join(",")
        ))
        .expect("seed detections");
    (db, dir)
}

fn ledger(db: &AnalyticsDb, days: i64, seconds: impl Fn(i64) -> f64) {
    let sqlite = rusqlite::Connection::open_in_memory().expect("sqlite");
    sqlite
        .execute_batch(
            "CREATE TABLE recording_effort (
                date TEXT NOT NULL, source TEXT NOT NULL,
                seconds REAL NOT NULL DEFAULT 0, PRIMARY KEY (date, source));",
        )
        .expect("ledger schema");
    for back in 1..=days {
        sqlite
            .execute(
                "INSERT INTO recording_effort VALUES (?1, 'mic', ?2)",
                rusqlite::params![day(db, back), seconds(back)],
            )
            .expect("ledger row");
    }
    db.sync_recording_effort(&sqlite).expect("sync effort");
}

fn station_with_outage() -> (AnalyticsDb, TempDir) {
    let (db, dir) = store(30);
    ledger(&db, 30, |b| {
        if OFF.contains(&b) {
            outage_seconds(b)
        } else {
            FULL_DAY
        }
    });
    (db, dir)
}

/// The diversity rows as the API serves them: `date` → the row's JSON.
fn diversity(db: &AnalyticsDb) -> Vec<serde_json::Value> {
    TimeSeriesDb::new(db.conn())
        .expect("executor")
        .daily_richness(&DiversityParams {
            lookback_days: 60,
            include_shannon: true,
        })
        .expect("daily_richness")
        .iter()
        .map(|r| serde_json::to_value(r).expect("row serialises"))
        .collect()
}

fn row_for<'a>(rows: &'a [serde_json::Value], date: &str) -> Option<&'a serde_json::Value> {
    rows.iter().find(|r| r["date"] == date)
}

fn quiet_days(db: &AnalyticsDb) -> Vec<String> {
    TimeSeriesDb::new(db.conn())
        .expect("executor")
        .quiet_days(5, 60)
        .expect("quiet_days")
        .into_iter()
        .map(|r| r.window_start)
        .collect()
}

/// RES-14. No day of the outage reports a species count or a diversity: it
/// is either absent or carries `null` for both.
#[test]
fn a_week_without_effort_has_no_richness_and_no_diversity() {
    let (db, _dir) = station_with_outage();
    let rows = diversity(&db);
    for back in OFF {
        let date = day(&db, back);
        if let Some(r) = row_for(&rows, &date) {
            assert!(
                r["species_richness"].is_null(),
                "a day the station did not listen to ({date}) has a species richness: {r}"
            );
            assert!(
                r["shannon_h"].is_null(),
                "a day the station did not listen to ({date}) has a Shannon diversity: {r}"
            );
        }
    }
}

/// Counterpart: a day the station listened to all day and heard nothing is
/// zero species, a value and not a gap; and an ordinary day keeps its count.
#[test]
fn a_listened_silent_day_is_zero_species() {
    let (db, _dir) = station_with_outage();
    let rows = diversity(&db);
    let silent = day(&db, SILENT);
    let r = row_for(&rows, &silent)
        .unwrap_or_else(|| panic!("no diversity row for the silent listened day {silent}"));
    assert_eq!(r["species_richness"], 0, "{r}");
    let ordinary = day(&db, 3);
    let r = row_for(&rows, &ordinary).expect("an ordinary day's row");
    assert_eq!(r["species_richness"], 2, "{r}");
    assert!(r["shannon_h"].as_f64().is_some_and(|h| h > 0.0), "{r}");
}

/// RES-14. No day of the outage is a quiet day: the station was not there to
/// hear whether it was quiet.
#[test]
fn a_week_without_effort_is_not_a_run_of_quiet_days() {
    let (db, _dir) = station_with_outage();
    let quiet = quiet_days(&db);
    for back in OFF {
        let date = day(&db, back);
        assert!(
            !quiet.contains(&date),
            "a day the station did not listen to ({date}) was reported as a quiet day: {quiet:?}"
        );
    }
}

/// Counterpart: a day the station listened to and heard nothing is a quiet
/// day, and an ordinary day is not.
#[test]
fn a_listened_silent_day_is_a_quiet_day() {
    let (db, _dir) = station_with_outage();
    let quiet = quiet_days(&db);
    assert!(quiet.contains(&day(&db, SILENT)), "{quiet:?}");
    assert!(!quiet.contains(&day(&db, 3)), "{quiet:?}");
}

/// Counterpart: a station with no ledger is judged as it always was — its
/// days without detections are quiet days, and a day with detections has its
/// species count.
#[test]
fn a_station_without_a_ledger_is_judged_as_before() {
    let (db, _dir) = store(30);
    let quiet = quiet_days(&db);
    assert!(quiet.contains(&day(&db, 13)), "{quiet:?}");
    let rows = diversity(&db);
    let r = row_for(&rows, &day(&db, 12)).expect("a row for a day with detections");
    assert_eq!(r["species_richness"], 1, "{r}");
}
