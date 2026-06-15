//! Integration tests for analytics endpoints.

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
async fn analytics_endpoints_report_unavailable_without_duckdb() {
    let app = app();

    // These endpoints don't require query params and report unavailable without DuckDB
    for endpoint in &[
        "/api/v2/analytics/retention",
        "/api/v2/analytics/funnel",
        "/api/v2/analytics/funnel-events",
        "/api/v2/analytics/sequence-count",
        "/api/v2/analytics/sequence-match-events",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(*endpoint)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK, "failed: {endpoint}");

        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(
            json["status"], "unavailable",
            "endpoint {endpoint} should report unavailable without DuckDB"
        );
    }

    // Sessions endpoint with optional params
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v2/analytics/sessions")
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
    assert_eq!(json["status"], "unavailable");

    // next-species endpoint with optional params (returns unavailable without DuckDB)
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v2/analytics/next-species?after=Robin")
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
    assert_eq!(json["status"], "unavailable");
}

#[tokio::test]
async fn analytics_status_endpoint() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/analytics/status")
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

    // analytics_compiled reflects whether the feature flag is active
    #[cfg(feature = "analytics")]
    assert_eq!(json["analytics_compiled"], true);
    #[cfg(not(feature = "analytics"))]
    assert_eq!(json["analytics_compiled"], false);
    // Without a DuckDB path configured, analytics is not configured
    assert_eq!(json["analytics_configured"], false);
    assert!(json["endpoints"].is_object());
}
