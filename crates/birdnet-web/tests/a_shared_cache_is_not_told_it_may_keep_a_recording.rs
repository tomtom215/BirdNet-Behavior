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

/// One second of 16-bit mono silence at 48 kHz: the smallest file the
/// spectrogram renderer will draw.
fn silent_wav() -> Vec<u8> {
    let rate: u32 = 48_000;
    let data_len: u32 = rate * 2;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    w.resize(44 + data_len as usize, 0);
    w
}

/// The spectrogram of a recording is as private as the recording: it went out
/// `public, max-age=3600` after the audio itself had been made `private`.
#[tokio::test]
async fn a_recordings_spectrogram_is_private_too() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("clip.wav"), silent_wav()).unwrap();
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let st = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
        .with_recording_dir(dir.path().to_path_buf());
    let cc = cache_control(&st, "/api/v2/spectrogram/clip.wav").await;
    assert!(!cc.contains("public"), "{cc}");
    assert!(cc.contains("private"), "{cc}");
}
