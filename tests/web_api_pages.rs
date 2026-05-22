//! Integration tests for page rendering (HTML pages and HTMX partials).

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
async fn dashboard_page_returns_html() {
    let app = app();

    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("BirdNet-Behavior"));
    assert!(html.contains("htmx.min.js"));
    assert!(html.contains("/static/css/app.css"));
    assert!(html.contains("Detections as they happen"));
    assert!(html.contains("Top species"));
}

#[tokio::test]
async fn species_page_returns_html() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/species")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("All Species"));
    assert!(html.contains("hx-get"));
}

#[tokio::test]
async fn htmx_stats_partial_returns_html() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/stats")
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

    assert!(html.contains("Detections"));
    assert!(html.contains("Species"));
    assert!(html.contains("stat-tile"));
    assert!(html.contains('5')); // total detections from test data
    assert!(html.contains('4')); // unique species from test data
}

#[tokio::test]
async fn htmx_detections_partial_returns_table() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/detections")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("feed-row"));
    assert!(html.contains("Eurasian Blackbird"));
    assert!(html.contains("European Robin"));
}

#[tokio::test]
async fn htmx_top_species_partial_returns_list() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/top-species")
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

    assert!(html.contains("Eurasian Blackbird"));
    assert!(html.contains("list-row"));
    assert!(html.contains("bnb-avatar"));
}

#[tokio::test]
async fn htmx_health_badge_returns_healthy() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/health-badge")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("Healthy"));
    assert!(html.contains("ok"));
}

#[tokio::test]
async fn analytics_page_returns_html() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/analytics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("Behavioral Analytics"));
    assert!(html.contains("Activity Sessions"));
    assert!(html.contains("Species Retention"));
}

#[tokio::test]
async fn htmx_hourly_chart_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/hourly-chart")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 16384)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // Should return either SVG chart or "no detections" message
    assert!(html.contains("<svg") || html.contains("No detections"));
}

#[tokio::test]
async fn htmx_daily_chart_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/daily-chart")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 16384)
        .await
        .unwrap();
    let html = String::from_utf8_lossy(&body);

    // Should return either SVG chart or "no data" message
    assert!(html.contains("<svg") || html.contains("No detection data"));
}

#[tokio::test]
async fn htmx_analytics_status_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/analytics-status")
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

    assert!(html.contains("Analytics Engine"));
}

#[tokio::test]
async fn htmx_analytics_config_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/analytics-config")
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

    assert!(html.contains("Version"));
    assert!(html.contains("SQLite Database"));
}

#[tokio::test]
async fn htmx_confidence_chart_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/confidence-chart")
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

    // Should contain SVG chart (test data has detections with various confidence levels)
    assert!(html.contains("<svg"));
}
