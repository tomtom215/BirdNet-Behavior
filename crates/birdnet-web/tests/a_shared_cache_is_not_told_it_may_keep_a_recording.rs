//! A shared cache is not told it may keep a station's recordings or feeds.
//!
//! Recordings went out with `Cache-Control: public, max-age=86400`, and the
//! RSS/iCal feeds with `public` too, on every station — including a private
//! one, where they are served only to a signed-in session. `public` licenses
//! any shared cache between the station and the reader (a household proxy, a
//! CDN in front of the station) to store the response and hand it to the next
//! person who asks. `private` keeps the browser's own cache and nothing else.
//! Share pages and static assets stay `public`; they are public by design.

use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

async fn cache_control(state: &AppState, uri: &str) -> String {
    let resp = build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{uri}: {}", resp.status());
    resp.headers()
        .get("cache-control")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

#[tokio::test]
async fn recordings_and_feeds_are_private_to_the_reader() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("clip.wav"), b"RIFF0000WAVE").unwrap();
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let st = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
        .with_recording_dir(dir.path().to_path_buf());
    for uri in [
        "/api/v2/recordings/clip.wav",
        "/feeds/today.rss",
        "/feeds/rare.rss",
        "/feeds/rare.ics",
    ] {
        let cc = cache_control(&st, uri).await;
        assert!(!cc.contains("public"), "{uri}: {cc}");
        assert!(cc.contains("private"), "{uri}: {cc}");
    }
}
