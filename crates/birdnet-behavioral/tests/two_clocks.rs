//! `detections_ts` carries two clocks, and they must not be interchangeable.
//!
//! # What this is defending
//!
//! `detection_timestamp` is the station's local wall clock — the right thing to
//! ask for hour-of-day, calendar date, and anything shown to a human.
//! `detection_instant` is the same detection as a point in time — the right
//! thing to ask for order and elapsed time.
//!
//! Local wall clock is not monotonic. One hour repeats every autumn and one
//! never happens every spring, so every duration measured on it is wrong across
//! a transition. Before `detected_at_utc` existed there was only the wall clock,
//! and `sessionize`, `window_funnel`, `sequence_match` and every gap query
//! measured elapsed time on it.
//!
//! These gates run against a real `DuckDB` with the real view, because the
//! failure they describe is a *value* being wrong, not a string being absent.

use birdnet_behavioral::connection::AnalyticsDb;

/// 2026-10-25, Europe/Berlin: the offset moves +2 → +1 at 01:00 UTC, so local
/// 02:30 happens twice — at 00:30Z and again at 01:30Z, one real hour apart.
const FIRST_PASS: i64 = 1_792_888_200;
const SECOND_PASS: i64 = 1_792_891_800;

fn seeded() -> (tempfile::TempDir, AnalyticsDb) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = AnalyticsDb::open(&dir.path().join("a.duckdb")).expect("open");
    db.conn()
        .execute_batch(&format!(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, detected_at_utc)
             VALUES ('2026-10-25','02:30:00','Turdus merula','Blackbird',0.9,{FIRST_PASS}),
                    ('2026-10-25','02:30:00','Turdus merula','Blackbird',0.9,{SECOND_PASS}),
                    ('2026-05-01','06:00:00','Parus major','Great Tit',0.9,NULL),
                    ('','','Erithacus rubecula','Robin',0.9,NULL);"
        ))
        .expect("seed");
    (dir, db)
}

fn minutes(db: &AnalyticsDb, col: &str) -> Option<f64> {
    db.conn()
        .query_row(
            &format!(
                "SELECT date_diff('minute', MIN({col}), MAX({col}))
                   FROM detections_ts WHERE Date = '2026-10-25'"
            ),
            [],
            |r| r.get(0),
        )
        .expect("query")
}

/// The reason the column exists, as a number.
///
/// Two detections one real hour apart, carrying identical `Date`/`Time`. On the
/// instant they are sixty minutes apart. On the wall clock they are zero — and
/// zero is what every session gap, funnel window and sequence-match interval in
/// this crate was computing on the autumn night.
#[test]
fn one_real_hour_reads_as_an_hour_on_the_instant_and_zero_on_the_wall_clock() {
    let (_dir, db) = seeded();
    assert_eq!(
        minutes(&db, "detection_instant"),
        Some(60.0),
        "the instant must know an hour passed"
    );
    assert_eq!(
        minutes(&db, "detection_timestamp"),
        Some(0.0),
        "the wall clock cannot — which is why it is not what durations ask"
    );
}

/// The local clock has to stay local, or every hour-of-day chart in the app
/// silently moves by the station's offset. This is the bug class the day strip
/// and the dawn-chorus markers have already been fixed for twice.
#[test]
fn the_wall_clock_column_is_still_the_wall_clock() {
    let (_dir, db) = seeded();
    let hour: Option<i8> = db
        .conn()
        .query_row(
            "SELECT EXTRACT(HOUR FROM detection_timestamp) FROM detections_ts
              WHERE Com_Name = 'Great Tit'",
            [],
            |r| r.get(0),
        )
        .expect("query");
    assert_eq!(
        hour,
        Some(6),
        "a 06:00 local detection must read as hour 6, whatever the offset"
    );
}

/// A row with no instant — history predating migration 32, or a wall clock that
/// names no point in time — must yield NULL rather than an epoch default, so it
/// drops out of ordered results instead of appearing on 1970-01-01.
#[test]
fn a_row_without_an_instant_yields_null_not_an_epoch_default() {
    let (_dir, db) = seeded();
    let nulls: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM detections_ts WHERE detection_instant IS NULL",
            [],
            |r| r.get(0),
        )
        .expect("query");
    assert_eq!(nulls, 2, "the un-stamped and the unplaceable row");
    let epoch: i64 = db
        .conn()
        .query_row(
            "SELECT COUNT(*) FROM detections_ts
              WHERE detection_instant < TIMESTAMP '1971-01-01'",
            [],
            |r| r.get(0),
        )
        .expect("query");
    assert_eq!(epoch, 0, "no detection was invented at the epoch");
}
