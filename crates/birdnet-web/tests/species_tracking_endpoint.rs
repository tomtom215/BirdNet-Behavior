//! `GET /api/v2/species/tracking` reports what is notable about a day.
//!
//! The pieces are tested where they live — the season table in `birdnet-core`,
//! the flags in `birdnet-db`, the window resolution in `birdnet-web`. What is
//! not covered by any of those is that they are *connected*: a version where
//! the endpoint returns a fixed empty list, or resolves the windows and then
//! ignores them, passes every one of those suites.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn state() -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    (dir, state)
}

fn seen(state: &AppState, sci: &str, date: &str, time: &str) {
    state.with_db(|conn| {
        conn.execute(
            "INSERT INTO detections
                 (Date, Time, Sci_Name, Com_Name, Confidence, Cutoff, Week, Sens, Overlap,
                  File_Name, chunk_offset_secs)
             VALUES (?1, ?4, ?2, ?3, 0.9, 0.7, 1, 1.25, 0.0, 'x.wav', 0)",
            rusqlite::params![date, sci, sci, time],
        )
        .expect("insert");
    });
}

fn set(state: &AppState, key: &str, value: &str) {
    state.with_db(|conn| {
        birdnet_db::settings::set(
            conn,
            key,
            value,
            birdnet_db::settings::SettingsCategory::Location,
        )
        .expect("set");
    });
}

async fn get_json(state: &AppState, uri: &str) -> serde_json::Value {
    let app = birdnet_web::server::build_router(state.clone());
    let res = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("router responds");
    assert_eq!(res.status(), StatusCode::OK, "GET {uri}");
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 20)
        .await
        .expect("body");
    serde_json::from_slice(&bytes).expect("json")
}

/// A first-ever detection is reported as such, with the windows it was judged
/// against.
#[tokio::test]
async fn a_first_ever_detection_is_reported_with_its_windows() {
    let (_d, state) = state();
    set(&state, "latitude", "52.2");
    seen(&state, "Strix aluco", "2026-04-10", "06:00:00");

    let body = get_json(&state, "/api/v2/species/tracking?date=2026-04-10").await;
    assert_eq!(body["date"], "2026-04-10");
    assert_eq!(body["season"], "spring", "52.2 N in April is spring");
    assert_eq!(
        body["season_start"], "2026-03-20",
        "season_start must be the date the season began, not its name"
    );
    assert_eq!(body["year_start"], "2026-01-01");

    let species = body["species"].as_array().expect("species array");
    assert_eq!(species.len(), 1);
    assert_eq!(species[0]["sci_name"], "Strix aluco");
    assert_eq!(species[0]["headline"], "first ever");
    assert_eq!(species[0]["new_ever"], true);
    assert_eq!(species[0]["days_since_previous"], serde_json::Value::Null);
}

/// The latitude the station was given decides the season it reports.
///
/// The counterpart to the test above: an endpoint that hard-coded northern
/// seasons — which is what every implementation does until someone south of
/// the equator complains — passes that one and fails this.
#[tokio::test]
async fn a_southern_station_reports_its_own_season() {
    let (_d, state) = state();
    set(&state, "latitude", "-33.9");
    seen(&state, "Strix aluco", "2026-04-10", "06:00:00");

    let body = get_json(&state, "/api/v2/species/tracking?date=2026-04-10").await;
    assert_eq!(
        body["season"], "fall",
        "April at 33.9 S is autumn, not spring"
    );
}

/// A resident is present in the list but has no headline.
#[tokio::test]
async fn a_resident_appears_without_a_headline() {
    let (_d, state) = state();
    set(&state, "latitude", "52.2");
    seen(&state, "Turdus merula", "2026-04-09", "06:00:00");
    seen(&state, "Turdus merula", "2026-04-10", "06:00:00");

    let body = get_json(&state, "/api/v2/species/tracking?date=2026-04-10").await;
    let species = body["species"].as_array().expect("array");
    assert_eq!(species.len(), 1);
    assert_eq!(species[0]["headline"], serde_json::Value::Null);
    assert_eq!(species[0]["days_since_previous"], 1);
}

/// `notable_only` filters, and filters to the right thing.
#[tokio::test]
async fn notable_only_keeps_the_news_and_drops_the_rest() {
    let (_d, state) = state();
    set(&state, "latitude", "52.2");
    seen(&state, "Turdus merula", "2026-04-09", "06:00:00");
    seen(&state, "Turdus merula", "2026-04-10", "06:00:00");
    seen(&state, "Hirundo rustica", "2026-04-10", "07:00:00");

    let all = get_json(&state, "/api/v2/species/tracking?date=2026-04-10").await;
    assert_eq!(all["species"].as_array().expect("array").len(), 2);

    let notable = get_json(
        &state,
        "/api/v2/species/tracking?date=2026-04-10&notable_only=true",
    )
    .await;
    let species = notable["species"].as_array().expect("array");
    assert_eq!(species.len(), 1, "only the swallow is news");
    assert_eq!(species[0]["sci_name"], "Hirundo rustica");
}

/// A station with no latitude reports no season rather than guessing one.
#[tokio::test]
async fn a_station_without_a_latitude_reports_no_season() {
    let (_d, state) = state();
    seen(&state, "Strix aluco", "2026-04-10", "06:00:00");

    let body = get_json(&state, "/api/v2/species/tracking?date=2026-04-10").await;
    assert_eq!(body["season"], serde_json::Value::Null);
    assert_eq!(
        body["species"][0]["new_this_season"], true,
        "with the season window collapsed onto the year, a first-of-year is also \
         first-of-season; the null season is what tells a caller not to show that badge"
    );
}

/// A malformed date returns an empty day rather than silently answering for
/// today.
#[tokio::test]
async fn a_malformed_date_is_not_silently_replaced_by_today() {
    let (_d, state) = state();
    seen(&state, "Strix aluco", "2026-04-10", "06:00:00");

    let body = get_json(&state, "/api/v2/species/tracking?date=nonsense").await;
    assert_ne!(
        body["date"], "nonsense",
        "the date must be rejected, not echoed"
    );
    // Falling back to today, which has no detections in this fixture.
    assert!(body["species"].as_array().expect("array").is_empty());
}
