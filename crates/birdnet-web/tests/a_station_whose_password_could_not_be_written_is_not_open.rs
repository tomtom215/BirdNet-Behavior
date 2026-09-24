//! A station whose configured password could not be written is not open.
//!
//! `CADDY_PWD` in `birdnet.conf` is hashed into the seed admin row at start.
//! When that write fails (a busy or full database), the row keeps its empty
//! hash, and `admin_password_configured` — env var or a real hash, nothing
//! else — read the station as one with no password: `/admin` open to anyone
//! on the network, and the sign-in form minting an admin session for any
//! password, on a station whose operator had set one. Only `?strict=1`
//! reported the failed write.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// A station whose admin row still has the empty seed hash.
fn station() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn admin(state: &AppState) -> StatusCode {
    build_router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/admin/settings")
                .header("host", "localhost")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
}

async fn sign_in(state: &AppState) -> axum::response::Response {
    build_router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/login")
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("username=admin&password=anything"))
                .unwrap(),
        )
        .await
        .unwrap()
}

#[tokio::test]
async fn a_failed_bootstrap_keeps_the_admin_panel_closed() {
    let st = station();
    st.set_admin_bootstrap_failed(true);
    assert_ne!(admin(&st).await, StatusCode::OK, "the admin panel was open");
    let res = sign_in(&st).await;
    assert!(
        res.headers().get("set-cookie").is_none(),
        "any password signed in: {:?}",
        res.headers()
    );
}

/// The counterpart: a station that genuinely has no password is still the
/// open station a fresh install is.
#[tokio::test]
async fn a_station_with_no_password_is_still_open() {
    let st = station();
    assert_eq!(admin(&st).await, StatusCode::OK);
}
