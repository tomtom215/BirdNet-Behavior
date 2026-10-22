//! The live log stream sends a byte the moment it opens, so a browser that
//! waits for one can say "Connected".
//!
//! `GET /admin/system/logs` is a Server-Sent Events stream. Its only periodic
//! output is a keep-alive comment every 15 seconds, and on a quiet log the
//! backlog it replays is empty — so for up to 15 seconds the response had
//! headers and no body. Chromium and WebKit fire an `EventSource`'s `open` on
//! the headers; **Firefox fires it on the first body byte**, and the viewer
//! (`/admin/system/logs/page`) sat at "Connecting…" over a healthy stream.
//! `tools/visual-qa/interactions.mjs` ("live logs: the stream connects") is the
//! browser-side witness and failed only in Firefox.
//!
//! This test goes through the real router (the admin gate, the security-header
//! pass, the compression layer that must leave `text/event-stream` alone) and
//! reads the first body frame with a deadline far inside the keep-alive.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_web::routes::admin::logs::{LogBroadcaster, LogLine};
use birdnet_web::state::AppState;
use tokio_stream::StreamExt as _;
use tower::ServiceExt as _;

/// Built at run time: the hard-coded-credential query follows a literal into a
/// parameter called `password` (CLAUDE.md).
fn password() -> String {
    format!("log-stream-password-{}", std::process::id())
}

fn station(broadcaster: LogBroadcaster) -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let hash = birdnet_db::accounts::hash_password(&password()).expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        [&hash],
    )
    .expect("give the seed admin a real password");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
        .with_log_broadcaster(broadcaster);
    birdnet_web::server::build_router(state)
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

async fn open_stream(
    app: &axum::Router,
    cookie: &str,
) -> impl tokio_stream::Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/system/logs")
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        res.headers()[header::CONTENT_TYPE]
            .to_str()
            .expect("ascii")
            .starts_with("text/event-stream"),
        "{:?}",
        res.headers()
    );
    res.into_body().into_data_stream()
}

/// The next chunk of the body as text, or `None` when none arrived within
/// `within`. The body is consumed as a stream, so a chunk already read is not
/// read twice.
async fn next_chunk(
    body: &mut (impl tokio_stream::Stream<Item = Result<axum::body::Bytes, axum::Error>> + Unpin),
    within: Duration,
) -> Option<String> {
    let chunk = tokio::time::timeout(within, body.next())
        .await
        .ok()??
        .ok()?;
    Some(String::from_utf8_lossy(&chunk).into_owned())
}

fn line(message: &str) -> LogLine {
    LogLine {
        level: "INFO".into(),
        message: message.into(),
        target: "test".into(),
        timestamp_ms: 0,
    }
}

/// A quiet log: nothing to replay, nothing published. The first chunk is a
/// comment, and it arrives inside two seconds — the keep-alive is fifteen.
#[tokio::test]
async fn a_quiet_log_stream_sends_its_first_byte_at_once() {
    let app = station(LogBroadcaster::new());
    let cookie = sign_in(&app).await;
    let mut body = open_stream(&app, &cookie).await;

    let first = next_chunk(&mut body, Duration::from_secs(2)).await.expect(
        "no body byte within 2 s of opening the stream: Firefox reports `open` only on \
             the first body byte, so the viewer would read `Connecting…` until the 15 s \
             keep-alive",
    );
    assert!(
        first.starts_with(':'),
        "the first thing on the wire is a comment, which an EventSource ignores: {first:?}"
    );
    assert!(
        !first.contains("event:") && !first.contains("data:"),
        "the opening chunk must not be an event the viewer would render: {first:?}"
    );
}

/// The counterpart, so "sends something" is not satisfied by a stream that
/// sends only the comment: the backlog still follows it, and a line published
/// after the connection still reaches the client as a `log` event.
#[tokio::test]
async fn the_comment_does_not_replace_the_backlog_or_the_live_lines() {
    let broadcaster = LogBroadcaster::new();
    broadcaster.publish(line("before the viewer opened"));
    let app = station(broadcaster.clone());
    let cookie = sign_in(&app).await;
    let mut body = open_stream(&app, &cookie).await;

    let mut seen = String::new();
    while !seen.contains("before the viewer opened") {
        let chunk = next_chunk(&mut body, Duration::from_secs(2))
            .await
            .unwrap_or_else(|| panic!("the replayed backlog never arrived; saw {seen:?}"));
        seen.push_str(&chunk);
    }
    assert!(seen.starts_with(':'), "the comment comes first: {seen:?}");
    assert!(
        seen.contains("event: log"),
        "the backlog is a `log` event: {seen:?}"
    );

    broadcaster.publish(line("after the viewer opened"));
    let mut live = String::new();
    while !live.contains("after the viewer opened") {
        let chunk = next_chunk(&mut body, Duration::from_secs(2))
            .await
            .unwrap_or_else(|| {
                panic!("a line published after the connection never arrived; saw {live:?}")
            });
        live.push_str(&chunk);
    }
}
