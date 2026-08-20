//! An analytics copy that predates `detected_at_utc` must repair itself.
//!
//! # The failure this exists to catch
//!
//! Migration 32 gave every detection a monotonic instant, and every analytic
//! that measures elapsed time or order moved onto `detection_instant`, which is
//! derived from it. That is the right change for the numbers and it introduces
//! an upgrade hazard the previous drift check could not see:
//!
//! The column adds **no rows** and changes **no verdicts**. A station upgrading
//! with a populated `analytics.duckdb` therefore agrees with SQLite on the row
//! count and on the rejected count — the two signals the startup drift check
//! compared — while every value of the new column in the analytics copy is
//! NULL, because the incremental sync only pulls rows newer than its cutoff and
//! there are none.
//!
//! The result is a station on which sessionize, window-funnel, retention,
//! next-species and every gap query silently return nothing, with both stores
//! answering every query they are asked and no error anywhere. That is the exact
//! shape of the bug the drift check was written for, in a new disguise, which is
//! why the fix is a third signal in the same check rather than a special case.
//!
//! These gates run against a real `DuckDB` because the failure is a value being
//! NULL, not a string being absent.

#![cfg(feature = "analytics")]

use birdnet_web::state::AppState;

/// A station whose analytics copy has been synced, then deliberately regressed
/// to the pre-migration-32 state by blanking the instant column — which is
/// exactly what an upgraded store looks like.
fn station_with_a_stale_analytics_copy(dir: &std::path::Path) -> AppState {
    let db_path = dir.join("birds.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    let today: String = conn
        .query_row("SELECT date('now','localtime')", [], |r| r.get(0))
        .expect("today");
    for (time, sci, com) in [
        ("06:15:00", "Turdus merula", "Eurasian Blackbird"),
        ("06:45:00", "Turdus merula", "Eurasian Blackbird"),
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

    // Regress the copy: this is what a store synced by an older release holds.
    state
        .with_analytics(|adb| {
            adb.conn()
                .execute_batch("UPDATE detections SET detected_at_utc = NULL;")
                .expect("blank the instants");
        })
        .expect("analytics is configured");
    state
}

fn unstamped(state: &AppState) -> u64 {
    state
        .with_analytics(|adb| adb.unstamped_detection_count().expect("count"))
        .expect("analytics is configured")
}

/// Row and verdict counts agree across the regression — which is precisely why
/// a third signal was needed.
///
/// If this ever fails, the hazard has changed shape and the gate below is
/// testing something other than what it claims.
#[test]
fn the_two_older_drift_signals_cannot_see_a_stale_instant_column() {
    let dir = tempfile::tempdir().unwrap();
    let state = station_with_a_stale_analytics_copy(dir.path());

    let sqlite_rows = state
        .with_db(birdnet_db::sqlite::detection_count)
        .expect("count");
    let sqlite_rejected = state
        .with_db(birdnet_db::sqlite::rejected_detection_count)
        .expect("count");
    let (olap_rows, olap_rejected) = state
        .with_analytics(|adb| {
            (
                adb.detection_count().expect("rows"),
                adb.rejected_detection_count().expect("rejected"),
            )
        })
        .expect("analytics is configured");

    assert_eq!(
        u64::try_from(sqlite_rows).unwrap(),
        olap_rows,
        "row counts agree — the old check sees nothing"
    );
    assert_eq!(
        sqlite_rejected, olap_rejected,
        "verdict counts agree — the old check sees nothing"
    );
    assert_eq!(
        unstamped(&state),
        3,
        "and yet every instant in the analytics copy is NULL"
    );
}

/// The third signal notices, and the rebuild repairs it.
#[test]
fn the_drift_check_repairs_an_analytics_copy_that_predates_the_instant() {
    let dir = tempfile::tempdir().unwrap();
    let state = station_with_a_stale_analytics_copy(dir.path());
    assert_eq!(unstamped(&state), 3, "precondition: the copy is stale");

    // What the startup path does when the signals disagree.
    state
        .resync_analytics_full()
        .expect("analytics is configured")
        .expect("rebuild");

    assert_eq!(
        unstamped(&state),
        0,
        "after the rebuild every row carries the instant it has in SQLite"
    );
}

/// And through the path a station actually takes: a restart, with no operator
/// action.
///
/// The two gates above call `resync_analytics_full` directly, which proves the
/// rebuild works and says nothing about whether anything *asks* for it. This
/// goes through `AppState::new_with_analytics` — the startup path — so it fails
/// if the third drift signal is not wired into the comparison there.
#[test]
fn the_startup_drift_check_repairs_a_stale_instant_column_by_itself() {
    let dir = tempfile::tempdir().unwrap();
    let state = station_with_a_stale_analytics_copy(dir.path());
    assert_eq!(unstamped(&state), 3, "precondition: the copy is stale");
    drop(state);

    let reopened = AppState::new_with_analytics(
        dir.path().join("birds.db"),
        &dir.path().join("analytics.duckdb"),
    )
    .expect("analytics state reopens");

    assert_eq!(
        unstamped(&reopened),
        0,
        "a station upgrading with a populated analytics copy must repair it on \
         the next start — row counts and verdict counts both agree, so nothing \
         else in the drift check can notice"
    );
}

/// And the point of repairing it: an elapsed-time analytic that returned
/// nothing on the stale copy returns a real answer on the repaired one.
///
/// Asserting the count is not enough — a rebuild that copied the column and
/// broke the view would still pass that.
#[test]
fn an_elapsed_time_query_comes_back_to_life_after_the_rebuild() {
    let dir = tempfile::tempdir().unwrap();
    let state = station_with_a_stale_analytics_copy(dir.path());

    let gap_minutes = |state: &AppState| -> Option<f64> {
        state
            .with_analytics(|adb| {
                adb.conn()
                    .query_row(
                        "SELECT date_diff('minute', MIN(detection_instant), MAX(detection_instant))
                           FROM detections_ts WHERE Sci_Name = 'Turdus merula'",
                        [],
                        |r| r.get(0),
                    )
                    .expect("query")
            })
            .expect("analytics is configured")
    };

    assert_eq!(
        gap_minutes(&state),
        None,
        "on the stale copy the instant is NULL and the answer is nothing at all"
    );

    state
        .resync_analytics_full()
        .expect("analytics is configured")
        .expect("rebuild");

    assert_eq!(
        gap_minutes(&state),
        Some(30.0),
        "the two blackbirds are half an hour apart, and the repaired copy says so"
    );
}
