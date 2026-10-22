//! The Settings location form and the time zone the wizard recorded.
//!
//! The setup wizard's "look up my location" stores a time zone beside the
//! coordinates. Its form had a hidden field that went stale when the person
//! typed other coordinates (fixed there). The Settings form has no time-zone
//! field and no look-up button, so that exact defect cannot occur on it; the
//! same fault arrives by another road: the zone the wizard stored stays in the
//! settings table when the coordinates are changed here, and Diagnostics then
//! compares the host clock with a zone for somewhere else (50.0/5.0 beside
//! `America/Los_Angeles`).
//!
//! A move on this form clears the stored zone, because nothing on the form can
//! say what the new place's zone is. Counterparts: saving something else, or the
//! same coordinates, keeps it.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        for (k, v) in [
            ("latitude", "37.7749"),
            ("longitude", "-122.4194"),
            ("timezone", "America/Los_Angeles"),
        ] {
            birdnet_db::settings::set(conn, k, v, birdnet_db::settings::SettingsCategory::Location)
                .expect("setting");
        }
    });
    (dir, state)
}

async fn save(state: &AppState, fields: &[(&str, &str)]) -> String {
    let body = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish();
    let res = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/settings")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body))
                .unwrap(),
        )
        .await
        .expect("response");
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn zone(state: &AppState) -> String {
    state
        .with_db(|conn| birdnet_db::settings::get(conn, "timezone"))
        .unwrap_or_default()
}

/// Seen failing before the change: the zone stayed `America/Los_Angeles` beside
/// 50.0/5.0.
#[tokio::test]
async fn moving_the_station_clears_the_zone_recorded_for_the_old_place() {
    let (_dir, state) = station();
    let reply = save(&state, &[("latitude", "50.0"), ("longitude", "5.0")]).await;
    assert!(!reply.contains("Nothing was saved"), "{reply}");
    assert_eq!(
        zone(&state),
        "",
        "a zone for California stayed beside 50.0/5.0"
    );
    assert!(reply.contains("time zone recorded at setup"), "{reply}");
    // One coordinate is a move as well.
    let (_dir, state) = station();
    save(&state, &[("longitude", "5.0")]).await;
    assert_eq!(zone(&state), "");
}

#[tokio::test]
async fn saving_something_else_or_the_same_coordinates_keeps_the_zone() {
    let (_dir, state) = station();
    let reply = save(&state, &[("audio_format", "flac")]).await;
    assert_eq!(zone(&state), "America/Los_Angeles");
    assert!(!reply.contains("time zone recorded at setup"), "{reply}");
    // The same place typed the way a European keyboard types it.
    save(
        &state,
        &[("latitude", "37,7749"), ("longitude", "-122,4194")],
    )
    .await;
    assert_eq!(zone(&state), "America/Los_Angeles");
}
