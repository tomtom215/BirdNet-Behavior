//! Integration tests for species HTMX partials and species list search, plus
//! admin cookie-auth and O-15 RBAC gating exercised through the real router.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rusqlite::{Connection, params};
use tower::ServiceExt;

use birdnet_db::accounts::{self, Role, SessionStore, UserStore};
use birdnet_web::server::build_router;
use birdnet_web::session;
use birdnet_web::state::AppState;

/// Create a test `AppState` with an in-memory database and sample data.
fn test_state() -> AppState {
    let conn = Connection::open_in_memory().unwrap();
    // Apply the full migration chain — hand-coded CREATE TABLE
    // declarations here drift the moment a migration adds a column.
    // The same anti-pattern (caught in PR #35 against
    // `open_or_create`, flagged again in ADR-16) means tests give
    // false greens until something downstream tries to read the
    // missing column. Defer to migrate() and the schema is always
    // current.
    birdnet_db::migration::migrate(&conn).unwrap();

    let records = [
        (
            "2026-03-12",
            "06:30:00",
            "Turdus merula",
            "Eurasian Blackbird",
            0.87,
        ),
        (
            "2026-03-12",
            "06:35:00",
            "Erithacus rubecula",
            "European Robin",
            0.92,
        ),
        (
            "2026-03-12",
            "06:45:00",
            "Turdus merula",
            "Eurasian Blackbird",
            0.78,
        ),
        ("2026-03-12", "07:00:00", "Parus major", "Great Tit", 0.81),
        (
            "2026-03-11",
            "18:00:00",
            "Cyanistes caeruleus",
            "Eurasian Blue Tit",
            0.75,
        ),
    ];

    for (date, time, sci, com, conf) in &records {
        conn.execute(
            "INSERT INTO detections (Date, Time, Sci_Name, Com_Name, Confidence)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![date, time, sci, com, conf],
        )
        .unwrap();
    }

    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

fn app() -> axum::Router {
    let state = test_state();
    build_router(state)
}

#[tokio::test]
async fn htmx_species_detections_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/species-detections?name=Eurasian%20Blackbird")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("<table>"));
    assert!(html.contains("Confidence"));
}

#[tokio::test]
async fn htmx_species_hourly_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/species-hourly?name=Eurasian%20Blackbird")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // Should render SVG chart since we have detections at hours 06 and 07
    assert!(html.contains("<svg"));
}

#[tokio::test]
async fn htmx_species_daily_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/species-daily?name=Eurasian%20Blackbird")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // Should render SVG chart since we have detection data
    assert!(html.contains("<svg"));
}

#[tokio::test]
async fn htmx_species_list_search() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/species-list?q=robin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("European Robin"));
    // Should NOT contain other species
    assert!(!html.contains("Eurasian Blackbird"));
    assert!(!html.contains("Great Tit"));
}

#[tokio::test]
async fn htmx_species_list_search_no_match() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/species-list?q=flamingo")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("No matching species found"));
}

// ---------------------------------------------------------------------------
// Admin cookie-auth + O-15 RBAC (drives the real `build_router` stack:
// CSRF guard → cookie middleware → role check).
// ---------------------------------------------------------------------------

