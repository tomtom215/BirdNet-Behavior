//! Integration tests for export endpoints.

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
async fn export_detections_invalid_date_returns_400() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/detections/export?from=bad-date")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
