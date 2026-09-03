//! The station served from under a reverse-proxy prefix, end to end.
//!
//! # Why this is a separate test binary
//!
//! The base path is a process-lifetime constant behind a `OnceLock` — there is
//! exactly one prefix per running station, and making it re-settable would be
//! inventing a capability nothing needs in order to test it. So the test that
//! needs a *non-empty* prefix gets its own process, which is what an
//! integration test binary is.
//!
//! # What it is defending
//!
//! `Router::nest` fixes incoming requests and nothing else. Everything the
//! station *emits* is an absolute path from `/`, and each of those points
//! outside the application when the prefix is set. The failure is not a clean
//! one: the page renders and then some links 404 while others work, which
//! reads like a caching bug. So the assertions below are mostly about what
//! comes *out* — links, form actions, HTMX targets, the redirect after login,
//! the cookie's `Path`, and the attribute the WebSocket scripts read.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const BASE: &str = "/birdnet";

/// A station with a real on-disk database, served under `BASE`.
///
/// The prefix is installed directly rather than through `BIRDNET_BASE_PATH`:
/// `std::env::set_var` is `unsafe` in edition 2024 and `unsafe` is forbidden
/// workspace-wide, and setting a process-wide variable from one of several
/// concurrently-running `#[tokio::test]`s is exactly the race that made it
/// `unsafe`. `init` is idempotent, so whichever test reaches it first installs
/// the prefix and `build_router`'s own `init(from_env())` is then a no-op.
fn station(dir: &std::path::Path) -> AppState {
    let base = birdnet_web::base_path::BasePath::parse(BASE).expect("valid prefix");
    assert_eq!(
        birdnet_web::base_path::init(base).as_str(),
        BASE,
        "the prefix must be the one in force"
    );
    AppState::new(dir.join("birds.db")).expect("open state")
}

async fn get(state: &AppState, path: &str) -> (StatusCode, axum::http::HeaderMap, String) {
    let req = Request::builder()
        .uri(path)
        .body(Body::empty())
        .expect("request");
    let res = build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response");
    let status = res.status();
    let headers = res.headers().clone();
    let bytes = axum::body::to_bytes(res.into_body(), 8 << 20)
        .await
        .expect("body");
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// A page is reachable at the prefix and not beside it.
///
/// Both halves matter. Serving at the prefix is the feature; *not* serving at
/// the root is what proves the router was nested rather than merged, which a
/// test that only checked the prefix would let through.
#[tokio::test]
async fn the_station_answers_under_the_prefix_and_not_beside_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = station(dir.path());

    let (status, _, body) = get(&state, "/birdnet/species").await;
    assert_eq!(status, StatusCode::OK, "a page must serve under {BASE}");
    assert!(body.contains("<html"), "and be a full page");

    let (status, _, _) = get(&state, "/species").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an un-prefixed page must not answer; the router is nested, not merged"
    );
}

/// The prefix *with* a trailing slash is the URL a browser shows after any
/// link to the station root, and the one most proxies forward.
///
/// It needs its own route: axum 0.8.9's `nest("/b", …)` matches `/b` and
/// `/b/x` but not `/b/` — measured with a two-route probe, not assumed. Without
/// this the feature fails on the first page an operator opens, which is the
/// worst possible place for it to fail.
#[tokio::test]
async fn the_prefix_with_a_trailing_slash_is_not_a_dead_end() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = station(dir.path());

    let (status, headers, _) = get(&state, "/birdnet/").await;
    assert!(
        status.is_redirection(),
        "{BASE}/ must not 404; got {status}"
    );
    assert_eq!(
        headers.get(header::LOCATION).and_then(|v| v.to_str().ok()),
        Some(BASE),
        "and canonicalise onto the un-slashed prefix"
    );

    // ...and the un-slashed prefix itself is live, so the redirect above lands
    // somewhere rather than bouncing.
    let (status, _, _) = get(&state, BASE).await;
    assert!(
        status.is_success() || status.is_redirection(),
        "{BASE} itself must answer; got {status}"
    );
}

/// A visitor who reaches the host root is sent to the prefix rather than shown
/// a 404 that looks like the station is down.
#[tokio::test]
async fn the_bare_root_redirects_into_the_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = station(dir.path());

    let (status, headers, _) = get(&state, "/").await;
    assert!(
        status.is_redirection(),
        "the host root should redirect, got {status}"
    );
    assert_eq!(
        headers.get(header::LOCATION).and_then(|v| v.to_str().ok()),
        Some(BASE),
        "and land on the prefix itself"
    );
}

