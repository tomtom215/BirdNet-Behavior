//! Integration tests for the first-run onboarding flow (G-09).
//!
//! Covers the first-boot redirect (`GET /` → `/onboarding` only when the
//! station has no detections and isn't onboarded) and the persistence of
//! `POST /onboarding/save` (location/timezone settings + the completion flag).

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use rusqlite::Connection;
use tower::ServiceExt;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// Fresh station: schema applied, no detections, not onboarded.
fn fresh_state() -> AppState {
    let conn = Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

/// A station that already has a detection (so it is past first-run).
fn state_with_detection() -> AppState {
    let conn = Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            "2026-03-12",
            "06:30:00",
            "Turdus merula",
            "Eurasian Blackbird",
            0.9
        ],
    )
    .unwrap();
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn get_status(router: axum::Router, uri: &str) -> StatusCode {
    router
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

/// POST a urlencoded body (no Origin/Host header → passes the stateless,
/// origin-based CSRF guard, just like a same-origin browser submit).
async fn post_form(
    router: axum::Router,
    uri: &str,
    body: &'static str,
) -> axum::response::Response {
    router
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
}

#[tokio::test]
async fn fresh_station_redirects_to_onboarding() {
    let resp = build_router(fresh_state())
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers().get(header::LOCATION).unwrap(), "/onboarding");
}

#[tokio::test]
async fn station_with_detections_sees_dashboard() {
    assert_eq!(
        get_status(build_router(state_with_detection()), "/").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn onboarding_wizard_serves() {
    assert_eq!(
        get_status(build_router(fresh_state()), "/onboarding").await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn save_persists_location_and_marks_complete() {
    let state = fresh_state();

    let resp = post_form(
        build_router(state.clone()),
        "/onboarding/save",
        "latitude=51.5&longitude=-0.12&timezone=Europe/London&notification_mode=rare",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers().get(header::LOCATION).unwrap(), "/");

    let (lat, lon, tz, complete) = state.with_db(|c| {
        (
            birdnet_db::settings::get(c, "latitude").unwrap(),
            birdnet_db::settings::get(c, "longitude").unwrap(),
            birdnet_db::settings::get(c, "timezone").unwrap(),
            birdnet_db::settings::get(c, "onboarding_complete").unwrap(),
        )
    });
    assert_eq!(lat, "51.5");
    assert_eq!(lon, "-0.12");
    assert_eq!(tz, "Europe/London");
    assert_eq!(complete, "true");

    // Having onboarded, `/` now serves the dashboard instead of redirecting.
    assert_eq!(get_status(build_router(state), "/").await, StatusCode::OK);
}

#[tokio::test]
async fn save_with_no_fields_still_completes_without_writing_blanks() {
    let state = fresh_state();

    let resp = post_form(build_router(state.clone()), "/onboarding/save", "").await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    let complete = state.with_db(|c| birdnet_db::settings::get(c, "onboarding_complete").unwrap());
    assert_eq!(
        complete, "true",
        "clicking through must still mark complete"
    );

    // No empty latitude row was written.
    let lat = state.with_db(|c| birdnet_db::settings::get(c, "latitude"));
    assert!(
        lat.is_err(),
        "an empty submit must not persist a blank latitude"
    );
}
