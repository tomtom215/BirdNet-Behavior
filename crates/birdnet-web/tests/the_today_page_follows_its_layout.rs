//! The Today page is rendered in the stored layout, and the editor changes it.
//!
//! Checked on the page the router actually serves, by the position of each
//! block's own landmark in the document — so "the order changed" means the
//! DOM changed, which is what a screen reader and the Tab key follow.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// A landmark unique to each block.
const HERO: &str = r#"data-screen-label="Hero""#;
const NUDGE: &str = r#"id="today-nudge""#;
const DAYSTRIP: &str = r#"data-screen-label="Day strip""#;
const LOG: &str = r#"data-screen-label="Unified log""#;
const TOP: &str = r#"id="top-species""#;
const BEST: &str = r#"id="best-detections""#;
const STATION: &str = r#"id="station-line""#;

fn station(detections: bool) -> (tempfile::TempDir, AppState) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = AppState::new(dir.path().join("birds.db")).expect("state");
    state.with_db(|conn| {
        birdnet_db::settings::set(
            conn,
            "onboarding_complete",
            "true",
            birdnet_db::settings::SettingsCategory::General,
        )
        .expect("onboarded");
        if detections {
            conn.execute(
                "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
                 VALUES ('2026-05-01', '06:00:00', 'Turdus merula', 'Eurasian Blackbird', 0.9)",
                [],
            )
            .expect("seed");
        }
    });
    (dir, state)
}

fn store(state: &AppState, layout: &str) {
    state.with_db(|conn| {
        birdnet_db::settings::set(
            conn,
            "today_layout",
            layout,
            birdnet_db::settings::SettingsCategory::General,
        )
        .expect("store layout");
    });
}

async fn send(state: &AppState, method: &str, uri: &str) -> (StatusCode, String) {
    let res = build_router(state.clone())
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 4 << 20)
        .await
        .expect("body");
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

async fn today(state: &AppState) -> String {
    let (status, body) = send(state, "GET", "/").await;
    assert_eq!(status, StatusCode::OK);
    body
}

fn at(page: &str, landmark: &str) -> Option<usize> {
    page.find(landmark)
}

#[tokio::test]
async fn the_page_is_built_in_the_stored_order_without_the_hidden_blocks() {
    let (_dir, st) = station(true);
    store(&st, "daystrip,hero,-nudge,log,station,-best,top-species");
    let page = today(&st).await;

    let daystrip = at(&page, DAYSTRIP).expect("day strip shown");
    let hero = at(&page, HERO).expect("hero shown");
    assert!(daystrip < hero, "the day strip was moved above the hero");
    assert!(at(&page, NUDGE).is_none(), "the nudge is hidden");
    assert!(at(&page, BEST).is_none(), "best recordings are hidden");
    let station_line = at(&page, STATION).expect("station shown");
    let top = at(&page, TOP).expect("top species shown");
    assert!(
        station_line < top,
        "the station card was moved above top species"
    );
    assert!(
        at(&page, LOG).is_some_and(|log| hero < log),
        "the log stays below the top"
    );
}

/// The counterpart: with nothing stored the page is the template as written,
/// so the test above cannot pass on a renderer that reorders by itself.
#[tokio::test]
async fn with_no_layout_stored_every_block_is_in_its_place() {
    let (_dir, st) = station(true);
    let page = today(&st).await;
    let order: Vec<usize> = [HERO, NUDGE, DAYSTRIP, LOG, TOP, BEST, STATION]
        .iter()
        .map(|m| at(&page, m).unwrap_or_else(|| panic!("{m} missing")))
        .collect();
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
    assert!(!page.contains("<!--block:"), "the markers are not shipped");
    assert!(
        !page.contains("{{work2_solo}}"),
        "the placeholder is filled"
    );
}

/// A station with no detections keeps its setup checklist whatever is stored.
#[tokio::test]
async fn a_new_station_ignores_the_layout() {
    let (_dir, st) = station(false);
    store(&st, "-hero,-nudge,-daystrip");
    let page = today(&st).await;
    assert!(at(&page, HERO).is_some(), "the first-run hero is shown");
}

#[tokio::test]
async fn the_editor_saves_a_change_and_the_page_follows_it() {
    let (_dir, st) = station(true);
    let (status, body) = send(&st, "POST", "/admin/today-layout/hero/hide").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains(r#"id="set-today-layout" hx-swap-oob="true""#),
        "{body}"
    );
    assert!(
        body.contains("/admin/today-layout/hero/show"),
        "now offers Show: {body}"
    );

    let stored = st.with_db(|conn| birdnet_db::settings::get(conn, "today_layout"));
    assert_eq!(
        stored.expect("stored"),
        "-hero,nudge,daystrip,log,top-species,best,station"
    );
    assert!(
        at(&today(&st).await, HERO).is_none(),
        "the page lost its hero"
    );
}

/// An unknown block or action is answered with a toast (htmx would discard
/// a 4xx body) and stores nothing.
#[tokio::test]
async fn a_bad_request_changes_nothing() {
    let (_dir, st) = station(true);
    for uri in [
        "/admin/today-layout/video-embed/hide",
        "/admin/today-layout/hero/sideways",
    ] {
        let (status, _) = send(&st, "POST", uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
    }
    let stored = st.with_db(|conn| birdnet_db::settings::get(conn, "today_layout"));
    assert!(stored.is_err(), "nothing was written: {stored:?}");
}
