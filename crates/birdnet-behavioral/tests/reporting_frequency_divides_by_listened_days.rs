//! CHT-16 / G-01: reporting frequency is days heard over days *listened*, per
//! part of the year, from the listening ledger.
//!
//! Every date is fixed. The ledger's typical day is 86 400 s (the median of
//! its days), so a listened day needs at least 43 200 s summed over sources.
//!
//! * 1–7 May 2025: listened all seven days; a Song Thrush on 2, 4 and 6 May
//!   (twice on the 2nd: a day, not a detection, is the unit), and once on
//!   1 May in a detection review rejected.
//! * 8–14 May 2025: no ledger row at all; a Thrush on the 9th.
//! * 15–21 May 2025: the 15th has 30 000 s (under half a typical day) and a
//!   Thrush; the 16th to the 21st are listened on two microphones of 43 200 s
//!   each, and no Thrush.
//! * 1–7 June, two years: 1–3 June 2024 and 1–4 June 2025 listened; a Thrush
//!   on 2 June 2024 and 3 June 2025, pooled into one part of the year.
//! * 15 January 2024, before the ledger's first row: a Thrush.

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_behavioral::reporting::{ReportingFrequency, period_of};
use tempfile::TempDir;

/// The listened-day floor the web passes (the trend queries' `EFFORT_FLOOR_SHARE`).
const FLOOR: f64 = 0.5;

fn detection(db: &AnalyticsDb, day: &str, time: &str, com: &str, verdict: Option<&str>) {
    let verdict = verdict.map_or_else(|| "NULL".to_owned(), |v| format!("'{v}'"));
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, detected_at_utc, review_verdict)
             VALUES ('{day}', '{time}', 'Sci {com}', '{com}', 0.9, '{com}-{day}-{time}.wav',
                     epoch(TIMESTAMP '{day} {time}'), {verdict});"
        ))
        .expect("seed detection");
}

fn effort(db: &AnalyticsDb, day: &str, source: &str, seconds: f64) {
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO recording_effort VALUES ('{day}', '{source}', {seconds});"
        ))
        .expect("seed effort");
}

fn store() -> (TempDir, AnalyticsDb) {
    let dir = TempDir::new().expect("tempdir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    db.conn()
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS recording_effort (date TEXT, source TEXT, seconds DOUBLE);",
        )
        .expect("ledger");
    for d in 1..=7 {
        effort(&db, &format!("2025-05-0{d}"), "local", 86_400.0);
    }
    effort(&db, "2025-05-15", "local", 30_000.0);
    for d in 16..=21 {
        effort(&db, &format!("2025-05-{d}"), "local", 43_200.0);
        effort(&db, &format!("2025-05-{d}"), "RTSP_1", 43_200.0);
    }
    for d in 1..=3 {
        effort(&db, &format!("2024-06-0{d}"), "local", 86_400.0);
    }
    for d in 1..=4 {
        effort(&db, &format!("2025-06-0{d}"), "local", 86_400.0);
    }
    let thrush = "Song Thrush";
    detection(&db, "2025-05-01", "05:00:00", thrush, Some("rejected"));
    detection(&db, "2025-05-02", "05:00:00", thrush, None);
    detection(&db, "2025-05-02", "07:00:00", thrush, Some("confirmed"));
    detection(&db, "2025-05-04", "05:00:00", thrush, None);
    detection(&db, "2025-05-06", "05:00:00", thrush, None);
    detection(&db, "2025-05-09", "05:00:00", thrush, None);
    detection(&db, "2025-05-15", "05:00:00", thrush, None);
    detection(&db, "2024-06-02", "05:00:00", thrush, None);
    detection(&db, "2025-06-03", "05:00:00", thrush, None);
    detection(&db, "2024-01-15", "05:00:00", thrush, None);
    // Another bird on every listened day, so the station's record is not the
    // Thrush's alone.
    for d in 1..=21 {
        detection(
            &db,
            &format!("2025-05-{d:02}"),
            "06:00:00",
            "Blackbird",
            None,
        );
    }
    (dir, db)
}

fn thrush(db: &AnalyticsDb) -> ReportingFrequency {
    db.reporting_frequency("Song Thrush", FLOOR)
        .expect("the query runs")
}

