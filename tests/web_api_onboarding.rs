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

/// The whole point of the step: a first-run operator is *asked* for the
/// threshold instead of silently inheriting one they never see. Before this
/// existed the wizard never mentioned confidence at all, so a station that
/// wanted stricter (or looser) detection had to find Settings → Detection
/// unprompted.
#[tokio::test]
async fn wizard_prompts_for_the_confidence_threshold() {
    let resp = build_router(fresh_state())
        .oneshot(
            Request::builder()
                .uri("/onboarding")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let html = String::from_utf8(body.to_vec()).unwrap();

    assert!(
        html.contains(r#"name="confidence_threshold""#),
        "the wizard must submit a confidence_threshold field"
    );
    let expected_default = format!(
        r#"id="ob-conf" value="{}""#,
        birdnet_core::config::DEFAULT_CONFIDENCE_THRESHOLD
    );
    assert!(
        html.contains(&expected_default),
        "the pre-selected default must be DEFAULT_CONFIDENCE_THRESHOLD ({expected_default}) \
         — the same value the daemon enforces and Settings → Detection advertises"
    );
    assert!(
        html.contains(r#"data-value="0.75""#),
        "the recommended card must carry the shared default"
    );
    for preset in ["0.9", "0.6", "0.4"] {
        assert!(
            html.contains(&format!(r#"data-value="{preset}""#)),
            "missing the {preset} preset"
        );
    }
    assert!(
        html.contains("Step <span id=\"ob-cur\">1</span> of 6"),
        "the step counter must match the number of steps actually rendered"
    );
    // `<section` anchors this to the step bodies — a bare `class="ob-step`
    // also matches the `ob-stepper` container that holds the numbered pips.
    let rendered_steps = html.matches(r#"<section class="ob-step"#).count();
    assert_eq!(
        rendered_steps, 6,
        "6 step sections expected, found {rendered_steps}"
    );
    let pips = html.matches(r#"data-pip=""#).count();
    assert_eq!(
        pips, rendered_steps,
        "the stepper must show one pip per step, else the wizard skips a dot"
    );
}

#[tokio::test]
async fn save_persists_the_chosen_confidence_threshold() {
    let state = fresh_state();
    let resp = post_form(
        build_router(state.clone()),
        "/onboarding/save",
        "latitude=51.5&longitude=-0.12&confidence_threshold=0.85",
    )
    .await;
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);

    let conf = state.with_db(|c| birdnet_db::settings::get(c, "confidence_threshold").unwrap());
    assert_eq!(
        conf, "0.85",
        "the wizard's choice must land in the settings table the overlay reads"
    );
}

/// Counter-test: the field is a plain form value, and an out-of-range
/// `CONFIDENCE` is a *fatal* doctor error (`ExecStartPre` exit 2). A crafted
/// POST must not be able to leave the station unable to start.
#[tokio::test]
async fn save_rejects_a_confidence_the_daemon_would_refuse_to_start_on() {
    for bad in ["70", "-0.5", "abc", "1.5", ""] {
        let state = fresh_state();
        let body = format!("confidence_threshold={bad}");
        let resp = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/onboarding/save")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);

        let stored = state.with_db(|c| birdnet_db::settings::get(c, "confidence_threshold"));
        assert!(
            stored.is_err(),
            "confidence_threshold={bad:?} must not be persisted, got {stored:?}"
        );
        // The rejection must not derail the rest of the wizard.
        let complete =
            state.with_db(|c| birdnet_db::settings::get(c, "onboarding_complete").unwrap());
        assert_eq!(complete, "true");
    }
}

/// The in-range boundaries the daemon does accept must still be storable —
/// the guard rejects the unusable, not the unusual.
#[tokio::test]
async fn save_accepts_in_range_boundary_confidences() {
    for good in ["0", "1", "0.05", "1.0"] {
        let state = fresh_state();
        let body = format!("confidence_threshold={good}");
        let resp = build_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/onboarding/save")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER);
        let stored =
            state.with_db(|c| birdnet_db::settings::get(c, "confidence_threshold").unwrap());
        assert_eq!(stored, good);
    }
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
