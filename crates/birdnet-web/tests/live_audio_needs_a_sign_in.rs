//! The live microphone feed needs a sign-in unless the station says otherwise
//! (`OOB-13`).
//!
//! Stored clips pass the human-sound filter; the live feed passes none, and it
//! was served to anyone who could reach the port. These gates hold both sides:
//!
//! * with an admin password and no `live_audio` value (a new install, or a
//!   seed that failed), `/stream` and the HLS control routes answer `401` with
//!   a sentence saying what to do, and the Live page offers a sign-in instead
//!   of a Listen button that would fail silently;
//! * a session opens them; `live_audio=anyone` opens them (an upgraded
//!   station); a station with no password is open as its admin panel is; a
//!   private station's `live_audio` carve-out still opens them;
//! * the live spectrogram socket and the token-addressed HLS content are not
//!   behind it.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_web::private_mode::PublicAccess;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Built at run time: see `tests/no_literal_reaches_a_credential_parameter.rs`.
fn admin_password() -> String {
    format!("listen-password-{}", std::process::id())
}

fn station(with_password: bool, live_audio: Option<&str>) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    if with_password {
        let hash = birdnet_db::accounts::hash_password(&admin_password()).expect("hash");
        conn.execute(
            "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
            [&hash],
        )
        .expect("give the seed admin a real password");
    }
    if let Some(v) = live_audio {
        conn.execute(
            "INSERT INTO settings (key, value, category) VALUES ('live_audio', ?1, 'audio')",
            [v],
        )
        .expect("store live_audio");
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn send(
    app: &axum::Router,
    method: Method,
    path: &str,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("host", "localhost")
        .header("origin", "http://localhost");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let res = app
        .clone()
        .oneshot(req.body(Body::empty()).expect("request"))
        .await
        .expect("router responds");
    let status = res.status();
    // A successful `/stream` body never ends.
    let body = if status.is_success() && path == "/stream" {
        String::new()
    } else {
        let bytes = axum::body::to_bytes(res.into_body(), 1024 * 1024)
            .await
            .unwrap_or_default();
        String::from_utf8_lossy(&bytes).into_owned()
    };
    (status, body)
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
            admin_password()
        )))
        .expect("build request");
    let res = app.clone().oneshot(req).await.expect("router responds");
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "login did not succeed");
    res.headers()
        .get(header::SET_COOKIE)
        .expect("login sets a cookie")
        .to_str()
        .expect("ascii cookie")
        .split(';')
        .next()
        .expect("cookie pair")
        .to_owned()
}

/// The routes the policy covers, with the method each is called with.
const AUDIO: &[(&str, &str)] = &[
    ("GET", "/stream"),
    ("POST", "/api/v2/streams/hls/no_such_source/start"),
    ("POST", "/api/v2/streams/hls/no_such_source/stop"),
    ("POST", "/api/v2/streams/hls/heartbeat"),
    ("GET", "/api/v2/streams/hls/status"),
];

fn method(m: &str) -> Method {
    Method::from_bytes(m.as_bytes()).expect("method")
}

#[tokio::test]
async fn a_stranger_is_refused_the_live_feed_on_a_station_with_a_password() {
    for live_audio in [None, Some("signed-in")] {
        let app = birdnet_web::server::build_router(station(true, live_audio));
        for (m, path) in AUDIO {
            let (status, body) = send(&app, method(m), path, None).await;
            assert_eq!(
                status,
                StatusCode::UNAUTHORIZED,
                "{m} {path} (live_audio={live_audio:?}) answered {status}: {body}"
            );
            assert!(
                body.contains("signed-in users"),
                "{m} {path}'s refusal must say why: {body}"
            );
        }
    }
}

