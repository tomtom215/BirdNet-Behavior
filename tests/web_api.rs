//! Integration tests for the web API.
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
    assert_eq!(json["database"], "ok");
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
}

#[tokio::test]
async fn detections_by_date() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections?date=2026-03-12")
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

    assert_eq!(json["total"], 4);
    let detections = json["detections"].as_array().unwrap();
    assert_eq!(detections.len(), 4);

    // Should be ordered by time DESC
    assert_eq!(detections[0]["time"], "07:00:00");
}

#[tokio::test]
async fn recent_detections_with_limit() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections/recent?limit=3")
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

    assert_eq!(json["total"], 3);
}

#[tokio::test]
async fn top_species() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/top?limit=10")
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

    let species = json["species"].as_array().unwrap();
    assert_eq!(species.len(), 4);

    // Blackbird has 2 detections, should be first
    assert_eq!(species[0]["com_name"], "Eurasian Blackbird");
    assert_eq!(species[0]["count"], 2);
}

#[tokio::test]
async fn hourly_activity() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/activity?date=2026-03-12")
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

    let activity = json["activity"].as_array().unwrap();
    assert_eq!(activity.len(), 2); // hours 06 and 07
    assert_eq!(activity[0]["hour"], "06");
    assert_eq!(activity[0]["count"], 3);
}

#[tokio::test]
async fn analytics_endpoints_report_unavailable_without_duckdb() {
    let app = app();

    // These endpoints don't require query params and report unavailable without DuckDB
    for endpoint in &["/api/v2/analytics/retention", "/api/v2/analytics/funnel"] {
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

    // Without analytics feature, should report not compiled
    assert_eq!(json["analytics_compiled"], false);
    assert_eq!(json["analytics_configured"], false);
    assert!(json["endpoints"].is_object());
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

#[tokio::test]
async fn export_detections_csv() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections/export")
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
    assert!(content_type.contains("text/csv"));

    let disposition = response
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(disposition.contains("detections.csv"));

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let csv = String::from_utf8_lossy(&body);

    // Header row
    assert!(csv.starts_with("Date,Time,Sci_Name,Com_Name,Confidence"));
    // 5 data rows + 1 header = 6 lines
    assert_eq!(csv.lines().count(), 6);
    assert!(csv.contains("Eurasian Blackbird"));
}

#[tokio::test]
async fn export_detections_csv_with_date_filter() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections/export?from=2026-03-12&to=2026-03-12")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let csv = String::from_utf8_lossy(&body);

    // 4 detections on 2026-03-12 + 1 header = 5 lines
    assert_eq!(csv.lines().count(), 5);
    // Should NOT contain the 2026-03-11 detection
    assert!(!csv.contains("2026-03-11"));
}

#[tokio::test]
async fn export_detections_json() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections/export?format=json")
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
    assert!(content_type.contains("application/json"));

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["total"], 5);
    assert!(json["detections"].is_array());
}

#[tokio::test]
async fn export_species_csv() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/export")
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
    assert!(content_type.contains("text/csv"));

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let csv = String::from_utf8_lossy(&body);

    assert!(csv.starts_with("Com_Name,Sci_Name,Count,Avg_Confidence"));
    // 4 unique species + 1 header = 5 lines
    assert_eq!(csv.lines().count(), 5);
}

#[tokio::test]
async fn export_species_json() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/export?format=json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert_eq!(json["total"], 4);
    assert!(json["species"].is_array());
}

#[tokio::test]
async fn daily_detections_endpoint() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections/daily?days=30")
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

    assert!(json["daily"].is_array());
    assert!(json["total"].as_u64().unwrap() > 0);
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
async fn species_image_info_without_cache() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/image/Turdus%20merula")
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

    // Without image cache configured, should report disabled
    assert_eq!(json["status"], "disabled");
}

#[tokio::test]
async fn species_detail_page_returns_html() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/species/detail?name=Eurasian%20Blackbird")
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

    assert!(html.contains("Eurasian Blackbird"));
    assert!(html.contains("Turdus merula"));
    assert!(html.contains("hx-get")); // HTMX partials
}

#[tokio::test]
async fn species_detail_page_without_name_shows_error() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/species/detail")
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

    assert!(html.contains("No species specified"));
}

#[tokio::test]
async fn htmx_species_summary_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/species-summary?name=Eurasian%20Blackbird")
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

    assert!(html.contains("stat-card"));
    assert!(html.contains("Detections"));
    assert!(html.contains("Avg Confidence"));
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
async fn htmx_species_info_partial() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/pages/species-info?name=Eurasian%20Blackbird")
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

    // Without image cache, should show the "no info available" message
    assert!(html.contains("No additional info available"));
    assert!(html.contains("--image-cache-dir"));
}
