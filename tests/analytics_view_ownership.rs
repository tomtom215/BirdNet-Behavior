//! Two crates define `detections_ts`. They must not disagree.
//!
//! `birdnet-behavioral` and `birdnet-timeseries` both run against the *same*
//! `DuckDB` connection — `AppState::with_timeseries` hands `AnalyticsDb::conn()`
//! straight to `TimeSeriesDb::new` — and both create a view called
//! `detections_ts` with `CREATE OR REPLACE`. Whichever ran most recently on
//! that connection is the one every subsequent query sees, in both crates.
//!
//! That made a definition mismatch far worse than "one crate's pages ignore
//! rejections". Before this suite, the time-series definition omitted the
//! `review_verdict` filter the behavioural one carried, so opening a single
//! time-series page replaced the behavioural view for the rest of the
//! connection's life: a rejected detection reappeared in sessionize, retention,
//! funnel, next-species and co-occurrence, and stayed back until the next full
//! sync happened to restore the other definition. Which number a dashboard
//! showed depended on what the operator had browsed, nothing reported it, and
//! `tests/analytics_divergence.rs` — which exists precisely to catch the two
//! stores disagreeing — could not see it, because both stores were right and
//! the *view* was what changed underneath them.
//!
//! Two gates, because either alone is satisfiable without the other: the texts
//! can agree while the behaviour is broken by a third writer, and the behaviour
//! can pass by accident of ordering while the texts have already drifted.

#![cfg(feature = "analytics")]

use birdnet_db::sqlite::ReviewStatus;
use birdnet_web::state::AppState;

/// Collapse whitespace so formatting differences are not treated as drift.
fn normalise(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The two definitions must be the same statement.
///
/// A cheap early warning: this fails the moment someone edits one crate's view
/// without the other, which is how the original divergence arose.
#[test]
fn both_crates_define_the_same_detections_ts_view() {
    assert_eq!(
        normalise(birdnet_behavioral::queries::CREATE_DETECTIONS_TS_VIEW),
        normalise(birdnet_timeseries::queries::ENSURE_TS_VIEW),
        "birdnet-behavioral and birdnet-timeseries create the same view name on the \
         same connection with CREATE OR REPLACE, so the last one to run decides what \
         every analytic in both crates sees. They must be one statement."
    );
}

/// A station with three detections of three species on one day, synced.
fn station(dir: &std::path::Path) -> (AppState, String) {
    let db_path = dir.join("birds.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    let today: String = conn
        .query_row("SELECT date('now','localtime')", [], |r| r.get(0))
        .expect("today");
    for (time, sci, com) in [
        ("06:15:00", "Turdus merula", "Eurasian Blackbird"),
        ("07:15:00", "Erithacus rubecula", "European Robin"),
        ("08:15:00", "Parus major", "Great Tit"),
    ] {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES (?1, ?2, ?3, ?4, 0.85)",
            rusqlite::params![&today, time, sci, com],
        )
        .unwrap();
    }
    drop(conn);
    let state = AppState::new_with_analytics(db_path, &dir.join("analytics.duckdb"))
        .expect("analytics state opens");
    state
        .resync_analytics_full()
        .expect("analytics is configured")
        .expect("initial sync");
    (state, today)
}

fn ts_rows(state: &AppState) -> i64 {
    state
        .with_analytics(|adb| {
            adb.conn()
                .query_row("SELECT COUNT(*) FROM detections_ts", [], |r| r.get(0))
                .expect("count")
        })
        .expect("analytics is configured")
}

/// Running a time-series query must not change what the behavioural analytics
/// see.
///
/// This is the behavioural half, and the one that actually reproduced: with the
/// pre-fix definitions the count went from 2 to 3 across a single `quiet_days`
/// call, silently readmitting the rejected detection.
#[test]
fn a_time_series_query_does_not_readmit_a_rejected_detection() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    state
        .set_detection_review(
            &today,
            "07:15:00",
            "Erithacus rubecula",
            "European Robin",
            ReviewStatus::Rejected,
            Some("misidentified — traffic noise"),
        )
        .expect("review recorded");

    let before = ts_rows(&state);
    assert_eq!(before, 2, "the rejection reached the analytics store");

    // One time-series page's worth of work, on the same connection.
    let _ = state.with_timeseries(|ts| ts.quiet_days(1, 0));

    assert_eq!(
        ts_rows(&state),
        before,
        "opening a time-series page changed what every behavioural analytic sees"
    );
}

/// The counterpart, so the gate above cannot pass by a view that hides
/// everything: an unreviewed detection must survive both definitions. `NULL <>
/// 'rejected'` is NULL in SQL, which a WHERE treats as false — the exact
/// mistake that would empty every dashboard on a station with a review backlog.
#[test]
fn unreviewed_detections_survive_a_time_series_query() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _today) = station(dir.path());
    assert_eq!(ts_rows(&state), 3, "nothing is reviewed yet");
    let _ = state.with_timeseries(|ts| ts.quiet_days(1, 0));
    assert_eq!(
        ts_rows(&state),
        3,
        "an unreviewed detection must not be filtered out by either definition"
    );
}
