//! A delete names one detection, and takes that detection's clip with it.
//!
//! Two defects met here. Every page and API write keyed a detection by date,
//! time and species — which names one row *per source* that heard the bird in
//! that second — so deleting the microphone's detection deleted the camera's
//! too, locked or not. And deleting a detection left its clip in the
//! recordings directory, where `GET /api/v2/recordings` still listed it and
//! anyone who could reach the station could still download it.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const MIC: &str = "Robin-90-2026-05-01-birdnet-06:10:00.wav";
const CAM: &str = "Robin-88-2026-05-01-birdnet-cam2-06:10:00.wav";

/// Two sources, one bird, one second; `lock_cam` locks the camera's row.
fn station(lock_cam: bool) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().unwrap();
    for f in [MIC, CAM] {
        std::fs::write(dir.path().join(f), b"RIFF....WAVE").unwrap();
    }
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    for (file, locked) in [(MIC, false), (CAM, lock_cam)] {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name, is_locked)
             VALUES ('2026-05-01', '06:10:00', 'Erithacus rubecula', 'European Robin', 0.9, ?1, ?2)",
            rusqlite::params![file, i64::from(locked)],
        )
        .unwrap();
    }
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
        .with_recording_dir(dir.path().to_path_buf());
    (dir, state)
}

async fn post(state: &AppState, uri: &str, body: &str) -> StatusCode {
    build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(uri)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn get(state: &AppState, uri: &str) -> (StatusCode, String) {
    let res = build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

fn rows(state: &AppState) -> Vec<String> {
    state.with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT File_Name FROM detections ORDER BY File_Name")
            .unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    })
}

fn form(file: Option<&str>) -> String {
    let mut f = "date=2026-05-01&time=06%3A10%3A00&sci_name=Erithacus+rubecula".to_owned();
    if let Some(file) = file {
        f.push_str("&file_name=");
        f.push_str(&file.replace(':', "%3A"));
    }
    f
}

#[tokio::test]
async fn deleting_one_sources_detection_leaves_the_others() {
    for uri in ["/pages/recordings-delete", "/pages/today-delete"] {
        let (_dir, state) = station(false);
        assert_eq!(post(&state, uri, &form(Some(MIC))).await, StatusCode::OK);
        assert_eq!(rows(&state), vec![CAM.to_owned()], "{uri}");
    }
}

#[tokio::test]
async fn a_delete_that_names_no_clip_spares_a_locked_sibling() {
    // An older caller that sends no clip: the unlocked row goes, the locked
    // one stays.
    let (_dir, state) = station(true);
    post(&state, "/pages/recordings-delete", &form(None)).await;
    assert_eq!(rows(&state), vec![CAM.to_owned()]);
}

#[tokio::test]
async fn the_deleted_detections_clip_is_no_longer_listed_or_served() {
    let (dir, state) = station(false);
    let (_, listing) = get(&state, "/api/v2/recordings").await;
    assert!(
        listing.contains(MIC),
        "fixture: the clip is listed before: {listing}"
    );

    post(&state, "/pages/recordings-delete", &form(Some(MIC))).await;

    assert!(!dir.path().join(MIC).exists(), "the clip file is gone");
    let (_, listing) = get(&state, "/api/v2/recordings").await;
    assert!(!listing.contains(MIC), "{listing}");
    let (status, _) = get(&state, &format!("/api/v2/recordings/{MIC}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // The other source's clip is untouched.
    assert!(dir.path().join(CAM).exists());
}

#[tokio::test]
async fn a_clip_another_detection_still_names_is_kept() {
    // Two species heard in one segment share its clip.
    let (dir, state) = station(false);
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
             VALUES ('2026-05-01', '06:10:00', 'Parus major', 'Great Tit', 0.8, ?1)",
            [MIC],
        )
        .unwrap()
    });
    post(&state, "/pages/recordings-delete", &form(Some(MIC))).await;
    assert!(
        dir.path().join(MIC).exists(),
        "the Great Tit still needs it"
    );
}

#[tokio::test]
async fn locking_one_sources_clip_leaves_the_others_unlocked() {
    let (_dir, state) = station(false);
    assert_eq!(
        post(&state, "/pages/recordings-lock", &form(Some(CAM))).await,
        StatusCode::OK
    );
    let locked: Vec<(String, i64)> = state.with_db(|conn| {
        let mut stmt = conn
            .prepare("SELECT File_Name, is_locked FROM detections ORDER BY File_Name")
            .unwrap();
        stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    });
    assert_eq!(locked, vec![(CAM.to_owned(), 1), (MIC.to_owned(), 0)]);
}
