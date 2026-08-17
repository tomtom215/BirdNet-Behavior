//! The OLAP copy must follow the operator's edits, not just their detections.
//!
//! `SQLite` is the source of truth; `DuckDB` is a derived copy that every
//! behavioural and time-series dashboard reads. The write paths keep the two in
//! step *for new detections* — `src/daemon/processor.rs` inserts into both — but
//! four ordinary operator actions write only to `SQLite`:
//!
//! | Action | Route | `SQLite` write |
//! |---|---|---|
//! | Delete a detection | `/pages/today-delete`, `/pages/recordings-delete` | `delete_detection` |
//! | Re-label a detection | `/pages/today-relabel` | `relabel_detection` |
//! | Clear all data | `/admin` → System → Clear detections | `DELETE FROM detections` |
//! | Approve a quarantined detection | `/pages/quarantine-*` | `approve_quarantine` |
//!
//! Nothing reconciles the difference afterwards. The startup sync is
//! *incremental* — its cutoff is the newest row already in `DuckDB` — so it can
//! only ever add newer rows; it never removes one, never re-reads a changed
//! one, and skips a back-dated one entirely. `full_resync_from_sqlite` is the
//! only repair, and until this suite existed it was reachable from exactly one
//! place: finishing a BirdNET-Pi migration.
//!
//! The failure is silent and permanent. An operator who deletes a false
//! positive sees it vanish from Today and keep counting in Patterns forever;
//! one who clears the station's data sees `0` detections on the dashboard and a
//! full history in the analytics dashboards beside it.
//!
//! The suite gates this at two levels, because they fail independently:
//!
//! * **Contract** — `AppState`'s paired writes touch both stores. A regression
//!   here breaks every caller at once.
//! * **Routes** — the handlers actually *use* the paired writes. A new route
//!   that reaches for `with_db(|c| birdnet_db::sqlite::…)` compiles, passes the
//!   contract tests, and reintroduces the whole defect; only driving the real
//!   HTTP handler catches that.

