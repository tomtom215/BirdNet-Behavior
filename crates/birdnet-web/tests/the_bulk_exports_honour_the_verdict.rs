//! `R-17`, the remainder: a detection a reviewer rejected must not leave the
//! station through the CSV, JSON or `BirdDB.txt` export.
//!
//! Migration 26 gave every row a `review_verdict` and the `detections_analytic`
//! view that hides `'rejected'`; the eBird export was moved onto that view in
//! `dad46d1`. The other three bulk exports still read the raw table, so the
//! one surface where a dataset leaves the station undid the reviewer's work.
//!
//! Observed failing against the shipped tree: all three bodies contained
//! `Cuculus canorus`, the rejected row.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

fn station_with_a_rejected_detection() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO detections
             (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
              File_Name, chunk_offset_secs, review_verdict)
         VALUES
             ('2026-09-07', '06:00:00', 'Turdus merula', 'Eurasian Blackbird',
              0.91, 0.7, 36, 1.25, 0.0, 'kept.wav', 0, NULL),
             ('2026-09-07', '06:10:00', 'Cuculus canorus', 'Common Cuckoo',
              0.88, 0.7, 36, 1.25, 0.0, 'rejected.wav', 0, 'rejected');",
    )
    .expect("seed");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn get(state: &AppState, path: &str) -> (StatusCode, String) {
    let resp = build_router(state.clone())
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn a_rejected_detection_is_in_none_of_the_bulk_exports() {
    let state = station_with_a_rejected_detection();
    for path in [
        "/api/v2/detections/export",
        "/api/v2/detections/export?format=json",
        "/api/v2/detections/export/birddb",
    ] {
        let (status, body) = get(&state, path).await;
        assert_eq!(status, StatusCode::OK, "{path}: {body}");
        assert!(
            !body.contains("Cuculus canorus"),
            "{path} exported a detection the reviewer rejected:\n{body}"
        );
        // The counterpart: an export that dropped everything would pass the
        // assertion above and be useless.
        assert!(
            body.contains("Turdus merula"),
            "{path} lost the detection that was kept:\n{body}"
        );
    }
}

/// The date filter still applies on the view, and a date range that excludes
/// the kept row returns an empty export rather than the rejected one.
#[tokio::test]
async fn the_date_filter_composes_with_the_verdict() {
    let state = station_with_a_rejected_detection();
    let (status, body) = get(&state, "/api/v2/detections/export?from=2026-09-08").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(!body.contains("Turdus merula"), "{body}");
    assert!(!body.contains("Cuculus canorus"), "{body}");
    let (status, body) = get(
        &state,
        "/api/v2/detections/export?format=json&from=2026-09-07&to=2026-09-07",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(json["total"], 1, "{body}");
}
