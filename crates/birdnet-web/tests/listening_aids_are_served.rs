//! The listening aids (boost, rumble cut, compressor: `static/listen-fx.js`)
//! reach the pages that play audio, under the strict content-security policy.
//!
//! The Web Audio behaviour itself is measured in a real browser by
//! `tools/visual-qa/listen-fx.mjs`; this file holds what can be held without
//! one: the script and its stylesheet are served, every full page and the
//! standalone player load the script as a *real* script (the security layer
//! escapes any `<script` it did not seal, which left the first version of the
//! player page printing the tag as text), the controls have somewhere to be
//! built, and the files use no inline code, `eval` or raw colour.

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

async fn get(state: &AppState, uri: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let res = build_router(state.clone())
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .expect("response");
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = axum::body::to_bytes(res.into_body(), 1 << 22)
        .await
        .expect("body");
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

#[tokio::test]
async fn the_script_and_its_stylesheet_are_served() {
    let (_d, state) = station();
    let (status, headers, js) = get(&state, "/static/listen-fx.js").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .contains("javascript")
    );
    assert!(js.contains("buildChain") && js.contains("createDynamicsCompressor"));
    let (status, headers, css) = get(&state, "/static/css/listen-fx.css").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()
            .unwrap()
            .starts_with("text/css")
    );
    assert!(css.contains(".lfx"));
}

#[tokio::test]
async fn the_files_use_what_the_policy_allows() {
    let (_d, state) = station();
    let (_, _, js) = get(&state, "/static/listen-fx.js").await;
    for banned in [
        "eval(",
        "new Function",
        "innerHTML = '<script",
        "document.write",
    ] {
        assert!(!js.contains(banned), "listen-fx.js uses {banned}");
    }
    let (_, _, css) = get(&state, "/static/css/listen-fx.css").await;
    // Colour comes from the design tokens (app.css), never a literal.
    for literal in ["#", "rgb(", "rgba(", "hsl(", "oklch("] {
        assert!(
            !css.contains(literal),
            "listen-fx.css has a raw colour: {literal}"
        );
    }
    assert!(css.contains("var(--"));
}

#[tokio::test]
async fn full_pages_load_the_script_and_the_station_page_has_the_controls() {
    let (_d, state) = station();
    // `/` sends a station with no detections to the setup page; the clips
    // page is a full layout page either way.
    let (status, _, clips) = get(&state, "/recordings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        clips.contains(r#"src="/static/listen-fx.js"#),
        "layout.html must load it"
    );
    let (status, _, settings) = get(&state, "/station/settings").await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        settings.contains("data-listen-fx"),
        "the display card needs a mount point"
    );
}

#[tokio::test]
async fn the_standalone_player_loads_it_as_a_script_not_as_text() {
    let (_d, state) = station();
    let (status, _, page) = get(&state, "/player/clip.wav").await;
    assert_eq!(status, StatusCode::OK);
    // Sealed scripts carry the request's nonce, so the tag is `<script nonce=… src=…>`.
    assert!(
        page.contains(r#"src="/static/listen-fx.js"#) && !page.contains("&lt;script"),
        "the player page must load the script as a script"
    );
    assert!(
        !page.contains("&lt;script src=\"/static/listen-fx.js"),
        "the security layer escaped the tag: it must be sealed"
    );
    assert!(
        page.contains("data-listen-fx"),
        "the player needs a mount point"
    );
}
