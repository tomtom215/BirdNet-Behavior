//! A private station serves nothing but the sign-in to a stranger — and
//! exactly what the operator carved out (O-4).
//!
//! The station's default contract is "viewing is open". Behind a tunnel or a
//! port forward that contract publishes the detection history and a live
//! microphone feed to whoever has the URL. Private mode is the switch, and
//! these gates hold both directions of it:
//!
//! 1. With private mode on and a password set, the dashboard, the read API
//!    and both `WebSockets` are refused without a session, while the sign-in
//!    form, its assets and the health probe stay open.
//! 2. A session opens everything, so the operator is not locked out of their
//!    own station.
//! 3. `live_audio` opens the stream and the live spectrogram socket and
//!    nothing else; `share` keeps an operator-minted share link working,
//!    audio included, while the recordings route it used to redirect to stays
//!    closed; `metrics` opens the exposition.
//! 4. Private mode with no admin password fails closed: `503` and a message,
//!    not the open station the operator asked not to have.
//! 5. With private mode off nothing changes, password or not.

use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_web::private_mode::PublicAccess;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const PASSWORD: &str = "a-real-password";
const CLIP: &str = "Eurasian_Blackbird-91-2026-09-08-birdnet-06:10:00.wav";

/// A station with one detection and its clip on disk. `password` gives the
/// seed admin a real hash so the gates observe an authorisation decision
/// rather than the fresh-station bypass.
fn station(password: bool, tmp: &std::path::Path) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    if password {
        let hash = birdnet_db::accounts::hash_password(PASSWORD).expect("hash");
        conn.execute(
            "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
            [&hash],
        )
        .expect("give the seed admin a real password");
    }
    conn.execute(
        "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence, File_Name) \
         VALUES ('2026-09-08', '06:10:00', 'Turdus merula', 'Eurasian Blackbird', 0.91, ?1)",
        [CLIP],
    )
    .expect("insert detection");
    std::fs::write(tmp.join(CLIP), b"RIFF not really a wav but bytes are bytes")
        .expect("write clip");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
        .with_recording_dir(tmp.to_path_buf())
}

fn private(state: AppState, public: &[PublicAccess]) -> AppState {
    state.with_private_mode(public.iter().copied().collect::<BTreeSet<_>>())
}

async fn get(app: &axum::Router, path: &str, cookie: Option<&str>) -> axum::response::Response {
    let mut req = Request::builder().method(Method::GET).uri(path);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    app.clone()
        .oneshot(req.body(Body::empty()).expect("build request"))
        .await
        .expect("router responds")
}

async fn status(app: &axum::Router, path: &str) -> StatusCode {
    get(app, path, None).await.status()
}

/// Sign in as the seed admin and return the `bnb-session` cookie.
async fn sign_in(app: &axum::Router) -> String {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", "http://localhost")
        .header("host", "localhost")
        .body(Body::from(format!("username=admin&password={PASSWORD}")))
        .expect("build request");
    let res = app.clone().oneshot(req).await.expect("router responds");
    assert_eq!(res.status(), StatusCode::SEE_OTHER, "login did not succeed");
    let set = res
        .headers()
        .get(header::SET_COOKIE)
        .expect("login sets a cookie")
        .to_str()
        .expect("ascii cookie");
    set.split(';').next().expect("cookie pair").to_owned()
}

fn share_link(state: &AppState) -> String {
    // The token is signed with a process secret, not with anything on the
    // state; the parameter documents which station the link is for.
    let _ = state;
    let token =
        birdnet_web::routes::share::issue_token_for("2026-09-08", "06:10:00", "Eurasian Blackbird");
    format!("/r/{token}")
}

/// Everything a stranger must not see on a private station, with what the
/// refusal looks like: pages redirect to the form, the API and the sockets
/// get a 401.
const CLOSED_PAGES: &[&str] = &[
    "/",
    "/quarantine",
    "/recordings",
    "/species",
    "/feeds/rss",
    "/r/x",
];
fn closed_api() -> Vec<String> {
    vec![
        "/api/v2/detections".to_owned(),
        "/api/v2/detections/recent".to_owned(),
        "/api/v2/ws/detections".to_owned(),
        "/api/v2/ws/spectrogram".to_owned(),
        "/api/v2/metrics".to_owned(),
        "/api/v2/recordings".to_owned(),
        format!("/api/v2/recordings/{CLIP}"),
    ]
}

#[tokio::test]
async fn a_private_station_serves_a_stranger_only_the_sign_in() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let state = private(station(true, tmp.path()), &[]);
    let app = birdnet_web::server::build_router(state);

    for path in CLOSED_PAGES {
        let res = get(&app, path, None).await;
        assert_eq!(
            res.status(),
            StatusCode::SEE_OTHER,
            "GET {path} on a private station answered {} to a stranger; it must send them to the sign-in",
            res.status()
        );
        let location = res.headers()[header::LOCATION].to_str().expect("ascii");
        assert!(
            location.starts_with("/login?next="),
            "GET {path} redirected to {location}, not the sign-in"
        );
    }
    for path in closed_api() {
        let res = get(&app, &path, None).await;
        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "GET {path} on a private station answered {} to a stranger",
            res.status()
        );
    }
    // `/stream` is the live microphone; it is a page-shaped path, so the
    // refusal is the redirect, and the point is that it is a refusal.
    assert_eq!(status(&app, "/stream").await, StatusCode::SEE_OTHER);

    for path in [
        "/api/v2/health",
        "/login",
        "/static/css/app.css",
        "/favicon.ico",
    ] {
        let code = status(&app, path).await;
        assert!(
            code.is_success(),
            "GET {path} answered {code}; a browser needs it to reach the sign-in, or the watchdog to see the process"
        );
    }
}