/// Every URL the dashboard emits carries the prefix.
///
/// Checked against the real rendered page rather than a fixture: the point of
/// this feature is the markup that actually ships, and a fixture proves
/// nothing about the 322 literal paths across the templates and handlers.
#[tokio::test]
async fn every_link_a_rendered_page_emits_is_prefixed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = station(dir.path());
    let (status, _, body) = get(&state, "/birdnet/species").await;
    assert_eq!(status, StatusCode::OK);

    // Nothing application-absolute may escape un-prefixed.
    for attr in ["href", "src", "action", "hx-get", "hx-post", "hx-delete"] {
        let needle = format!("{attr}=\"/");
        for (i, _) in body.match_indices(&needle) {
            let after = &body[i + needle.len() - 1..];
            assert!(
                after.starts_with(BASE) || after.starts_with("//"),
                "{attr} escaped un-prefixed: {:?}",
                &after[..after.len().min(60)]
            );
        }
    }
    // ...and it did find some, so the loop above is not vacuous.
    assert!(
        body.matches("=\"/birdnet/").count() > 10,
        "only {} prefixed URLs on the page; the rewrite is not running",
        body.matches("=\"/birdnet/").count()
    );

    // The stylesheet and the scripts are the ones a reader notices first.
    assert!(body.contains("/birdnet/static/css/app.css"), "stylesheet");
}

/// The scripts that build a WebSocket URL at run time read the prefix from the
/// document, because nothing on the way out can rewrite a string a browser
/// assembles.
#[tokio::test]
async fn the_page_publishes_the_prefix_for_its_scripts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = station(dir.path());
    let (_, _, body) = get(&state, "/birdnet/species").await;
    assert!(
        body.contains(r#"<body data-base-path="/birdnet""#),
        "the prefix must be published on <body> for live-detections.js"
    );
}

/// A static asset is served from under the prefix — the assets are a separate
/// router (`static_files::router()`) merged into the tree, and a mount that
/// only covered the page routes would leave every page unstyled.
#[tokio::test]
async fn static_assets_are_served_from_under_the_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = station(dir.path());

    let (status, headers, _) = get(&state, "/birdnet/static/css/app.css").await;
    assert_eq!(status, StatusCode::OK, "the stylesheet must load");
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|c| c.starts_with("text/css")),
        "and be CSS"
    );
}

/// The API answers under the prefix too. `nest("/api/v2", …)` inside a nested
/// router is the composition most likely to be got wrong, so it is asserted
/// rather than assumed.
#[tokio::test]
async fn the_api_answers_under_the_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = station(dir.path());

    let (status, _, _) = get(&state, "/birdnet/api/v2/health").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "health must answer under the prefix"
    );

    let (status, _, _) = get(&state, "/api/v2/health").await;
    assert_eq!(status, StatusCode::NOT_FOUND, "and not beside it");
}

/// The session cookie is scoped to the prefix.
///
/// Two failures if it is not. A cookie left at `Path=/` is sent to every other
/// application on the same host — this station's session token handed to a
/// neighbour it has nothing to do with. And a cookie at `Path=/birdnet`
/// without the trailing slash also matches `/birdnetsomethingelse` under
/// RFC 6265's path-match rule, which is the same leak by a subtler route.
#[test]
fn the_session_cookie_is_scoped_to_the_prefix() {
    let base = birdnet_web::base_path::BasePath::parse(BASE).expect("valid prefix");
    birdnet_web::base_path::init(base);

    let set = birdnet_web::session::build_set_cookie("tok", 3_600_000, None);
    assert!(
        set.contains("Path=/birdnet/;"),
        "the session cookie must be scoped to the prefix: {set}"
    );
    let clear = birdnet_web::session::build_clear_cookie(None);
    assert!(
        clear.contains("Path=/birdnet/;"),
        "and so must the one that clears it, or sign-out leaves the token behind: {clear}"
    );
}

/// A redirect issued inside the application lands inside the prefix.
///
/// This is the single most visible way base-path support fails: a `Location`
/// header is not HTML, never reaches the body-rewriting pass, and sends the
/// browser out of the prefix on every login and every form that redirects.
#[tokio::test]
async fn a_redirect_stays_inside_the_prefix() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = station(dir.path());

    // `/` inside the app redirects (to onboarding on a fresh station); the
    // target must carry the prefix.
    let (status, headers, _) = get(&state, BASE).await;
    assert!(status.is_redirection(), "expected a redirect, got {status}");
    let loc = headers
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .expect("a redirect carries a Location");
    assert!(
        loc.starts_with(BASE),
        "the redirect left the prefix: {loc:?}"
    );
}