#![cfg(feature = "analytics")]

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A station with three detections, synced into `DuckDB`.
///
/// Dates are relative to today for the same reason the rest of the suite does
/// it: the dashboards filter on a look-back from `CURRENT_DATE`.
fn station(dir: &std::path::Path) -> (AppState, String) {
    let db_path = dir.join("birds.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();

    let today: String = conn
        .query_row("SELECT date('now')", [], |r| r.get(0))
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

/// Rows currently in the `DuckDB` copy.
fn olap_count(state: &AppState) -> u64 {
    state
        .with_analytics(birdnet_behavioral::connection::AnalyticsDb::detection_count)
        .expect("analytics is configured")
        .expect("count")
}

/// Rows in the `DuckDB` copy carrying `sci_name`.
fn olap_count_of(state: &AppState, sci_name: &str) -> i64 {
    state
        .with_analytics(|adb| {
            adb.conn()
                .query_row(
                    &format!("SELECT COUNT(*) FROM detections WHERE Sci_Name = '{sci_name}'"),
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .expect("count by species")
        })
        .expect("analytics is configured")
}

/// Deleting a detection must remove it from the analytics copy too.
#[test]
fn deleting_a_detection_removes_it_from_the_olap_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    assert_eq!(olap_count(&state), 3, "fixture");

    let deleted = state
        .delete_detection(&today, "07:15:00", "Erithacus rubecula")
        .expect("delete");
    assert!(deleted, "the row was there to delete");

    assert_eq!(
        olap_count(&state),
        2,
        "the deleted detection is still in the analytics copy, so every \
         behavioural and time-series dashboard keeps counting it"
    );
}

/// Re-labelling a detection must correct it in the analytics copy too.
#[test]
fn relabelling_a_detection_updates_the_olap_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    assert_eq!(olap_count_of(&state, "Parus major"), 1, "fixture");

    let relabelled = state
        .relabel_detection(
            &today,
            "08:15:00",
            "Parus major",
            "Cyanistes caeruleus",
            "Eurasian Blue Tit",
        )
        .expect("relabel");
    assert!(relabelled, "the row was there to relabel");

    assert_eq!(
        olap_count_of(&state, "Parus major"),
        0,
        "the old identification survives in the analytics copy"
    );
    assert_eq!(
        olap_count_of(&state, "Cyanistes caeruleus"),
        1,
        "the corrected identification never reaches the analytics copy"
    );
}

/// Approving a quarantined detection must admit it to the analytics copy.
///
/// This one cannot be repaired by a restart even in principle: the quarantined
/// row carries its *original* timestamp, so it is back-dated relative to
/// whatever `DuckDB` already holds, and the incremental sync's `>= cutoff`
/// filter skips it for good.
#[test]
fn approving_a_quarantined_detection_admits_it_to_the_olap_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    assert_eq!(olap_count(&state), 3, "fixture");

    state
        .with_db(|conn| {
            birdnet_db::sqlite::insert_quarantine(
                conn,
                &birdnet_db::sqlite::QuarantineRecord {
                    date: &today,
                    time: "05:00:00",
                    sci_name: "Strix aluco",
                    com_name: "Tawny Owl",
                    confidence: 0.55,
                    sf_probability: None,
                    reason: birdnet_db::sqlite::QuarantineReason::LowConfidence,
                    file_name: None,
                    lat: None,
                    lon: None,
                    week: None,
                },
            )
        })
        .expect("quarantine insert");
    let id = state
        .with_db(|conn| {
            conn.query_row("SELECT id FROM quarantine LIMIT 1", [], |r| {
                r.get::<_, i64>(0)
            })
        })
        .expect("quarantine id");

    let admitted = state.approve_quarantine(id).expect("approve");
    assert!(admitted, "the row was newly admitted to SQLite");

    assert_eq!(
        olap_count(&state),
        4,
        "an approved detection never reaches the analytics copy — and being \
         back-dated, no later incremental sync will pick it up either"
    );
}

/// Clearing the station's data must clear the analytics copy too.
///
/// The strongest of the four: the dashboard reports zero detections while every
/// analytics dashboard still renders the whole history.
#[test]
fn clearing_detections_clears_the_olap_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _today) = station(dir.path());
    assert_eq!(olap_count(&state), 3, "fixture");

    state.clear_detections().expect("clear");

    assert_eq!(
        olap_count(&state),
        0,
        "'clear all detections' left the analytics copy fully populated"
    );
}

/// A station that already diverged must repair itself on the next start.
///
/// The paired writes above stop *new* divergence. They do nothing for the
/// stations already running: every install that ever deleted a detection, cleared
/// its data, or approved a quarantined row carries a permanently wrong analytics
/// copy, and nothing an operator can reach from the UI rebuilds it.
///
/// So the startup sync now compares row counts afterwards and rebuilds when they
/// disagree. This test creates exactly that state — rows removed from `SQLite`
/// behind the copy's back, which is what the old code left behind — and asserts
/// the next `AppState` heals it with no operator action.
#[test]
fn a_station_that_already_diverged_repairs_itself_on_the_next_start() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    assert_eq!(olap_count(&state), 3, "fixture");

    // Exactly what 0.14.0 and earlier did: delete from SQLite only.
    state
        .with_db(|conn| {
            birdnet_db::sqlite::delete_detection(conn, &today, "07:15:00", "Erithacus rubecula")
        })
        .expect("delete");
    assert_eq!(
        olap_count(&state),
        3,
        "the copy is now stale, as it would be"
    );
    drop(state);

    let reopened = AppState::new_with_analytics(
        dir.path().join("birds.db"),
        &dir.path().join("analytics.duckdb"),
    )
    .expect("analytics state reopens");

    assert_eq!(
        olap_count(&reopened),
        2,
        "an already-diverged station stayed diverged across a restart"
    );
}

