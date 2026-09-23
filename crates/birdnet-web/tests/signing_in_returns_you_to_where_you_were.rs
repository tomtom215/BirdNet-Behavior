//! Signing in returns you to the page you asked for — and only to this station.
//!
//! Two defects in the one `next=` value:
//!
//! * **Off-site.** `sanitize_next` refused `//evil.example` but not
//!   `/\evil.example`. The `http` crate accepts a backslash, the redirect went
//!   out as `Location: /\evil.example`, and browsers read a leading `/\` as
//!   `//`: a sign-in link on this station's own domain delivered the owner,
//!   freshly authenticated, to someone else's page.
//! * **Nowhere.** The gate percent-encodes the original path into
//!   `/login?next=…` (so `?` becomes `%3F`), and the sign-in page read `next=`
//!   raw and never decoded it. Every deep link with a query string — an alert's
//!   "view this detection", a bookmarked settings tab — landed on a 404 after a
//!   correct password.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const PASSWORD: &str = "a-real-password";

fn app() -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let hash = birdnet_db::accounts::hash_password(PASSWORD).expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        [&hash],
    )
    .expect("give the seed admin a real password");
    build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
}

async fn send(app: &axum::Router, req: Request<Body>) -> axum::response::Response {
    app.clone().oneshot(req).await.expect("router responds")
}

fn location(res: &axum::response::Response) -> String {
    res.headers()
        .get(header::LOCATION)
        .expect("a Location header")
        .to_str()
        .expect("ascii")
        .to_owned()
}

async fn sign_in(app: &axum::Router, next: &str) -> axum::response::Response {
    let body = form_urlencoded::Serializer::new(String::new())
        .append_pair("username", "admin")
        .append_pair("password", PASSWORD)
        .append_pair("next", next)
        .finish();
    send(
        app,
        Request::builder()
            .method(Method::POST)
            .uri("/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("origin", "http://localhost")
            .header("host", "localhost")
            .body(Body::from(body))
            .unwrap(),
    )
    .await
}

#[tokio::test]
async fn a_backslash_cannot_send_you_off_the_station() {
    let app = app();
    for hostile in ["/\\evil.example", "/\\/evil.example", "/\t/evil.example"] {
        let res = sign_in(&app, hostile).await;
        assert_eq!(res.status(), StatusCode::SEE_OTHER, "{hostile:?}");
        assert_eq!(
            location(&res),
            "/admin/overview",
            "next={hostile:?} was followed off-site"
        );
    }
    // Counterpart: an ordinary path is still honoured.
    let res = sign_in(&app, "/admin/notifications").await;
    assert_eq!(location(&res), "/admin/notifications");
}

#[tokio::test]
async fn a_deep_link_with_a_query_survives_the_sign_in_page() {
    let app = app();
    let wanted = "/admin/notifications?page=2";

    let gate = send(
        &app,
        Request::builder().uri(wanted).body(Body::empty()).unwrap(),
    )
    .await;
    assert_eq!(gate.status(), StatusCode::SEE_OTHER);
    let login_url = location(&gate);
    assert!(login_url.starts_with("/login?next="), "{login_url}");

    let page = send(
        &app,
        Request::builder()
            .uri(&login_url)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    let html = String::from_utf8_lossy(
        &axum::body::to_bytes(page.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .into_owned();
    assert!(
        html.contains(r#"name="next" value="/admin/notifications?page=2""#),
        "the sign-in form does not carry the decoded destination"
    );

    let res = sign_in(&app, wanted).await;
    assert_eq!(location(&res), wanted);
}
