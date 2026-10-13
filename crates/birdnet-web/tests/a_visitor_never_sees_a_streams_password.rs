//! A page anyone may read names an unlabelled stream by its host, never by
//! its URL.
//!
//! The Live view's source picker and the Today page's first-run checklist fell
//! back to a source's `device_id` when it had no label, and a stream's
//! `device_id` is its URL — `user:pass@` and all. Both pages are open to
//! visitors on a password-protected station, so an anonymous request was
//! handed the camera's password. Since G-7 a URL can also carry an SRT
//! passphrase in its query.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_db::audio_sources::{AudioSourceStore as _, NewAudioSource, SourceKind};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const CAM_PASS: &str = "campass-41d2";
const SRT_PASS: &str = "srtpass-77c1";

/// A configured station with an admin password and two unlabelled streams.
fn station() -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let hash = birdnet_db::accounts::hash_password("admin-password-1").expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        [&hash],
    )
    .expect("admin password");
    conn.execute_batch("INSERT INTO settings (key, value) VALUES ('onboarding_complete', 'true');")
        .expect("onboarded");
    for (id, url) in [
        (
            "cam1",
            format!("rtsp://viewer:{CAM_PASS}@camera.local/stream"),
        ),
        (
            "srt1",
            format!("srt://srt.local:8890?streamid=read:garden&passphrase={SRT_PASS}"),
        ),
    ] {
        conn.insert(&NewAudioSource::defaults(id, SourceKind::Rtsp, url))
            .expect("source");
    }
    build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
}

async fn anonymous_get(app: &axum::Router, uri: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .header("host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

/// Observed failing with the `device_id` fallback restored in `listen.rs`:
/// the Live view printed `rtsp://viewer:campass-41d2@camera.local/stream`.
#[tokio::test]
async fn the_live_view_names_an_unlabelled_stream_by_its_host() {
    let (status, page) = anonymous_get(&station(), "/recordings?view=live").await;
    assert_eq!(status, StatusCode::OK, "the Live view is public");
    assert!(!page.contains(CAM_PASS), "the camera's password was served");
    assert!(!page.contains(SRT_PASS), "the SRT passphrase was served");
    // The counterpart: both sources are still there to choose between.
    assert!(page.contains("camera.local"), "{page}");
    assert!(page.contains("srt.local:8890"), "{page}");
}

/// Observed failing with the `device_id` fallback restored in `today.rs`.
#[tokio::test]
async fn the_first_run_checklist_names_an_unlabelled_stream_by_its_host() {
    let (status, page) = anonymous_get(&station(), "/").await;
    assert_eq!(status, StatusCode::OK, "Today is public");
    assert!(
        page.contains("Network stream · camera.local"),
        "the checklist names the first source"
    );
    assert!(!page.contains(CAM_PASS), "the camera's password was served");
}