// ---------------------------------------------------------------------------
// Route-level gates
// ---------------------------------------------------------------------------
//
// The contract tests above prove `AppState` keeps both stores in step. These
// prove the handlers *use* it. The distinction is not academic: every one of
// the four defects this suite covers existed precisely because a handler
// reached past the pairing to the raw `SQLite` helper.

/// POST a form to `uri` and return the status.
async fn post_form(state: &AppState, uri: &str, body: String) -> StatusCode {
    build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

/// The Today page's delete button must reach the analytics copy.
#[tokio::test]
async fn today_delete_route_removes_the_detection_from_the_olap_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (state, today) = station(dir.path());
    assert_eq!(olap_count(&state), 3, "fixture");

    let status = post_form(
        &state,
        "/pages/today-delete",
        format!("date={today}&time=07%3A15%3A00&sci_name=Erithacus+rubecula"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the delete route answered");

    assert_eq!(
        olap_count(&state),
        2,
        "the Today delete handler bypassed the paired write"
    );
}

/// The admin "clear detections" control must reach the analytics copy.
#[tokio::test]
async fn admin_clear_route_clears_the_olap_copy() {
    let dir = tempfile::tempdir().unwrap();
    let (state, _today) = station(dir.path());
    assert_eq!(olap_count(&state), 3, "fixture");

    let status = post_form(&state, "/admin/system/clear-detections", String::new()).await;
    assert_eq!(status, StatusCode::OK, "the clear route answered");

    assert_eq!(
        olap_count(&state),
        0,
        "the admin clear handler bypassed the paired write"
    );
}

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

/// Imported history must stay separable in the *analytics* store, not just in
/// SQLite.
///
/// Tagging rows in SQLite alone would be a half-fix: every behavioural and
/// time-series dashboard reads DuckDB, so if the column stops at the boundary
/// then the merged history is still one undifferentiated site everywhere a
/// researcher would actually look at it.
#[test]
fn provenance_survives_into_the_analytics_store() {
    use birdnet_migrate::birdnet_pi::BirdNetPiImporter;
    use birdnet_migrate::progress::ProgressHandle;
    use birdnet_migrate::provenance::ImportOptions;

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("birds.db");
    {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES ('2026-03-01','05:00:00','Erithacus rubecula','European Robin',0.8)",
            [],
        )
        .unwrap();
    }

    // A BirdNET-Pi database from 343 km away, on a clock six hours behind.
    let src = dir.path().join("other-station.db");
    {
        let conn = rusqlite::Connection::open(&src).unwrap();
        conn.execute_batch(
            "CREATE TABLE detections (Date TEXT, Time TEXT, Sci_Name TEXT, Com_Name TEXT,
             Confidence REAL, Lat REAL, Lon REAL, Cutoff REAL, Week INTEGER,
             Sens REAL, Overlap REAL, File_Name TEXT);
             INSERT INTO detections VALUES
               ('2026-03-01','06:30:00','Turdus merula','Eurasian Blackbird',0.9,
                48.8566,2.3522,0.7,1,1.0,0.0,'a.wav'),
               ('2026-03-02','06:30:00','Turdus merula','Eurasian Blackbird',0.9,
                48.8566,2.3522,0.7,1,1.0,0.0,'b.wav');",
        )
        .unwrap();
    }

    BirdNetPiImporter
        .migrate_with_options(
            &src,
            &db_path,
            &ProgressHandle::new(),
            &ImportOptions {
                shift_secs: 6 * 3600,
                label: Some("Paris transect".into()),
                source_utc_offset_secs: Some(-5 * 3600),
                notes: None,
            },
            (Some(51.5074), Some(-0.1278)),
        )
        .unwrap();

    let state = AppState::new_with_analytics(db_path, &dir.path().join("analytics.duckdb"))
        .expect("analytics state opens");
    state
        .resync_analytics_full()
        .expect("analytics is configured")
        .expect("resync");

    let (local, imported) = state
        .with_analytics(|adb| {
            let q = |sql: &str| {
                adb.conn()
                    .query_row(sql, [], |r| r.get::<_, i64>(0))
                    .expect("count")
            };
            (
                q("SELECT COUNT(*) FROM detections WHERE import_batch_id IS NULL"),
                q("SELECT COUNT(*) FROM detections WHERE import_batch_id IS NOT NULL"),
            )
        })
        .expect("analytics is configured");

    assert_eq!(
        (local, imported),
        (1, 2),
        "the analytics store cannot tell this station's recordings from another \
         station's import, so every location- and hour-based analytic reads the \
         merged history as one site"
    );

    // And the clock reconciliation reached the analytics copy too: 06:30 at the
    // source, six hours behind, is 12:30 here.
    let shifted: i64 = state
        .with_analytics(|adb| {
            adb.conn()
                .query_row(
                    "SELECT COUNT(*) FROM detections WHERE Time = '12:30:00'",
                    [],
                    |r| r.get::<_, i64>(0),
                )
                .expect("count")
        })
        .expect("analytics is configured");
    assert_eq!(
        shifted, 2,
        "the imported hours are still on the source's clock"
    );
}

