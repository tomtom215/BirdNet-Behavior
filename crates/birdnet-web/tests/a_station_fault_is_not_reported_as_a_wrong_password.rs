//! A correct password must never be reported as an incorrect one.
//!
//! # The defect
//!
//! `login_submit` verifies the password, then writes a session row. Both
//! failures redirected to `/login?error=1`, and `?error=1` renders "Incorrect
//! username or password." So when the write failed — a full disk, a locked
//! database, a schema the station could not reach — someone who had typed
//! their password correctly was told it was wrong.
//!
//! What that person does next is retype it, several times. Five failures in a
//! row trip `login_is_throttled`'s limiter, and the station then tells them
//! "Too many attempts", locking them out of their own garden over a fault that
//! was never theirs and that nothing on the page names. The real cause went to
//! the server log, which is the one place this reader cannot look.
//!
//! # What is guarded
//!
//! The discrimination, from both sides. A genuinely wrong password must still
//! say so — a station that blamed itself for every failed sign-in would be
//! just as wrong, and would hide a real typo — while a station that cannot
//! write a session must say that instead, and must not send the reader back to
//! their password.

use axum::body::Body;
use axum::http::{Request, Response, StatusCode, header};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const PASSWORD: &str = "hunter2correct";

/// A station with a known admin password. `break_sessions` drops the table
/// `create_session` writes to, so the password still verifies (a read of
/// `users`) and only the session write fails — which is the exact shape of the
/// real fault.
fn state_with_admin(break_sessions: bool) -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    let hash = birdnet_db::accounts::hash_password(PASSWORD).expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        rusqlite::params![hash],
    )
    .expect("set admin password");
    if break_sessions {
        conn.execute("DROP TABLE sessions", [])
            .expect("drop sessions");
    }
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn post_login(state: &AppState, password: &str) -> Response<Body> {
    let req = Request::builder()
        .method("POST")
        .uri("/login")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(Body::from(format!("username=admin&password={password}")))
        .expect("request");
    birdnet_web::server::build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response")
}

/// Follow the redirect and return **the rendered error text**, not the page.
///
/// Reading the whole page would be wrong here, and quietly so: `login.html`
/// opens with a comment documenting its own placeholders, and that comment
/// quotes "Incorrect username or password." verbatim. A `page.contains(...)`
/// assertion is satisfied by that comment whichever error is actually showing,
/// so the first version of this test failed against a correct fix.
async fn message_after_login(state: &AppState, password: &str) -> String {
    let res = post_login(state, password).await;
    let location = res
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        res.status().is_redirection(),
        "a failed sign-in should redirect back to the form; got {}",
        res.status()
    );
    let req = Request::builder()
        .uri(&location)
        .body(Body::empty())
        .expect("request");
    let page = birdnet_web::server::build_router(state.clone())
        .oneshot(req)
        .await
        .expect("response");
    assert_eq!(page.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(page.into_body(), 1 << 20)
        .await
        .expect("body");
    let html = String::from_utf8_lossy(&bytes);
    let open = r#"<div class="login-alert__body">"#;
    let start = html
        .find(open)
        .map(|i| i + open.len())
        .expect("the login form must render its alert body");
    let rest = &html[start..];
    let end = rest.find("</div>").expect("alert body must close");
    rest[..end].trim().to_owned()
}

#[tokio::test]
async fn a_session_that_cannot_be_written_is_not_blamed_on_the_password() {
    let state = state_with_admin(true);
    let page = message_after_login(&state, PASSWORD).await;

    assert!(
        !page.contains("Incorrect username or password."),
        "a correct password was reported as incorrect because the station \
         could not write the session row. The form said: {page:?}"
    );
    assert!(
        page.contains("could not start your session"),
        "the form must say what actually failed. The form said: {page:?}"
    );
}

/// The counterpart. If the station blamed itself for every failed sign-in, the
/// test above would pass and a real typo would never be named.
#[tokio::test]
async fn a_wrong_password_is_still_reported_as_a_wrong_password() {
    let state = state_with_admin(false);
    let page = message_after_login(&state, "definitelynotit").await;

    assert!(
        page.contains("Incorrect username or password."),
        "a wrong password must still say so. The form said: {page:?}"
    );
    assert!(
        !page.contains("could not start your session"),
        "a wrong password was excused as a station fault"
    );
}
