//! Authenticating a request is a read. It used to be a write too.
//!
//! `session_user` ran on every admin request, and on every request of a private
//! station, and did three things under the one writer lock: look the session up,
//! `UPDATE sessions SET last_seen`, look the user up. So every signed-in click
//! queued behind a detection being stored, and the detection processor queued
//! behind every click. The lookups are reads and now use a pooled reader. The
//! `last_seen` touch is a write, kept on the writer but only when the stored
//! value is at least a minute old: it is displayed on the Accounts page and
//! nothing else reads it, so a minute is invisible, and an authenticated burst of
//! fifty requests is one write instead of fifty.
//!
//! Two properties, each with its counterpart:
//!
//! * with the writer lock held by someone else, a signed-in request whose
//!   session was seen a moment ago still answers;
//! * a session that has not been seen for two minutes is still touched, so the
//!   throttle cannot pass by the touch never happening.

use std::sync::mpsc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// The seed admin's password, made per run rather than written down: code
/// scanning reads a literal that reaches `hash_password` as a hard-coded
/// cryptographic value, and this test needs only a password that works.
fn password() -> String {
    format!("password-{}", std::process::id())
}

fn station(dir: &std::path::Path) -> AppState {
    let state = AppState::new(dir.join("birds.db")).expect("state");
    let hash = birdnet_db::accounts::hash_password(&password()).expect("hash");
    state.with_db(|conn| {
        conn.execute(
            "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
            [&hash],
        )
        .expect("give the seed admin a real password");
    });
    state
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

async fn admin_status(app: &axum::Router, cookie: &str, within: Duration) -> Option<StatusCode> {
    let req = Request::builder()
        .uri("/admin/overview")
        .header(header::COOKIE, cookie)
        .body(Body::empty())
        .unwrap();
    tokio::time::timeout(within, app.clone().oneshot(req))
        .await
        .ok()
        .map(|r| r.expect("router responds").status())
}

fn last_seen(state: &AppState) -> String {
    state.with_db(|conn| {
        conn.query_row("SELECT last_seen FROM sessions LIMIT 1", [], |r| r.get(0))
            .expect("one session")
    })
}

fn set_last_seen(state: &AppState, seconds_ago: u32) {
    state.with_db(|conn| {
        conn.execute(
            "UPDATE sessions SET last_seen = datetime('now', ?1)",
            [format!("-{seconds_ago} seconds")],
        )
        .expect("age the session");
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_signed_in_request_answers_while_the_writer_lock_is_held() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = station(dir.path());
    assert!(state.reader_count() > 0, "precondition: pooled readers");
    let app = build_router(state.clone());
    let cookie = sign_in(&app).await;
    set_last_seen(&state, 10); // seen ten seconds ago: no touch is due

    // Someone else holds the writer until told otherwise.
    let (held_tx, held_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let holder = {
        let state = state.clone();
        std::thread::spawn(move || {
            state.with_db(|_| {
                held_tx.send(()).unwrap();
                let _ = release_rx.recv_timeout(Duration::from_secs(30));
            });
        })
    };
    held_rx.recv().unwrap();

    let status = admin_status(&app, &cookie, Duration::from_secs(3)).await;
    release_tx.send(()).ok();
    holder.join().unwrap();

    assert_eq!(
        status,
        Some(StatusCode::OK),
        "a signed-in request waited for the writer lock (None = no answer in 3 s)"
    );
}

#[tokio::test]
async fn the_session_is_touched_when_it_has_gone_quiet_and_not_before() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = station(dir.path());
    let app = build_router(state.clone());
    let cookie = sign_in(&app).await;

    // Seen ten seconds ago: three requests later it is still ten seconds ago.
    set_last_seen(&state, 10);
    let recent = last_seen(&state);
    for _ in 0..3 {
        assert_eq!(
            admin_status(&app, &cookie, Duration::from_secs(5)).await,
            Some(StatusCode::OK)
        );
    }
    assert_eq!(
        last_seen(&state),
        recent,
        "a request inside the minute rewrote last_seen"
    );

    // Not seen for two minutes: the next request records that it was.
    set_last_seen(&state, 120);
    let quiet = last_seen(&state);
    assert_eq!(
        admin_status(&app, &cookie, Duration::from_secs(5)).await,
        Some(StatusCode::OK)
    );
    assert_ne!(
        last_seen(&state),
        quiet,
        "a session idle for two minutes was not touched"
    );
}