#[tokio::test]
async fn the_live_page_offers_a_stranger_a_sign_in_not_a_dead_button() {
    let app = birdnet_web::server::build_router(station(true, None));
    let (status, html) = send(&app, Method::GET, "/recordings?view=live", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        !html.contains(r#"id="rc-listen-btn""#),
        "a stranger must not be offered a Listen button that answers 401"
    );
    assert!(
        html.contains("Sign in to listen"),
        "the Live page must say how to listen"
    );

    // Counterpart: a signed-in owner gets the button.
    let cookie = sign_in(&app).await;
    let (_, html) = send(&app, Method::GET, "/recordings?view=live", Some(&cookie)).await;
    assert!(
        html.contains(r#"id="rc-listen-btn""#),
        "signed in, the button is back"
    );
    assert!(!html.contains("Sign in to listen"));
}

#[tokio::test]
async fn a_session_opens_the_feed() {
    let app = birdnet_web::server::build_router(station(true, None));
    let cookie = sign_in(&app).await;
    for (m, path) in AUDIO {
        let (status, body) = send(&app, method(m), path, Some(&cookie)).await;
        assert_ne!(status, StatusCode::UNAUTHORIZED, "{m} {path}: {body}");
        assert!(!body.contains("signed-in users"), "{m} {path}: {body}");
    }
}

#[tokio::test]
async fn anyone_keeps_the_feed_open_and_so_does_a_station_without_a_password() {
    for (with_password, live_audio) in [
        (true, Some("anyone")),
        (false, None),
        (false, Some("signed-in")),
    ] {
        let app = birdnet_web::server::build_router(station(with_password, live_audio));
        for (m, path) in AUDIO {
            let (status, body) = send(&app, method(m), path, None).await;
            assert_ne!(
                status,
                StatusCode::UNAUTHORIZED,
                "{m} {path} (password: {with_password}, live_audio={live_audio:?}): {body}"
            );
        }
    }
}

#[tokio::test]
async fn a_private_stations_live_audio_carve_out_still_opens_it() {
    let state =
        station(true, None).with_private_mode(std::iter::once(PublicAccess::LiveAudio).collect());
    let app = birdnet_web::server::build_router(state);
    for (m, path) in AUDIO {
        let (status, body) = send(&app, method(m), path, None).await;
        assert_ne!(status, StatusCode::UNAUTHORIZED, "{m} {path}: {body}");
        assert_ne!(status, StatusCode::SEE_OTHER, "{m} {path}: {body}");
    }
}

#[tokio::test]
async fn the_spectrogram_and_the_token_content_are_not_behind_it() {
    let app = birdnet_web::server::build_router(station(true, None));
    for path in [
        "/api/v2/ws/spectrogram",
        "/api/v2/streams/hls/t/00000000000000000000000000000000/playlist.m3u8",
    ] {
        let (status, body) = send(&app, Method::GET, path, None).await;
        assert_ne!(status, StatusCode::UNAUTHORIZED, "{path}: {body}");
    }
}

/// The control on Settings → Capture sets the policy, at once: opening it lets
/// a stranger in on the next request, and closing it again shuts them out.
#[tokio::test]
async fn the_capture_tab_control_opens_and_closes_the_feed() {
    let app = birdnet_web::server::build_router(station(true, None));
    let cookie = sign_in(&app).await;

    let (status, body) = post_policy(&app, "anyone", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(&app, Method::GET, "/stream", None).await;
    assert_ne!(
        status,
        StatusCode::UNAUTHORIZED,
        "opened, a stranger is let in: {body}"
    );

    let (status, body) = post_policy(&app, "signed-in", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, _) = send(&app, Method::GET, "/stream", None).await;
    assert_eq!(
        status,
        StatusCode::UNAUTHORIZED,
        "closed again, the stranger is out"
    );

    // A value that is neither is refused and changes nothing.
    let (_, body) = post_policy(&app, "everyone", Some(&cookie)).await;
    assert!(body.contains("Choose who can hear"), "{body}");
    let (status, _) = send(&app, Method::GET, "/stream", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

/// A stranger cannot change the policy: the route is behind the admin gate.
#[tokio::test]
async fn a_stranger_cannot_open_the_feed_through_the_control() {
    let app = birdnet_web::server::build_router(station(true, None));
    let (status, _) = post_policy(&app, "anyone", None).await;
    assert!(status.is_client_error(), "{status}");
    let (status, _) = send(&app, Method::GET, "/stream", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

async fn post_policy(
    app: &axum::Router,
    value: &str,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri("/admin/audio/live-access")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", "http://localhost")
        .header("host", "localhost");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let res = app
        .clone()
        .oneshot(
            req.body(Body::from(format!("live_audio={value}")))
                .expect("request"),
        )
        .await
        .expect("router responds");
    let status = res.status();
    let bytes = axum::body::to_bytes(res.into_body(), 64 * 1024)
        .await
        .unwrap_or_default();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}