#[tokio::test]
async fn a_session_opens_the_private_station() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let state = private(station(true, tmp.path()), &[]);
    let app = birdnet_web::server::build_router(state);
    let cookie = sign_in(&app).await;

    for path in [
        "/",
        "/quarantine",
        "/api/v2/detections",
        "/api/v2/metrics",
        "/recordings",
    ] {
        let code = get(&app, path, Some(&cookie)).await.status();
        assert!(
            code.is_success(),
            "GET {path} with a session answered {code}; the operator is locked out of their own station"
        );
    }
}

#[tokio::test]
async fn each_carve_out_opens_its_own_surface_and_no_other() {
    let tmp = tempfile::tempdir().expect("tempdir");

    // live_audio: the stream and the live spectrogram socket. `/stream` with
    // no capture source answers 503 (nothing to stream), which is the handler
    // speaking — a gate would have said 303. The socket without an upgrade
    // header is the handler's 4xx, not the gate's 401.
    let live = birdnet_web::server::build_router(private(
        station(true, tmp.path()),
        &[PublicAccess::LiveAudio],
    ));
    assert_ne!(status(&live, "/stream").await, StatusCode::SEE_OTHER);
    assert_ne!(
        status(&live, "/api/v2/ws/spectrogram").await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status(&live, "/api/v2/ws/detections").await,
        StatusCode::UNAUTHORIZED,
        "live_audio must not open the detection socket"
    );
    assert_eq!(status(&live, "/").await, StatusCode::SEE_OTHER);

    // metrics: the exposition and nothing else.
    let metrics = birdnet_web::server::build_router(private(
        station(true, tmp.path()),
        &[PublicAccess::Metrics],
    ));
    assert_eq!(status(&metrics, "/api/v2/metrics").await, StatusCode::OK);
    assert_eq!(
        status(&metrics, "/api/v2/detections").await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn an_operator_minted_share_link_keeps_working_only_with_the_share_carve_out() {
    let tmp = tempfile::tempdir().expect("tempdir");

    let state = private(station(true, tmp.path()), &[PublicAccess::Share]);
    let link = share_link(&state);
    let app = birdnet_web::server::build_router(state);

    let page = get(&app, &link, None).await;
    assert_eq!(
        page.status(),
        StatusCode::OK,
        "the share page must render for a stranger"
    );
    let audio = get(&app, &format!("{link}/audio.wav"), None).await;
    assert_eq!(
        audio.status(),
        StatusCode::OK,
        "the shared clip must play for a stranger (status {}); a redirect to the gated \
         recordings route would land on the sign-in",
        audio.status()
    );
    let body = axum::body::to_bytes(audio.into_body(), usize::MAX)
        .await
        .expect("body");
    assert!(
        body.starts_with(b"RIFF"),
        "the clip bytes must be the file itself"
    );

    // The carve-out is the share link, not the recordings route behind it.
    assert_eq!(
        status(&app, &format!("/api/v2/recordings/{CLIP}")).await,
        StatusCode::UNAUTHORIZED,
        "`share` must not open the recordings route to anyone who can guess a filename"
    );
    assert_eq!(status(&app, "/").await, StatusCode::SEE_OTHER);

    // Without the carve-out the same link is refused.
    let closed = private(station(true, tmp.path()), &[]);
    let link = share_link(&closed);
    let closed = birdnet_web::server::build_router(closed);
    assert_eq!(status(&closed, &link).await, StatusCode::SEE_OTHER);
    assert_eq!(
        status(&closed, &format!("{link}/audio.wav")).await,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn private_mode_without_a_password_fails_closed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let state = private(station(false, tmp.path()), &[]);
    let app = birdnet_web::server::build_router(state);

    for path in ["/", "/api/v2/detections", "/stream"] {
        let res = get(&app, path, None).await;
        assert_eq!(
            res.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "GET {path} on a private station with no password answered {}; the admin gate's \
             open-bypass must not apply here",
            res.status()
        );
    }
    let res = get(&app, "/", None).await;
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("body");
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("CADDY_PWD"),
        "the 503 must say what to set; got: {body}"
    );
    assert_eq!(status(&app, "/api/v2/health").await, StatusCode::OK);
}

#[tokio::test]
async fn with_private_mode_off_nothing_changes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    for password in [false, true] {
        let state = station(password, tmp.path());
        let link = share_link(&state);
        let app = birdnet_web::server::build_router(state);
        for path in [
            "/",
            "/quarantine",
            "/api/v2/detections",
            "/api/v2/metrics",
            &format!("/api/v2/recordings/{CLIP}"),
            &link,
            &format!("{link}/audio.wav"),
        ] {
            let code = status(&app, path).await;
            assert!(
                code.is_success(),
                "GET {path} answered {code} with private mode off (password: {password}); \
                 the open contract must be untouched"
            );
        }
    }
}