// ---------------------------------------------------------------------------
// Effort-corrected abundance
// ---------------------------------------------------------------------------

/// The effort-corrected abundance query must return an actual rate.
///
/// It shipped for months joining a table called `recordings` that existed only
/// in the phenology module's own tests — so on a real station it had no
/// denominator, silently returned NULL rates, and looked like a feature nobody
/// needed. This drives it against the station's own `recording_effort`
/// (migration 27) end to end: SQLite → DuckDB → query.
///
/// The correction is not cosmetic. A solar recording window is six hours longer
/// in June than December; a week of downtime removes a week of listening. Both
/// move the raw count without moving a single bird, so a between-season or
/// between-year comparison of raw counts measures the station as much as the
/// birds.
#[test]
fn effort_corrected_abundance_returns_a_rate() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("birds.db");
    let year: i64 = {
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        birdnet_db::migration::migrate(&conn).unwrap();
        let today: String = conn
            .query_row("SELECT date('now','localtime')", [], |r| r.get(0))
            .unwrap();
        // Ten detections on one day, against four hours of listening.
        for i in 0..10 {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES (?1, ?2, 'Turdus merula', 'Eurasian Blackbird', 0.9)",
                rusqlite::params![&today, format!("06:{i:02}:00")],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO recording_effort (date, source, seconds) VALUES (?1, 'local', 14400.0)",
            rusqlite::params![&today],
        )
        .unwrap();
        today[..4].parse().unwrap()
    };

    let state = AppState::new_with_analytics(db_path, &dir.path().join("analytics.duckdb"))
        .expect("analytics state opens");
    state
        .resync_analytics_full()
        .expect("analytics is configured")
        .expect("resync");

    let params = birdnet_behavioral::phenology::AbundanceParams {
        species: None,
        year: u32::try_from(year).unwrap(),
        min_weekly_count: 1,
    };
    let sql = birdnet_behavioral::phenology::effort_corrected_abundance_sql(&params);

    let (raw, hours, rate): (i64, f64, f64) = state
        .with_analytics(|adb| {
            adb.conn()
                .query_row(&sql, [], |r| {
                    Ok((
                        r.get::<_, i64>(2)?,
                        r.get::<_, f64>(3)?,
                        r.get::<_, f64>(4)?,
                    ))
                })
                .expect("the effort-corrected query must run against the real schema")
        })
        .expect("analytics is configured");

    assert_eq!(raw, 10, "ten detections");
    assert!(
        (hours - 4.0).abs() < 1e-6,
        "four hours of listening, got {hours}"
    );
    assert!(
        (rate - 2.5).abs() < 1e-6,
        "ten detections over four hours is 2.5/hour, got {rate} — a NULL here is \
         the old failure: the query had no denominator on any real station"
    );
}
