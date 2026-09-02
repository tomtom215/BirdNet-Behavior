//! Nothing reachable without a login may change anything.
//!
//! The station's contract, as the README and the hardening guide both state it,
//! is *"viewing the dashboard is open; only `/admin` needs a password"*. That
//! was a claim about `/admin` that nobody had checked against the rest of the
//! tree, and it was false: thirteen `POST` routes sat in the public router —
//! delete a detection, relabel it, set or clear a review verdict, approve or
//! reject or delete a quarantined record, lock and unlock clips, and write the
//! station's coordinates and notification policy. Anyone who could load the
//! dashboard on the LAN could do all of it, with a same-origin CSRF check as the
//! only obstacle, which stops a hostile *page* and not a hostile *person*.
//!
//! Five gates, because the fix has halves that regress independently — and
//! because the first two are both satisfied by a fixture where the middleware
//! never has to decide anything:
//!
//! 1. The public router must expose no non-safe method at all. This is
//!    structural and catches the whole class — a new `POST` added to
//!    `pages::router()` fails here whatever it does.
//! 2. The mutating routes must actually be *mounted* somewhere, or "gated"
//!    would be indistinguishable from "deleted", and the dashboard's buttons
//!    would 404 with every test still green.
//! 3. And the gate must actually *refuse*. Gates 1 and 2 both pass in a test
//!    fixture where no admin password is configured, because the middleware
//!    bypasses entirely in that case — which is the fresh-station contract and
//!    is correct, but means neither of them observes an authorisation decision.
//!    The third gate sets a real argon2 hash on the seed admin row so the
//!    bypass does not fire, and asserts the request is turned away.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

/// Every state-changing page path, with the method the dashboard uses.
///
/// Written out rather than derived: this list is the *specification* of what is
/// meant to be gated, and deriving it from the router under test would make the
/// gate agree with whatever the code happens to do.
const MUTATING_PAGE_ROUTES: &[&str] = &[
    "/pages/today-delete",
    "/pages/today-relabel",
    "/pages/today-lock",
    "/pages/today-unlock",
    "/pages/detection-review",
    "/pages/detection-review-clear",
    "/pages/detection-review-inline",
    "/pages/recordings-lock",
    "/pages/recordings-unlock",
    "/pages/recordings-delete",
    "/pages/quarantine-approve",
    "/pages/quarantine-reject",
    "/pages/quarantine-delete",
    "/onboarding/save",
    "/pages/search-bulk",
];

fn test_state() -> AppState {
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

async fn status_of(router: axum::Router, method: Method, path: &str) -> StatusCode {
    let req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/x-www-form-urlencoded")
        .body(Body::empty())
        .expect("build request");
    router.oneshot(req).await.expect("router responds").status()
}

#[tokio::test]
async fn the_public_router_exposes_no_way_to_change_anything() {
    let public = birdnet_web::routes::public_routes().with_state(test_state());

    for path in MUTATING_PAGE_ROUTES {
        let status = status_of(public.clone(), Method::POST, path).await;
        assert!(
            status == StatusCode::METHOD_NOT_ALLOWED || status == StatusCode::NOT_FOUND,
            "POST {path} is reachable in the *public* router (status {status}); anyone who \
             can load the dashboard could call it. It belongs in \
             `pages::mutating_router()`, which is mounted behind the auth middleware."
        );
    }
}

#[tokio::test]
async fn the_mutating_routes_are_still_mounted_somewhere() {
    // The counterpart to the gate above, and it is not optional: deleting the
    // routes outright would satisfy that one perfectly while breaking every
    // button on the dashboard. `build_router` is the real composition the
    // binary serves.
    let app = birdnet_web::server::build_router(test_state());

    for path in MUTATING_PAGE_ROUTES {
        let status = status_of(app.clone(), Method::POST, path).await;
        assert_ne!(
            status,
            StatusCode::NOT_FOUND,
            "POST {path} is not mounted at all in the assembled router — gating it \
             has turned into losing it"
        );
        assert_ne!(
            status,
            StatusCode::METHOD_NOT_ALLOWED,
            "POST {path} exists but not for POST in the assembled router"
        );
    }
}

#[tokio::test]
async fn reading_the_dashboard_still_needs_no_login() {
    // The other half of the contract, which the fix must not have broken: the
    // pages themselves stay open. A regression here is somebody over-correcting
    // by gating the whole of `pages::router()`.
    let public = birdnet_web::routes::public_routes().with_state(test_state());

    for path in [
        "/pages/today-list",
        "/pages/quarantine-list",
        "/pages/recordings-clips",
        "/pages/detection-reviews-queue",
        "/onboarding",
    ] {
        let status = status_of(public.clone(), Method::GET, path).await;
        assert!(
            status.is_success() || status.is_redirection(),
            "GET {path} answered {status}; reading the dashboard must stay open"
        );
    }
}

#[tokio::test]
async fn with_a_password_set_an_unauthenticated_write_is_refused() {
    // The claim the whole change exists to make true. Both gates above are
    // satisfied by a fixture with no admin password, where the middleware
    // bypasses by design — so without this one, "gated" would be untested and
    // the routes could be mounted wide open behind a middleware that never
    // says no.
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    let hash = birdnet_db::accounts::hash_password("a-real-password").expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        [&hash],
    )
    .expect("give the seed admin a real password");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    let app = birdnet_web::server::build_router(state);

    for path in MUTATING_PAGE_ROUTES {
        let status = status_of(app.clone(), Method::POST, path).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "POST {path} was accepted without a session on a station that has an \
             admin password set. `redirect_to_login` answers unsafe methods with \
             401 + HX-Redirect, so anything else means the request reached the \
             handler."
        );
    }
}

#[tokio::test]
async fn a_password_does_not_close_the_dashboard() {
    // The counterpart: setting a password must not turn the *viewable* station
    // into a login wall, which is the over-correction this contract exists to
    // rule out in the other direction.
    let conn = rusqlite::Connection::open_in_memory().expect("open in-memory");
    birdnet_db::migration::migrate(&conn).expect("migrate schema");
    let hash = birdnet_db::accounts::hash_password("a-real-password").expect("hash");
    conn.execute(
        "UPDATE users SET pwd_argon2 = ?1 WHERE username = 'admin'",
        [&hash],
    )
    .expect("give the seed admin a real password");
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));
    let app = birdnet_web::server::build_router(state);

    for path in ["/pages/today-list", "/quarantine", "/recordings"] {
        let status = status_of(app.clone(), Method::GET, path).await;
        assert_ne!(
            status,
            StatusCode::UNAUTHORIZED,
            "GET {path} needed a login; reading stays open even with a password set"
        );
        assert!(
            status.is_success() || status.is_redirection(),
            "GET {path} answered {status}"
        );
    }
}
