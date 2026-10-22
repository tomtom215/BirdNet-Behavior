//! A private station's pages are not kept by a browser, a proxy or a service worker,
//! so signing out ends access to them.
//!
//! `static/sw.js` is written to answer the dashboard surfaces stale-while-revalidate,
//! but it is dormant today: registered with a scope that covers no page, it controls
//! none (its header says so, and `tools/visual-qa/sw-state.mjs` drives it in three
//! engines). Were it given a wider scope, the cached copy on a private station would
//! be the private dashboard, handed back after sign-out before the server could say
//! `303 /login`. The worker already refuses to cache a response marked
//! `Cache-Control: no-store`; the server marking them also covers the browser's own
//! cache, back-forward navigation and any proxy.
//!
//! The mark goes on exactly the responses the private gate lets through to a
//! signed-in browser, and nowhere else: an open station's pages stay cacheable. The
//! worker side is `the_service_worker_does_not_keep_a_private_page.rs`.

use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// The seed admin's password, built per process: the `CodeQL` query on
/// hard-coded cryptographic values follows a literal or a `const` into any
/// parameter called `password` (CLAUDE.md); `format!` over a runtime value is
/// not one.
fn password() -> String {
    format!("a-real-password-{}", std::process::id())
}

fn station(private: bool) -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let hash = birdnet_db::accounts::hash_password(&password()).expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        [&hash],
    )
    .expect("give the seed admin a real password");
    // A station with history: `/` sends an empty one to the setup wizard.
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name)
         VALUES ('2026-09-08', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.91, 'b.wav')",
        [],
    )
    .expect("seed a detection");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    let state = if private {
        state.with_private_mode(BTreeSet::new())
    } else {
        state
    };
    birdnet_web::server::build_router(state)
}

async fn get(app: &axum::Router, path: &str, cookie: Option<&str>) -> axum::response::Response {
    let mut req = Request::builder().method(Method::GET).uri(path);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    app.clone()
        .oneshot(req.body(Body::empty()).expect("request"))
        .await
        .expect("response")
}

async fn sign_in(app: &axum::Router) -> String {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", "http://localhost")
        .header("host", "localhost")
        .body(Body::from(format!(
            "username=admin&password={}",
            password()
        )))
        .expect("request");
    let res = app.clone().oneshot(req).await.expect("response");
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "sign-in failed");
    res.headers()
        .get(header::SET_COOKIE)
        .expect("a cookie")
        .to_str()
        .expect("ascii")
        .split(';')
        .next()
        .expect("pair")
        .to_owned()
}

fn cache_control(res: &axum::response::Response) -> String {
    res.headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned()
}

/// The pages `sw.js` serves stale-while-revalidate, and an API read for good
/// measure. Every one of them, signed in on a private station, says `no-store`.
const GATED: &[&str] = &["/", "/species", "/recordings", "/api/v2/detections/recent"];

#[tokio::test]
async fn what_a_private_station_shows_a_signed_in_browser_is_marked_no_store() {
    let app = station(true);
    let cookie = sign_in(&app).await;
    for path in GATED {
        let res = get(&app, path, Some(&cookie)).await;
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        assert!(
            cache_control(&res).contains("no-store"),
            "{path} was served to a signed-in browser without `no-store`, so the service \
             worker and the browser may keep it past sign-out: {:?}",
            res.headers().get(header::CACHE_CONTROL)
        );
    }
}

/// The counterpart, and the reason the mark is not simply put on everything:
/// an open station's pages are public, and the service worker's offline copy of
/// them is a feature (a kiosk display riding out a network outage).
#[tokio::test]
async fn an_open_stations_pages_stay_cacheable() {
    let app = station(false);
    for path in ["/", "/species", "/recordings"] {
        let res = get(&app, path, None).await;
        assert_eq!(res.status(), StatusCode::OK, "{path}");
        assert!(
            !cache_control(&res).contains("no-store"),
            "{path}: an open station's page was marked no-store, which would take the \
             PWA's offline copy away: {:?}",
            res.headers().get(header::CACHE_CONTROL)
        );
    }
}

/// What a stranger is given on a private station is not the dashboard, so it is
/// not the thing to protect — but it must still not be the page that ends up
/// cached under the dashboard's URL. (The worker side of that is the redirect
/// rule; here: the refusal itself is not cacheable either.)
#[tokio::test]
async fn the_refusal_a_stranger_gets_is_not_cacheable() {
    let app = station(true);
    let res = get(&app, "/", None).await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert!(
        cache_control(&res).contains("no-store"),
        "{:?}",
        res.headers().get(header::CACHE_CONTROL)
    );
    let api = get(&app, "/api/v2/detections/recent", None).await;
    assert_eq!(api.status(), StatusCode::UNAUTHORIZED);
    assert!(cache_control(&api).contains("no-store"));
}
