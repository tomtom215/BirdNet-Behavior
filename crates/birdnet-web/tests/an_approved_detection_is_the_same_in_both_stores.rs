//! Approving a quarantined detection writes the same row to `SQLite` and to
//! the `DuckDB` analytics copy (N06-01).
//!
//! `approve_quarantine` copies the quarantined row's source, cutoff,
//! sensitivity and overlap into `detections`. The mirror into `DuckDB` wrote
//! all four as NULL, so the analytics copy of an approved detection said it
//! was heard by no source, under no settings, while `SQLite` said otherwise;
//! and the incremental sync never revisits a back-dated row to repair it.

#![cfg(feature = "analytics")]

use birdnet_web::state::AppState;

type Provenance = (Option<String>, Option<f64>, Option<f64>, Option<f64>);

#[test]
fn an_approved_detection_keeps_its_source_and_settings_in_both_stores() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("birds.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    drop(conn);
    let state = AppState::new_with_analytics(db_path, &dir.path().join("analytics.duckdb"))
        .expect("analytics state opens");

    state
        .with_db(|conn| {
            birdnet_db::sqlite::insert_quarantine(
                conn,
                &birdnet_db::sqlite::QuarantineRecord {
                    date: "2026-05-01",
                    time: "05:00:00",
                    sci_name: "Strix aluco",
                    com_name: "Tawny Owl",
                    confidence: 0.55,
                    sf_probability: None,
                    reason: birdnet_db::sqlite::QuarantineReason::LowConfidence,
                    file_name: Some("owl.wav"),
                    lat: None,
                    lon: None,
                    week: None,
                    run_id: None,
                    cutoff: Some(0.7),
                    sensitivity: Some(1.25),
                    overlap: Some(0.5),
                    source: Some("garden"),
                },
            )
        })
        .expect("quarantine insert");
    let id: i64 = state
        .with_db(|conn| conn.query_row("SELECT id FROM quarantine", [], |r| r.get(0)))
        .unwrap();

    assert!(state.approve_quarantine(id).expect("approve"));

    let sqlite: Provenance = state
        .with_db(|conn| {
            conn.query_row(
                "SELECT Source, Cutoff, Sens, Overlap FROM detections",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
        })
        .unwrap();
    assert_eq!(
        sqlite,
        (Some("garden".to_owned()), Some(0.7), Some(1.25), Some(0.5)),
        "precondition: SQLite copies the quarantined row's provenance"
    );
    let duckdb: Provenance = state
        .with_analytics(|adb| {
            adb.conn().query_row(
                "SELECT Source, Cutoff, Sens, Overlap FROM detections",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
        })
        .expect("analytics is configured")
        .expect("read the mirror");
    assert_eq!(duckdb, sqlite, "the analytics copy must hold the same row");
}