/// Build a state whose seed `admin` row carries a *real* argon2 hash — so the
/// middleware's `admin_password_configured()` is true and it enforces the
/// cookie path instead of the "no password → open" bypass — plus a second
/// `viewer`-role account. Returns the state and a signed session cookie for
/// each role.
///
/// No env vars are set: `unsafe_code = "deny"` forbids `std::env::set_var`, so
/// the cookie is minted with the same per-process fallback secret the running
/// middleware resolves — `issue_token` here and `validate_token` there agree
/// within this test binary.
fn gated_state_with_cookies() -> (AppState, String, String) {
    let conn = Connection::open_in_memory().unwrap();
    birdnet_db::migration::migrate(&conn).unwrap();
    let state = AppState::from_connection(conn, std::path::PathBuf::from(":memory:"));

    let (admin_sid, viewer_sid) = state
        .with_db(
            |conn| -> Result<(String, String), accounts::AccountsError> {
                // Real hash on the seed admin → "admin password configured".
                let admin = conn.find_user_by_name("admin")?;
                conn.set_password(admin.id, &accounts::hash_password("admin-pw")?)?;
                // A viewer-role account.
                let viewer = conn.create_user(
                    "viewer",
                    &accounts::hash_password("viewer-pw")?,
                    Role::Viewer,
                    None,
                )?;
                // Far-future session rows the middleware looks up per request.
                let admin_sid = session::generate_session_id();
                let viewer_sid = session::generate_session_id();
                conn.create_session(&admin_sid, admin.id, "2999-01-01 00:00:00", None, None)?;
                conn.create_session(&viewer_sid, viewer.id, "2999-01-01 00:00:00", None, None)?;
                Ok((admin_sid, viewer_sid))
            },
        )
        .expect("seed admin hash + viewer account + bound sessions");

    let cookie = |sid: &str| {
        format!(
            "{}={}",
            session::COOKIE_NAME,
            session::issue_token(sid, session::DEFAULT_TTL_MS)
        )
    };
    (state, cookie(&admin_sid), cookie(&viewer_sid))
}

/// Send one request through a fresh clone of the router and return its status.
async fn rbac_status(
    router: &axum::Router,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
) -> StatusCode {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        builder = builder.header(axum::http::header::COOKIE, c);
    }
    router
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap()
        .status()
}

/// The dashboard stays open on the LAN, but `/admin` sits behind the cookie
/// middleware (the O-14/O-15 wire flip). Once signed in, a `viewer` keeps read
/// access to `/admin` while every write is gated to the `admin` role — the
/// O-15 RBAC contract, proven end to end rather than only at the unit level.
#[tokio::test]
async fn admin_cookie_gate_and_viewer_rbac() {
    let (state, admin_cookie, viewer_cookie) = gated_state_with_cookies();
    let router = build_router(state);

    // Public API stays viewable with no login.
    assert_eq!(
        rbac_status(&router, "GET", "/api/v2/health", None).await,
        StatusCode::OK,
        "the dashboard/API must stay viewable without a login"
    );

    // Unauthenticated: an admin read redirects to /login (303); an admin write
    // is rejected outright (401 — htmx will not follow a 303 on writes).
    assert_eq!(
        rbac_status(&router, "GET", "/admin/overview", None).await,
        StatusCode::SEE_OTHER,
        "an unauthenticated admin read must redirect to /login"
    );
    assert_eq!(
        rbac_status(&router, "POST", "/admin/rules", None).await,
        StatusCode::UNAUTHORIZED,
        "an unauthenticated admin write must be rejected"
    );

    // Viewer: reads are admitted; writes are denied (403, O-15 RBAC).
    let viewer_read = rbac_status(&router, "GET", "/admin/overview", Some(&viewer_cookie)).await;
    assert_ne!(
        viewer_read,
        StatusCode::UNAUTHORIZED,
        "a signed-in viewer must be admitted to admin reads"
    );
    assert_ne!(
        viewer_read,
        StatusCode::FORBIDDEN,
        "a viewer keeps read access to /admin"
    );
    assert_eq!(
        rbac_status(&router, "POST", "/admin/rules", Some(&viewer_cookie)).await,
        StatusCode::FORBIDDEN,
        "a viewer must be denied admin writes (403)"
    );

    // Admin: the identical write clears the role gate and reaches the handler
    // (which 4xx's on the empty form body — the point is it is never 401/403).
    let admin_write = rbac_status(&router, "POST", "/admin/rules", Some(&admin_cookie)).await;
    assert_ne!(
        admin_write,
        StatusCode::UNAUTHORIZED,
        "an admin session must authenticate through"
    );
    assert_ne!(
        admin_write,
        StatusCode::FORBIDDEN,
        "an admin must pass the RBAC write gate"
    );
}
