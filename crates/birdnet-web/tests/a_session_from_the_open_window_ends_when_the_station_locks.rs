//! A session minted while the station had no password ends when it gets one.
//!
//! On a fresh station with no admin password, `POST /login` with *any*
//! credentials mints a real admin session row and a 14-day cookie (the
//! "open bypass"). That is fine while the station is open — everyone is admin
//! anyway. It stopped being fine the moment the owner chose a password: neither
//! the accounts page's first-password branch nor the setup wizard revoked
//! anything, and the signing secret is persisted, so whoever had signed in
//! during the open window stayed admin — through restarts — on a station its
//! owner believed was now locked.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_db::accounts::UserStore as _;
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

const PASSWORD: &str = "owner-chosen-password";

fn open_station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn send(app: &axum::Router, req: Request<Body>) -> axum::response::Response {
    app.clone().oneshot(req).await.expect("router responds")
}

fn post_form(uri: &str, body: String, cookie: Option<&str>) -> Request<Body> {
    let mut req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded")
        .header("origin", "http://localhost")
        .header("host", "localhost");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    req.body(Body::from(body)).unwrap()
}

/// Sign in during the open window, with credentials that mean nothing.
async fn stranger_signs_in(app: &axum::Router) -> String {
    let res = send(
        app,
        post_form("/login", "username=anyone&password=anything".into(), None),
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    res.headers()
        .get(header::SET_COOKIE)
        .expect("the open window mints a cookie")
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned()
}

async fn admin_status(app: &axum::Router, cookie: &str) -> StatusCode {
    send(
        app,
        Request::builder()
            .uri("/admin/overview")
            .header(header::COOKIE, cookie)
            .body(Body::empty())
            .unwrap(),
    )
    .await
    .status()
}

#[tokio::test]
async fn the_accounts_page_first_password_signs_the_open_window_out() {
    let state = open_station();
    let admin_id = state
        .with_db(|c| c.find_user_by_name("admin"))
        .expect("seed admin")
        .id;
    let app = build_router(state);
    let stranger = stranger_signs_in(&app).await;

    // The owner, on the open bypass (no cookie), chooses a password.
    let res = send(
        &app,
        post_form(
            &format!("/admin/accounts/users/{admin_id}"),
            format!("password={PASSWORD}"),
            None,
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::OK, "setting the password failed");
    let owner = res
        .headers()
        .get(header::SET_COOKIE)
        .expect("the owner leaves signed in")
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();

    assert_eq!(
        admin_status(&app, &stranger).await,
        StatusCode::SEE_OTHER,
        "a session from the open window is still admin after the station was locked"
    );
    // Counterpart: the owner's own fresh session is the one that survives.
    assert_eq!(admin_status(&app, &owner).await, StatusCode::OK);
}

#[tokio::test]
async fn the_wizard_first_password_signs_the_open_window_out() {
    let app = build_router(open_station());
    let stranger = stranger_signs_in(&app).await;

    let res = send(
        &app,
        post_form(
            "/onboarding/save",
            format!("password={PASSWORD}&password_confirm={PASSWORD}"),
            None,
        ),
    )
    .await;
    assert_eq!(res.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        res.headers()
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok()),
        Some("/"),
        "the wizard refused the password"
    );

    assert_eq!(
        admin_status(&app, &stranger).await,
        StatusCode::SEE_OTHER,
        "a session from the open window is still admin after the wizard locked the station"
    );
}