#[test]
fn a_week_listened_seven_days_and_heard_on_three_reads_three_of_seven() {
    let (_dir, db) = store();
    let rf = thrush(&db);
    assert!(rf.has_ledger);
    let may_1_7 = rf.period(period_of(5, 1)).expect("1–7 May was listened");
    assert_eq!(
        (may_1_7.listened_days, may_1_7.heard_days),
        (7, 3),
        "two detections on the 2nd are one day; the rejected one on the 1st is none: {rf:?}"
    );
}

#[test]
fn a_week_with_no_listening_has_no_row_rather_than_a_share_of_zero() {
    let (_dir, db) = store();
    let rf = thrush(&db);
    assert!(
        rf.period(period_of(5, 8)).is_none(),
        "8–14 May has no ledger row, so nothing is known of it: {rf:?}"
    );
}

#[test]
fn a_short_day_is_not_a_listened_day_and_microphones_add_up() {
    let (_dir, db) = store();
    let rf = thrush(&db);
    let may_15_21 = rf.period(period_of(5, 15)).expect("15–21 May was listened");
    // The 15th recorded 30 000 s, under half the typical day, and is left out
    // with its Thrush; the 16th to the 21st are two half-days on two
    // microphones each, a whole day, and count.
    assert_eq!(
        (may_15_21.listened_days, may_15_21.heard_days),
        (6, 0),
        "{rf:?}"
    );
}

#[test]
fn the_same_part_of_two_years_is_pooled() {
    let (_dir, db) = store();
    let rf = thrush(&db);
    let june_1_7 = rf.period(period_of(6, 1)).expect("1–7 June was listened");
    assert_eq!(
        (june_1_7.listened_days, june_1_7.heard_days),
        (7, 2),
        "{rf:?}"
    );
    assert_eq!(rf.years, 2);
    assert_eq!(rf.first_listened.as_deref(), Some("2024-06-01"));
    assert_eq!(rf.last_listened.as_deref(), Some("2025-06-04"));
}

#[test]
fn days_heard_the_ledger_does_not_count_are_said_not_dropped() {
    let (_dir, db) = store();
    let rf = thrush(&db);
    // Heard (not rejected) on 8 days: 2, 4, 6, 9 and 15 May 2025, 2 June 2024,
    // 3 June 2025 and 15 January 2024. Of those, 9 May (no row), 15 May
    // (short) and 15 January 2024 (before the ledger) are not listened days.
    assert_eq!((rf.heard_days, rf.heard_days_not_counted), (8, 3), "{rf:?}");
    assert!(
        rf.typical_seconds
            .is_some_and(|t| (t - 86_400.0).abs() < 1e-6)
    );
}

#[test]
fn a_species_never_heard_has_no_days_heard() {
    let (_dir, db) = store();
    let rf = db
        .reporting_frequency("Wallcreeper", FLOOR)
        .expect("the query runs");
    assert_eq!(rf.heard_days, 0);
    // Counterpart: the station's listening is still there; the bird is what is missing.
    assert!(rf.periods.iter().all(|p| p.heard_days == 0) && !rf.periods.is_empty());
}

#[test]
fn a_store_with_no_ledger_says_so() {
    let dir = TempDir::new().expect("tempdir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    detection(&db, "2025-05-02", "05:00:00", "Song Thrush", None);
    // A fresh store has the ledger's table and no row in it: nothing to divide by.
    let rf = thrush(&db);
    assert!(!rf.has_ledger && rf.periods.is_empty(), "{rf:?}");
    assert_eq!((rf.heard_days, rf.heard_days_not_counted), (1, 1));
    // Nor with no table at all (a store that predates it).
    db.conn()
        .execute_batch("DROP TABLE IF EXISTS recording_effort;")
        .expect("drop");
    let rf = thrush(&db);
    assert!(!rf.has_ledger && rf.periods.is_empty(), "{rf:?}");
    // Counterpart: one ledger row and the store has a ledger.
    db.conn()
        .execute_batch(
            "CREATE TABLE recording_effort (date TEXT, source TEXT, seconds DOUBLE);
             INSERT INTO recording_effort VALUES ('2025-05-02', 'local', 86400);",
        )
        .expect("ledger");
    let rf = thrush(&db);
    assert!(rf.has_ledger, "{rf:?}");
    assert_eq!(
        rf.period(period_of(5, 1))
            .map(|p| (p.listened_days, p.heard_days)),
        Some((1, 1))
    );
}

#[test]
fn a_floor_outside_a_share_is_refused() {
    let (_dir, db) = store();
    assert!(db.reporting_frequency("Song Thrush", 0.0).is_err());
    assert!(db.reporting_frequency("Song Thrush", 50.0).is_err());
}
