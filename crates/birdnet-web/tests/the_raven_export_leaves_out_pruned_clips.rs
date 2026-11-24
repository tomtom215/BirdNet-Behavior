//! A Raven selection table names only clips that are still on the station
//! (RES-09).
//!
//! Retention and the per-species cap delete a clip's audio and stamp its
//! detection `Clip_Pruned_At`; the row stays. The combined Raven table read
//! every row with a clip name and a length, so it listed selections in files
//! that no longer existed: Raven Pro fails to open such a multi-file table, or
//! the analyst silently loses the selections. A pruned row is now left out,
//! and the response says how many were (`X-Omitted-Pruned-Clips`), so a short
//! table is not mistaken for a quiet season.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    conn.execute_batch(
        "INSERT INTO detections
             (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, Duration_Secs,
              clip_offset_secs, detection_secs, Clip_Pruned_At)
         VALUES
             ('2026-09-08', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.9,
              'kept.wav', 6.0, 1.5, 3.0, NULL),
             ('2026-09-07', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.9,
              'pruned.wav', 6.0, 1.5, 3.0, 1788000000);",
    )
    .expect("seed");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn get(state: &AppState, path: &str) -> (StatusCode, String, Option<String>) {
    let resp = build_router(state.clone())
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let omitted = resp
        .headers()
        .get("x-omitted-pruned-clips")
        .map(|v| v.to_str().unwrap().to_owned());
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap(), omitted)
}

/// The probe: a row whose clip was pruned gets no selection, the row whose
/// clip is there keeps its own (the counterpart), and the omission is counted.
#[tokio::test]
async fn a_pruned_clip_gets_no_selection_and_the_omission_is_counted() {
    let state = station();
    let (status, body, omitted) = get(&state, "/api/v2/detections/export/raven").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        body.contains("\tkept.wav\t"),
        "a clip still on disk keeps its selection: {body}"
    );
    assert!(
        !body.contains("pruned.wav"),
        "a selection names a clip retention already deleted: {body}"
    );
    assert_eq!(omitted.as_deref(), Some("1"), "the omission is counted");
}

/// The per-clip table of a pruned clip has nothing to point Raven at; it is
/// refused as gone rather than served as a selection in a missing file. A
/// clip that is there still has its table.
#[tokio::test]
async fn a_pruned_clip_has_no_table_of_its_own() {
    let state = station();
    let (status, body, _) = get(&state, "/api/v2/recordings/pruned.wav/raven.txt").await;
    assert_eq!(status, StatusCode::GONE, "{body}");
    let (status, body, _) = get(&state, "/api/v2/recordings/pruned.wav/labels.txt").await;
    assert_eq!(status, StatusCode::GONE, "{body}");
    let (status, body, _) = get(&state, "/api/v2/recordings/kept.wav/raven.txt").await;
    assert_eq!(status, StatusCode::OK, "{body}");
}
