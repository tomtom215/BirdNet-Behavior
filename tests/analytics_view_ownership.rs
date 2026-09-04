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

// ---------------------------------------------------------------------------
// The same failure, reintroduced by migration 34's second filter.
//
// Everything above is about the `review_verdict` clause, which both crates
// carry. Migration 34 added a *second* rule to the same view: on a station that
// has imported another site's history and asked for it to be excluded, the
// behavioural view gains `AND import_batch_id IS NULL`.
//
// That rule is built by `birdnet_behavioral::queries::detections_ts_view_sql`,
// which takes the flag. `birdnet-timeseries` has no such function — it has one
// constant, and `TimeSeriesDb::new` ran `CREATE OR REPLACE VIEW` with it on
// every construction. So on a station with the setting on, opening any
// time-series page replaced the excluding view with the including one for the
// rest of the connection's life, and sessionize, retention, funnel,
// next-species, co-occurrence and phenology all silently began counting another
// station's records as this one's — the exact damage `provenance.rs` warns is
// "not detectable after the fact".
//
// The gate above could not see it: it compares `CREATE_DETECTIONS_TS_VIEW`
// against `ENSURE_TS_VIEW`, and those two are identical. The excluding variant
// has no counterpart in `birdnet-timeseries` to be compared with.
// ---------------------------------------------------------------------------

/// The station above, plus two detections imported from somewhere else.
fn station_with_an_import(dir: &std::path::Path) -> (AppState, String) {
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
    conn.execute(
        "INSERT INTO import_batches (id, source_kind, row_count) VALUES (1, 'birdnet-pi', 2)",
        [],
    )
    .unwrap();
    for (time, sci, com) in [
        ("09:15:00", "Sylvia atricapilla", "Eurasian Blackcap"),
        ("10:15:00", "Fringilla coelebs", "Common Chaffinch"),
    ] {
        conn.execute(
            "INSERT INTO detections
               (Date, Time, Sci_Name, Com_Name, Confidence, import_batch_id)
             VALUES (?1, ?2, ?3, ?4, 0.85, 1)",
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

/// The definition of `detections_ts` as `DuckDB` currently holds it.
fn view_sql(state: &AppState) -> String {
    state
        .with_analytics(|adb| {
            adb.conn()
                .query_row(
                    "SELECT sql FROM duckdb_views() WHERE view_name = 'detections_ts'",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .expect("detections_ts must exist")
        })
        .expect("analytics is configured")
}

/// The behavioural half, and the one that reproduces: an operator who asked for
/// another site's history to be excluded must not have it counted again because
/// somebody opened a chart.
#[test]
fn an_excluded_import_stays_excluded_through_a_time_series_query() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _today) = station_with_an_import(dir.path());

    state
        .with_analytics(|adb| adb.set_exclude_imports(true))
        .expect("analytics is configured")
        .expect("exclude imports");

    let before = ts_rows(&state);
    assert_eq!(
        before, 3,
        "the exclusion reached the analytics store: three recorded here, two imported"
    );

    // One time-series page's worth of work, on the same connection.
    let _ = state.with_timeseries(|ts| ts.quiet_days(1, 0));

    assert_eq!(
        ts_rows(&state),
        before,
        "opening a time-series page readmitted another station's imported \
         detections into every behavioural analytic"
    );
}

/// The counterpart, so the gate above cannot pass by a view that excludes
/// imports unconditionally. Including an import is a legitimate choice — it is
/// the default, and merging two sites is a thing operators do — so a station
/// that has not asked for the exclusion must still see all five.
#[test]
fn an_included_import_stays_included_through_a_time_series_query() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _today) = station_with_an_import(dir.path());

    state
        .with_analytics(|adb| adb.set_exclude_imports(false))
        .expect("analytics is configured")
        .expect("include imports");

    assert_eq!(ts_rows(&state), 5, "the default counts imported rows");
    let _ = state.with_timeseries(|ts| ts.quiet_days(1, 0));
    assert_eq!(
        ts_rows(&state),
        5,
        "an included import must not be filtered out by either definition"
    );
}

/// The sharp one, and the rule rather than one of its consequences.
///
/// `detections_ts` has exactly one owner: `birdnet-behavioral`, which is the
/// only crate that knows the flag. Constructing a time-series executor is a
/// read, and a read must not redefine the catalog underneath the other crate.
///
/// This is stated as "the definition does not change" rather than "the
/// definition contains `import_batch_id`", so a third rule added to the view
/// later inherits the protection instead of needing its own gate — which is how
/// the second rule came to be unprotected.
#[test]
fn constructing_the_executor_does_not_redefine_the_view() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _today) = station_with_an_import(dir.path());
    state
        .with_analytics(|adb| adb.set_exclude_imports(true))
        .expect("analytics is configured")
        .expect("exclude imports");

    let before = view_sql(&state);
    assert!(
        before.contains("import_batch_id"),
        "the fixture must start from the excluding definition, or this gate is \
         vacuous — got: {before}"
    );

    let _ = state.with_timeseries(|ts| ts.quiet_days(1, 0));

    assert_eq!(
        view_sql(&state),
        before,
        "constructing a TimeSeriesDb rewrote the shared `detections_ts` view"
    );
}
