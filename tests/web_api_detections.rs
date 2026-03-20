//! Integration tests for detection endpoints.

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

    assert_eq!(json["count"], 4);
    let detections = json["detections"].as_array().unwrap();
    assert_eq!(detections.len(), 4);

    // Should be ordered by time DESC
    assert_eq!(detections[0]["time"], "07:00:00");
}

#[tokio::test]
async fn detections_by_species_filter() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections?species=Eurasian%20Blackbird")
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

    assert_eq!(json["count"], 2);
    let detections = json["detections"].as_array().unwrap();
    assert!(
        detections
            .iter()
            .all(|d| d["com_name"] == "Eurasian Blackbird")
    );
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
async fn detections_pagination() {
    let app = app();

    // First page: limit 2, offset 0
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections?limit=2&offset=0")
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

    assert_eq!(json["count"], 2);
    assert_eq!(json["limit"], 2);
    assert_eq!(json["offset"], 0);

    let detections = json["detections"].as_array().unwrap();
    assert_eq!(detections.len(), 2);
}

#[tokio::test]
async fn detections_pagination_second_page() {
    let app = app();

    // Second page: limit 2, offset 2
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections?limit=2&offset=2")
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

    assert_eq!(json["count"], 2);
    assert_eq!(json["offset"], 2);
}

#[tokio::test]
async fn detections_pagination_beyond_data() {
    let app = app();

    // Beyond available data
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections?limit=10&offset=100")
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

    assert_eq!(json["count"], 0);
}

#[tokio::test]
async fn detections_invalid_date_returns_400() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections?date=not-a-date")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();

    assert!(json["error"].as_str().unwrap().contains("invalid date"));
}

#[tokio::test]
async fn detections_pagination_includes_total_count() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections?limit=2&offset=0")
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

    // Paginated responses should include total_count
    assert_eq!(json["total_count"], 5); // 5 total records in test data
    assert_eq!(json["count"], 2);
    assert_eq!(json["limit"], 2);
    assert_eq!(json["offset"], 0);
}

#[tokio::test]
async fn detections_limit_is_capped() {
    let app = app();

    // Request with very large limit — should be capped to MAX_LIMIT (1000)
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections?limit=999999")
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

    // Limit should be capped at 1000
    assert_eq!(json["limit"], 1000);
}
