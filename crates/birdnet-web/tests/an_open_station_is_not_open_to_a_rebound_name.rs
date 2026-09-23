//! A station with no password is open to its network — not to every website.
//!
//! See `birdnet_web::open_admin_host` for the attack. The gate drives the real
//! router with the `Host` a DNS-rebinding page produces, on both doors that
//! grant the open bypass: the admin middleware itself, and `POST /login`, which
//! on an open station mints an admin cookie for any credentials.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn open_station() -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
}

async fn admin(app: &axum::Router, host: &str) -> StatusCode {
    app.clone()
        .oneshot(
            Request::builder()
                .uri("/admin/overview")
                .header(header::HOST, host)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn the_admin_panel_refuses_a_rebound_name() {
    let app = open_station();
    assert_eq!(
        admin(&app, "rebind.evil.example:8502").await,
        StatusCode::FORBIDDEN,
        "an open station served its admin panel to a name any website can rebind"
    );
    // Counterpart: the names the owner actually uses still work.
    for host in [
        "192.168.1.20:8502",
        "birdnet.local:8502",
        "localhost:8502",
        "birdnet",
    ] {
        assert_eq!(admin(&app, host).await, StatusCode::OK, "{host}");
    }
}

#[tokio::test]
async fn a_rebound_name_cannot_mint_an_open_station_cookie() {
    let app = open_station();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/login")
                .header("content-type", "application/x-www-form-urlencoded")
                .header(header::HOST, "rebind.evil.example:8502")
                .header("origin", "http://rebind.evil.example:8502")
                .body(Body::from("username=x&password=y"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    assert!(
        res.headers().get(header::SET_COOKIE).is_none(),
        "a rebound name was handed an admin session"
    );
}
