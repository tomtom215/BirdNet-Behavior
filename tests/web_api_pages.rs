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
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS detections (
            Date TEXT NOT NULL,
            Time TEXT NOT NULL,
            Sci_Name TEXT NOT NULL,
            Com_Name TEXT NOT NULL,
            Confidence REAL NOT NULL,
            Lat REAL,
            Lon REAL,
            Cutoff REAL,
            Week INTEGER,
            Sens REAL,
            Overlap REAL,
            File_Name TEXT
        );",
    )
    .unwrap();

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
    assert!(html.contains("Recent Detections"));
    assert!(html.contains("Top Species"));
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

    assert!(html.contains("Total Detections"));
    assert!(html.contains("Unique Species"));
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

    assert!(html.contains("<table>"));
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
    assert!(html.contains("species-item"));
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
