//! A reviewer's verdict has to change the numbers, or it is not a review.
//!
//! `detection_reviews` has stored confirmed/rejected verdicts since migration
//! 13, and exactly one surface ever read them: the quality dashboard's own
//! "Review verdict trend" panel. Every other analytic — species counts, the life
//! list, the heat map, the dawn chorus, phenology, every behavioural and
//! time-series query — counted a rejected detection exactly as it counted a
//! confirmed one.
//!
//! So an operator could spend a season rejecting false positives and every chart
//! would look exactly as it did before. The only way to make a rejection *mean*
//! anything was to delete the detection, which discards the evidence — the
//! opposite of what a reviewable record is for. For a research station that is
//! the difference between a log of what a model reported and a dataset a
//! reviewer stands behind.
//!
//! # The two halves this suite holds apart
//!
//! * **Aggregates exclude rejects.** A count is a claim about what was there,
//!   and a reviewer who rejected a detection has said it was not.
//! * **Record-level surfaces still show them.** A reviewer must be able to find
//!   a rejected detection, listen again and change their mind. A verdict that
//!   hid its own evidence would be a trap, and clearing it must bring the
//!   detection straight back.
//!
//! Both halves are gated, because a change that satisfied only the first would
//! look like a fix and be a data-loss bug.

#![cfg(feature = "analytics")]

use birdnet_db::sqlite::ReviewStatus;
use birdnet_web::state::AppState;

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

/// Record a rejection through the **paired** write the routes use.
///
/// Deliberately not `with_db(set_detection_review)`: that reaches SQLite alone,
/// which is precisely the defect. A test that used it would pass the SQLite
/// assertions and be blind to the half that matters for the behavioural
/// dashboards.
fn reject(state: &AppState, date: &str, time: &str, sci: &str, com: &str) {
    state
        .set_detection_review(
            date,
            time,
            sci,
            com,
            ReviewStatus::Rejected,
            Some("misidentified — traffic noise"),
        )
        .expect("record verdict");
}

/// Rows the SQLite aggregate surfaces see.
fn analytic_rows(state: &AppState) -> i64 {
    state
        .with_db(|conn| {
            conn.query_row("SELECT COUNT(*) FROM detections_analytic", [], |r| {
                r.get::<_, i64>(0)
            })
        })
        .expect("count")
}

/// Rows the record-level surfaces see.
fn record_rows(state: &AppState) -> i64 {
    state
        .with_db(|conn| {
            conn.query_row("SELECT COUNT(*) FROM detections", [], |r| {
                r.get::<_, i64>(0)
            })
        })
        .expect("count")
}

/// Rows the DuckDB analytics read.
fn olap_analytic_rows(state: &AppState) -> i64 {
    state
        .with_analytics(|adb| {
            adb.conn()
                .query_row("SELECT COUNT(*) FROM detections_ts", [], |r| {
                    r.get::<_, i64>(0)
                })
                .expect("count")
        })
        .expect("analytics is configured")
}

/// Rejecting a detection removes it from the SQLite aggregates.
#[test]
fn a_rejected_detection_leaves_the_sqlite_aggregates() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    assert_eq!(analytic_rows(&state), 3, "fixture");

    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );

    assert_eq!(
        analytic_rows(&state),
        2,
        "the rejected detection is still counted by every SQLite aggregate — \
         species totals, the heat map, the dawn chorus, phenology"
    );
    // The evidence survives.
    assert_eq!(
        record_rows(&state),
        3,
        "rejecting must annotate, never delete — the audio and the row stay"
    );
}

/// …and from the DuckDB analytics, which is where the behavioural and
/// time-series dashboards read.
///
/// Fixing only the SQLite half would look like a fix and leave every
/// behavioural dashboard still counting the reject.
#[test]
fn a_rejected_detection_leaves_the_duckdb_analytics() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    assert_eq!(olap_analytic_rows(&state), 3, "fixture");

    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );

    assert_eq!(
        olap_analytic_rows(&state),
        2,
        "the reject still counts in every behavioural and time-series dashboard"
    );
}

/// Unreviewed detections must stay in. The counterpart that stops the fix from
/// being a blanket "hide everything".
///
/// This is not hypothetical: SQL three-valued logic makes `<> 'rejected'`
/// evaluate to NULL for an unreviewed row, and `WHERE` treats NULL as false — so
/// the obvious spelling of this filter would have hidden every detection nobody
/// had looked at yet, which on a real station is nearly all of them.
#[test]
fn unreviewed_and_confirmed_detections_stay_in_the_aggregates() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());

    state
        .set_detection_review(
            &today,
            "06:15:00",
            "Turdus merula",
            "Eurasian Blackbird",
            ReviewStatus::Confirmed,
            None,
        )
        .expect("confirm");

    assert_eq!(
        analytic_rows(&state),
        3,
        "confirming a detection, and leaving two unreviewed, must change nothing"
    );
    assert_eq!(olap_analytic_rows(&state), 3, "same in the OLAP copy");
}

/// Clearing a verdict brings the detection back.
///
/// Without this the exclusion would outlive the judgement that justified it,
/// and an accidental click would be unrecoverable through the UI.
#[test]
fn clearing_a_verdict_returns_the_detection_to_the_aggregates() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    reject(
        &state,
        &today,
        "07:15:00",
        "Erithacus rubecula",
        "European Robin",
    );
    assert_eq!(analytic_rows(&state), 2, "rejected");

    state
        .clear_detection_review(&today, "07:15:00", "Erithacus rubecula")
        .expect("clear verdict");

    assert_eq!(
        analytic_rows(&state),
        3,
        "clearing the verdict must undo the exclusion"
    );
}

/// A verdict recorded before this feature existed takes effect on upgrade.
///
/// Migration 26 backfills `review_verdict` from `detection_reviews`. Without the
/// backfill, every verdict an operator had already recorded would keep counting
/// for nothing, and only reviews made after the upgrade would mean anything —
/// which is precisely the complaint the feature exists to answer.
#[test]
fn verdicts_recorded_before_the_upgrade_are_backfilled() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("birds.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();

    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES ('2026-03-01','07:15:00','Erithacus rubecula','European Robin',0.8)",
        [],
    )
    .unwrap();
    // A verdict written straight to the table, as a pre-upgrade station holds
    // it: `detection_reviews` populated, `review_verdict` still NULL.
    conn.execute(
        "INSERT INTO detection_reviews (date, time, sci_name, com_name, status)
         VALUES ('2026-03-01','07:15:00','Erithacus rubecula','European Robin','rejected')",
        [],
    )
    .unwrap();
    conn.execute("UPDATE detections SET review_verdict = NULL", [])
        .unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM detections_analytic", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1,
        "before the backfill the verdict counts for nothing — this is the state \
         every existing station is in"
    );

    // Re-running the chain is a no-op for applied migrations, so drive the
    // backfill the way the migration does.
    conn.execute(
        "UPDATE detections
            SET review_verdict = (
                  SELECT r.status FROM detection_reviews r
                   WHERE r.date = detections.Date
                     AND r.time = detections.Time
                     AND r.sci_name = detections.Sci_Name)
          WHERE EXISTS (
                  SELECT 1 FROM detection_reviews r
                   WHERE r.date = detections.Date
                     AND r.time = detections.Time
                     AND r.sci_name = detections.Sci_Name)",
        [],
    )
    .unwrap();

    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM detections_analytic", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0,
        "the backfill must make an already-recorded verdict count"
    );
}
