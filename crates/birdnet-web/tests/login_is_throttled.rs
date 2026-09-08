//! The sign-in form refuses an address after five failures (O-6).
//!
//! The "Too many attempts" page existed, and the flag that rendered it was set
//! in exactly one place: a unit test. `login_submit` never set it, so the
//! branch was dead and the global limiter's ~30 posts a second per address
//! were all Argon2id hashes the Pi computed for whoever asked.
//!
//! Three things have to be true, and each is its own gate: the sixth attempt
//! from one address is refused, and refused *before* the password is checked
//! (a correct password fares no better); a sixth attempt from a different
//! address is not affected, because a blanket lock is a denial of service any
//! stranger can trigger against the operator; and a successful sign-in clears
//! the address, so an operator who mistyped four times is not one keystroke
//! from a quarter-hour lockout for the rest of the session.
//!
//! `oneshot` inserts no `ConnectInfo`, so the peer is loopback, which the
//! default trusted-proxy policy trusts: `X-Forwarded-For` therefore sets the
//! client address, exactly as it does for a station behind its own proxy.

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const PASSWORD: &str = "hunter2correct";

fn state_with_admin() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    let hash = birdnet_db::accounts::hash_password(PASSWORD).expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        rusqlite::params![hash],
    )
    .expect("set admin password");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn post_login(state: &AppState, from: &str, password: &str) -> Response<Body> {
    let app = birdnet_web::server::build_router(state.clone());
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", "http://localhost")
        .header("host", "localhost")
        .header("x-forwarded-for", from)
        .body(Body::from(format!("username=admin&password={password}")))
        .expect("build request");
    app.oneshot(req).await.expect("router responds")
}

fn audit_actions(state: &AppState) -> Vec<String> {
    state
        .with_db(|conn| -> rusqlite::Result<Vec<String>> {
            let mut stmt = conn.prepare("SELECT action FROM audit_log ORDER BY id")?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows)
        })
        .expect("read audit log")
}

/// Five wrong passwords redirect back to the form; the sixth is refused with
/// 429 and a `Retry-After`, and so is a *correct* password from the same
/// address — the password was not checked, which is the point.
#[tokio::test]
async fn the_sixth_attempt_from_one_address_is_refused_before_the_password_is_checked() {
    let state = state_with_admin();
    for i in 1..=5 {
        let res = post_login(&state, "203.0.113.7", "wrong").await;
        assert_eq!(
            res.status(),
            StatusCode::SEE_OTHER,
            "attempt {i}: a plain failure"
        );
        assert!(
            res.headers()["location"]
                .to_str()
                .unwrap()
                .contains("error=1"),
            "attempt {i}"
        );
    }
    let res = post_login(&state, "203.0.113.7", "wrong").await;
    assert_eq!(
        res.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the sixth attempt"
    );
    let retry: u64 = res.headers()["retry-after"]
        .to_str()
        .unwrap()
        .parse()
        .expect("Retry-After is seconds");
    assert!((1..=15 * 60).contains(&retry), "Retry-After {retry}s");
    let bytes = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .expect("read body");
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(
        body.contains("Too many attempts"),
        "the locked page is rendered"
    );
    assert!(body.contains("disabled"), "the form is disabled");

    // The right password from the throttled address gets the same answer:
    // no session is minted, because nothing was verified.
    let res = post_login(&state, "203.0.113.7", PASSWORD).await;
    assert_eq!(res.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        res.headers().get("set-cookie").is_none(),
        "no session for a refused attempt"
    );

    assert_eq!(
        audit_actions(&state),
        vec![
            "auth.login.fail",
            "auth.login.fail",
            "auth.login.fail",
            "auth.login.fail",
            "auth.login.fail",
            "auth.login.throttled",
            "auth.login.throttled",
        ],
        "five failures were checked, two refusals were not"
    );
}

/// A throttle that could not tell addresses apart would let a stranger lock
/// the operator out; the sixth attempt from another address goes through.
#[tokio::test]
async fn a_sixth_attempt_from_another_address_is_not_affected() {
    let state = state_with_admin();
    for _ in 0..5 {
        post_login(&state, "203.0.113.7", "wrong").await;
    }
    assert_eq!(
        post_login(&state, "203.0.113.7", "wrong").await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let other = post_login(&state, "203.0.113.8", PASSWORD).await;
    assert_eq!(
        other.status(),
        StatusCode::SEE_OTHER,
        "another address signs in"
    );
    assert!(other.headers().get("set-cookie").is_some());
}

/// Four mistakes and then the right password: signed in, and the slate is
/// clean, so the next mistake is the first of a new five.
#[tokio::test]
async fn a_successful_sign_in_clears_the_address_failures() {
    let state = state_with_admin();
    for _ in 0..4 {
        post_login(&state, "203.0.113.7", "wrong").await;
    }
    let ok = post_login(&state, "203.0.113.7", PASSWORD).await;
    assert_eq!(ok.status(), StatusCode::SEE_OTHER);
    assert!(ok.headers().get("set-cookie").is_some());
    for i in 1..=5 {
        assert_eq!(
            post_login(&state, "203.0.113.7", "wrong").await.status(),
            StatusCode::SEE_OTHER,
            "mistake {i} after a clean sign-in is still just a failure"
        );
    }
    assert_eq!(
        post_login(&state, "203.0.113.7", "wrong").await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}
