//! CHT-11: the end of the record is not a bird that failed to come back.
//!
//! Every distinct day a species was heard is a cohort anchor, and the rate at
//! N days is the share of anchors after which it was heard again within N
//! days. Nothing asked whether the record went on for N days after the anchor,
//! so an anchor three days before the end of the record was averaged in as
//! "did not return within a month" although nobody could have known. Every
//! long-interval rate was biased down, most for recent arrivals and on young
//! stations.
//!
//! An anchor is now eligible for an interval only when the record extends that
//! far past it (right-censoring), each rate carries the number of eligible
//! anchors, and an interval no anchor is eligible for has no rate at all
//! rather than 0 %.
//!
//! Every date is fixed: the record runs 2025-03-01 to 2025-06-30, the last day
//! the station heard any bird (a Blackbird heard daily throughout).

#![cfg(feature = "analytics")]

use birdnet_behavioral::connection::AnalyticsDb;
use birdnet_behavioral::types::{RetentionParams, SpeciesRetention};

/// Insert one detection of `com` on each of `days` (ISO dates).
fn heard(db: &AnalyticsDb, com: &str, days: &[&str]) {
    for d in days {
        db.conn()
            .execute_batch(&format!(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
                 VALUES ('{d}', '06:00:00', 'Sci {com}', '{com}', 0.9,
                         epoch(TIMESTAMP '{d} 06:00:00'));"
            ))
            .expect("seed");
    }
}

/// Insert one detection of `com` on every day from `from` for `n` days.
fn daily(db: &AnalyticsDb, com: &str, from: &str, n: u32) {
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
             SELECT strftime(t, '%Y-%m-%d'), '06:00:00', 'Sci {com}', '{com}', 0.9,
                    epoch(t + INTERVAL 6 HOUR)
               FROM range(TIMESTAMP '{from}', TIMESTAMP '{from}' + INTERVAL {n} DAY,
                          INTERVAL 1 DAY) r(t);"
        ))
        .expect("seed");
}

fn store() -> (tempfile::TempDir, AnalyticsDb) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    db.load_extension().expect("the behavioral extension loads");
    // The record: 2025-03-01 .. 2025-06-30 inclusive, 122 days.
    daily(&db, "Eurasian Blackbird", "2025-03-01", 122);
    // Censored: a visitor every ten days in the last month of the record. Each
    // visit was followed by the next while the record could see it; the last
    // one, nine days before the end, could not be followed within a fortnight
    // or a month by anything the station had yet recorded.
    heard(
        &db,
        "Common Swift",
        &["2025-06-01", "2025-06-11", "2025-06-21"],
    );
    // Newly arrived, heard daily over the last six days of the record: no
    // anchor is a week from the end, so nothing is known about a week.
    daily(&db, "Spotted Flycatcher", "2025-06-25", 6);
    // Counterpart: heard weekly through March, then never again while the
    // record ran for three more months. That is a real absence.
    heard(
        &db,
        "Wallcreeper",
        &[
            "2025-03-01",
            "2025-03-08",
            "2025-03-15",
            "2025-03-22",
            "2025-03-29",
        ],
    );
    (dir, db)
}

fn retention(db: &AnalyticsDb) -> Vec<SpeciesRetention> {
    db.retention(&RetentionParams {
        min_detections: 1,
        ..RetentionParams::default()
    })
    .expect("retention runs")
}

/// The reported rate of `sp` at `days`, and the number of anchors it is over.
fn cell(rows: &[SpeciesRetention], sp: &str, days: u32) -> (Option<f64>, u32) {
    let r = rows
        .iter()
        .find(|r| r.species == sp)
        .unwrap_or_else(|| panic!("{sp} is listed: {rows:?}"));
    let x = r
        .retention_rates
        .iter()
        .find(|x| x.days == days)
        .unwrap_or_else(|| panic!("{sp} has a {days}-day cell: {r:?}"));
    (x.rate, x.eligible)
}

fn close(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 1e-9)
}

#[test]
fn a_visit_the_record_could_not_follow_is_not_counted_as_no_return() {
    let (_dir, db) = store();
    let rows = retention(&db);
    // 06-01 and 06-11 are each followed ten days later; 06-21 is nine days
    // from the end. At 14 days the eligible anchors are 06-01 and 06-11
    // (06-11 + 14 = 06-25 <= 06-30); both returned.
    let (rate, n) = cell(&rows, "Common Swift", 14);
    assert!(
        close(rate, 1.0) && n == 2,
        "Swift at 14 d reads {rate:?} over {n} anchors; the record could follow two \
         visits a fortnight, and both were followed by another (uncensored: 2 of 3)"
    );
    // At 30 days not even 06-01 is eligible (06-01 + 30 = 07-01, a day past
    // the end), so there is nothing to say; uncensored, this too read 2 of 3.
    let (rate, n) = cell(&rows, "Common Swift", 30);
    assert!(rate.is_none() && n == 0, "Swift at 30 d: {rate:?} over {n}");
    // A Blackbird heard every day returned after every day the record could
    // follow (uncensored, its last day dragged every interval to 121/122).
    for days in [1, 2, 3, 7, 14, 30] {
        let (rate, n) = cell(&rows, "Eurasian Blackbird", days);
        assert!(
            close(rate, 1.0) && n == 122 - days,
            "Blackbird at {days} d: {rate:?} over {n}"
        );
    }
}

#[test]
fn an_interval_no_anchor_reaches_has_no_rate_rather_than_zero() {
    let (_dir, db) = store();
    let rows = retention(&db);
    // Heard 06-25 .. 06-30: a week after the first anchor is 07-02, past the end.
    let (rate, n) = cell(&rows, "Spotted Flycatcher", 7);
    assert!(
        rate.is_none() && n == 0,
        "Flycatcher at 7 d reads {rate:?} over {n} anchors; no anchor is a week from \
         the end of the record, so there is nothing to say"
    );
    // Counterpart: a day after each of the first five anchors is inside the record.
    let (rate, n) = cell(&rows, "Spotted Flycatcher", 1);
    assert!(
        close(rate, 1.0) && n == 5,
        "Flycatcher at 1 d: {rate:?} over {n}"
    );
}

#[test]
fn a_bird_that_stopped_coming_inside_the_record_still_reads_as_not_returning() {
    let (_dir, db) = store();
    let rows = retention(&db);
    // Five weekly visits, the last on 03-29 with three months of record after
    // it: every anchor is eligible at every interval. None returned within a
    // day; four of five within a week, a fortnight or a month.
    let (rate, n) = cell(&rows, "Wallcreeper", 1);
    assert!(
        close(rate, 0.0) && n == 5,
        "Wallcreeper at 1 d: {rate:?} over {n}"
    );
    for days in [7, 14, 30] {
        let (rate, n) = cell(&rows, "Wallcreeper", days);
        assert!(
            close(rate, 0.8) && n == 5,
            "Wallcreeper at {days} d: {rate:?} over {n}; its last visit is a real absence"
        );
    }
}
