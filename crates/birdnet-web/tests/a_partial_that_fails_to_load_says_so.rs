//! `UX-4` / `ui-1`: a page region that fails to load must say so.
//!
//! htmx leaves the target as it was on a 4xx/5xx and on a transport error, and
//! the shell registered no handler for either event, so a partial that failed
//! — or a station that died under an open dashboard — left its skeleton or
//! "Loading…" in place indefinitely, with nothing in the toast region and no
//! error text anywhere. Observed in a browser on both the seeded and a fresh
//! station: `htmx:sendError /pages/detections` fired, the page did not change.
//!
//! The shell now handles `htmx:responseError`, `htmx:sendError` and
//! `htmx:timeout` for GET loads by replacing the waiting region with a short
//! notice that names the failure and offers a reload; POSTs are left alone so
//! a form that failed can be retried. This gate holds the wiring in the served
//! shell; the behaviour was validated in Chromium (see the commit message).
//!
//! Observed failing against the shipped tree: the body of `GET /species` contained
//! none of the three event names.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt as _;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

#[tokio::test]
async fn the_shell_turns_a_failed_load_into_a_visible_notice() {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    let resp = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/species")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);
    for event in ["htmx:responseError", "htmx:sendError", "htmx:timeout"] {
        assert!(
            html.contains(&format!("'{event}'")),
            "the shell does not handle {event}; a failed partial stays a skeleton"
        );
    }
    assert!(
        html.contains("bnb-load-error") && html.contains("could not load"),
        "the shell has no notice to show for a failed region"
    );
    // The handler must not eat a failed POST: the form is the operator's way to retry.
    assert!(
        html.contains("cfg.verb === 'get'"),
        "the handler must act on GET loads only"
    );
}
