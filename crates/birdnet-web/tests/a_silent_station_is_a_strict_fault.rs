//! `/api/v2/health?strict=1` reads the detection deadman's verdict (AD-4).
//!
//! `detection_silence_secs` was on the body and in no status code, so a week
//! of silence left even the strict endpoint green: the one signal built to
//! prove the audio-to-insert chain alive reached the notifier and nothing a
//! monitor polls. The deadman now publishes its verdict, and `tripped` is a
//! strict fault — and only a strict one, since a silent station is exactly
//! the station a container supervisor must not restart in a loop.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    // A stopped daemon is a strict fault of its own; mark it running so the
    // deadman's verdict is the only thing under test.
    state
        .detection_status_flag()
        .store(true, std::sync::atomic::Ordering::Relaxed);
    state
}

async fn health(state: &AppState, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = birdnet_web::server::build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn a_tripped_deadman_is_a_strict_fault_and_nothing_else() {
    let state = station();
    let (status, body) = health(&state, "/api/v2/health?strict=1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["detection_deadman"], "off",
        "no deadman has reported yet"
    );

    state.metrics().set_detection_deadman(Some(false));
    let (status, body) = health(&state, "/api/v2/health?strict=1").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["detection_deadman"], "ok");

    state.metrics().set_detection_deadman(Some(true));
    let (status, body) = health(&state, "/api/v2/health?strict=1").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["detection_deadman"], "tripped");
    assert_eq!(body["status"], "degraded");

    let (status, body) = health(&state, "/api/v2/health").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "not strict: the supervisor must not restart a silent station: {body}"
    );
    assert_eq!(
        body["detection_deadman"], "tripped",
        "the verdict is still on the body"
    );
}
