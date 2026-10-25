//! The wizard's "Look up my approximate location" fills the latitude, the
//! longitude and a hidden time zone. A person who then types *different*
//! coordinates must not be saved with the zone of the old ones.
//!
//! # What was wrong
//!
//! The hidden `timezone` field outlived the edit of the coordinates it came
//! from: `50.0 / 5.0` was saved with `America/Los_Angeles`, and Diagnostics
//! then warned that the station's location is in Los Angeles and told the
//! person to run `sudo timedatectl set-timezone America/Los_Angeles`: wrong
//! advice, and a terminal.
//!
//! # What this gate holds
//!
//! The page carries, beside the zone, the coordinates it was looked up for
//! (`timezone_latitude`, `timezone_longitude`), and the save keeps the zone only
//! when the saved coordinates are those. Counterparts: the unchanged look-up is
//! kept, a comma decimal for the same point is the same point, and a request
//! with no marker (a hand-made POST, an older page) is judged by the zone alone
//! as it was.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

async fn post_save(state: &AppState, body: &str) -> StatusCode {
    build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/onboarding/save")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_owned()))
                .expect("request"),
        )
        .await
        .expect("response")
        .status()
}

fn setting(state: &AppState, key: &str) -> Option<String> {
    state.with_db(|conn| {
        conn.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
            r.get::<_, String>(0)
        })
        .ok()
    })
}

/// What the browser posts after a look-up that answered `34.0522, -118.2437`
/// (Los Angeles), the other fields as the wizard leaves them.
fn body(lat: &str, lon: &str, zone: &str, marker: (&str, &str)) -> String {
    format!(
        "latitude={lat}&longitude={lon}&timezone={zone}&timezone_latitude={}&timezone_longitude={}\
         &station_name=&database_lang=en&notification_mode=rare&confidence_threshold=0.7\
         &mic_choice=keep&mic_rtsp_url=&mic_manual_device=&password=&password_confirm=",
        marker.0, marker.1
    )
}

const LA: (&str, &str) = ("34.0522", "-118.2437");

#[tokio::test]
async fn coordinates_typed_after_a_look_up_do_not_keep_the_looked_up_zone() {
    let (_dir, state) = station();
    let status = post_save(&state, &body("50.0", "5.0", "America%2FLos_Angeles", LA)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(setting(&state, "latitude").as_deref(), Some("50.0"));
    assert_eq!(setting(&state, "longitude").as_deref(), Some("5.0"));
    assert_eq!(
        setting(&state, "timezone"),
        None,
        "the zone of Los Angeles was saved for a station at 50.0, 5.0"
    );
}

#[tokio::test]
async fn a_zone_with_no_saved_location_is_dropped_when_it_was_looked_up_for_one() {
    let (_dir, state) = station();
    post_save(&state, &body("", "", "America%2FLos_Angeles", LA)).await;
    assert_eq!(setting(&state, "timezone"), None);
}

#[tokio::test]
async fn the_unchanged_look_up_keeps_its_zone() {
    let (_dir, state) = station();
    post_save(
        &state,
        &body("34.0522", "-118.2437", "America%2FLos_Angeles", LA),
    )
    .await;
    assert_eq!(
        setting(&state, "timezone").as_deref(),
        Some("America/Los_Angeles"),
        "the counterpart: coordinates that are the look-up's keep its zone"
    );
}

#[tokio::test]
async fn the_same_point_written_with_a_comma_is_the_same_point() {
    let (_dir, state) = station();
    post_save(
        &state,
        &body("34%2C0522", "-118%2C2437", "America%2FLos_Angeles", LA),
    )
    .await;
    assert_eq!(
        setting(&state, "timezone").as_deref(),
        Some("America/Los_Angeles")
    );
}

#[tokio::test]
async fn a_zone_with_no_marker_is_judged_as_it_always_was() {
    let (_dir, state) = station();
    post_save(
        &state,
        "latitude=51.5&longitude=-0.12&timezone=Europe%2FLondon",
    )
    .await;
    assert_eq!(
        setting(&state, "timezone").as_deref(),
        Some("Europe/London"),
        "a request with no marker has nothing to be compared with"
    );
}
