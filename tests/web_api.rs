//! Integration tests for the web API — shared setup and basic API tests.
//!
//! Tests the full HTTP API including database interactions,
//! using an in-memory `SQLite` database and actual axum handlers.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rusqlite::{Connection, params};
use tower::ServiceExt;

use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

/// Create a test `AppState` with an in-memory database and sample data.
fn test_state() -> AppState {
    let conn = Connection::open_in_memory().unwrap();
    // Apply the full migration chain — hand-coded CREATE TABLE in
    // test fixtures drifts the moment a migration adds a column.
    // See ADR-16 "Anti-patterns this standard exists to prevent /
    // Hand-coded schema in test fixtures duplicating the migration".
    birdnet_db::migration::migrate(&conn).unwrap();

    // Insert sample detection data
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
async fn root_returns_api_info() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["name"], "BirdNet-Behavior API");
    assert_eq!(json["status"], "running");
}

#[tokio::test]
async fn health_endpoint_returns_healthy() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["status"], "healthy");
    // A station whose first daily integrity check has not run yet reports
    // `unchecked`, not `ok` and not `error`. Since migration 28 this endpoint
    // reads the recorded verdict rather than running its own `PRAGMA
    // quick_check` on every request — that pragma reads every page of the
    // database file, and the container HEALTHCHECK polls here every 30 s with a
    // 4 s curl timeout, which a three-year station could not meet.
    //
    // `unchecked` must keep returning 200: reporting it as degraded would leave
    // a freshly started container `unhealthy` for the five minutes before the
    // first maintenance tick.
    assert_eq!(json["database"], "unchecked");
}

/// A monitor must be able to get a **red** out of this endpoint for a station
/// that is not detecting.
///
/// The status code used to be `db_ok` and nothing else, so `/api/v2/health`
/// answered `200 "healthy"` on a station whose own response body said
/// `"detection_daemon": "stopped"` — verified against the running binary. That
/// is the endpoint the container `HEALTHCHECK` polls and the one every
/// off-the-shelf monitor gets pointed at, so a station that had recorded
/// nothing since March looked green to all of them.
///
/// Observed failing before `?strict`: the strict request returned 200.
#[tokio::test]
async fn strict_health_is_degraded_when_the_detection_daemon_is_not_running() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/health?strict=1")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        response.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "a station with no detection daemon must be reportable as down"
    );
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "degraded");
    assert_eq!(json["detection_daemon"], "stopped");
    assert_eq!(
        json["strict"], true,
        "the response must say which mode it answered in"
    );
}

/// The discrimination, and the reason the default did not simply change.
///
/// Docker restarts an unhealthy container, and a station whose daemon is down is
/// exactly the station that must stay up to be diagnosed — restarting it in a
/// loop destroys the journal that says why. So the same station, asked without
/// the flag, must still answer 200. A change that made every caller strict would
/// pass the test above and put field stations into a restart loop.
#[tokio::test]
async fn the_default_health_probe_stays_green_so_the_container_is_not_restarted() {
    for uri in [
        "/api/v2/health",
        "/api/v2/health?strict=0",
        "/api/v2/health?strict=false",
    ] {
        let response = app()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{uri} must stay 200 on a station whose daemon is not running"
        );
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["status"], "healthy", "{uri}");
        assert_eq!(
            json["detection_daemon"], "stopped",
            "{uri}: the body must still say so — the fact was always there, \
             only the status code was not"
        );
    }
}

/// …and once the daily check has recorded a pass, it says so.
///
/// The counterpart to the assertion above, so "unchecked" cannot quietly become
/// the only answer this endpoint ever gives.
#[tokio::test]
async fn health_endpoint_reports_a_recorded_pass() {
    let state = test_state();
    state.with_db(|conn| {
        birdnet_db::sqlite::record_run_result(
            conn,
            birdnet_db::sqlite::JOB_INTEGRITY_CHECK,
            1_700_000_000,
            Some(true),
        )
        .unwrap();
    });

    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/v2/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["database"], "ok");
    assert_eq!(json["status"], "healthy");
}

/// A recorded failure must reach the endpoint as a 503, on a database that is
/// itself intact — which is the only thing that distinguishes reading the
/// record from re-running the check.
#[tokio::test]
async fn health_endpoint_reports_a_recorded_failure_as_degraded() {
    let state = test_state();
    state.with_db(|conn| {
        birdnet_db::sqlite::record_run_result(
            conn,
            birdnet_db::sqlite::JOB_INTEGRITY_CHECK,
            1_700_000_000,
            Some(false),
        )
        .unwrap();
    });

    let response = build_router(state)
        .oneshot(
            Request::builder()
                .uri("/api/v2/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["database"], "error");
    assert_eq!(json["status"], "degraded");
}

#[tokio::test]
async fn stats_endpoint_returns_counts() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["total_detections"], 5);
    assert_eq!(json["unique_species"], 4);
    assert!(json["latest_detection"].is_object());
    assert_eq!(json["latest_detection"]["species"], "Great Tit");
    assert!(json["confidence_distribution"].is_object());
}

#[tokio::test]
async fn disk_info_endpoint() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/system/disk")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 200 (or 503 if disk is critical, unlikely in test)
    assert!(response.status().is_success() || response.status() == StatusCode::SERVICE_UNAVAILABLE);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    // Should have disk usage fields
    assert!(json["total_bytes"].is_number() || json["error"].is_string());
}

#[tokio::test]
async fn static_htmx_js_served() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/static/htmx.min.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(content_type, "application/javascript");

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    assert!(body.len() > 1000); // HTMX is ~50KB
}
