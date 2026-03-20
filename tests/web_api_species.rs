//! Integration tests for species endpoints and species detail pages.

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
async fn species_detail_api() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/detail?name=Eurasian%20Blackbird")
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

    assert_eq!(json["species"]["com_name"], "Eurasian Blackbird");
    assert_eq!(json["species"]["sci_name"], "Turdus merula");
    assert_eq!(json["species"]["count"], 2);
    assert!(json["hourly_activity"].is_array());
}

#[tokio::test]
async fn species_detail_api_not_found() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/detail?name=Nonexistent%20Bird")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn species_search_api() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/search?q=blackbird")
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

    assert_eq!(json["total"], 1);
    let species = json["species"].as_array().unwrap();
    assert_eq!(species[0]["com_name"], "Eurasian Blackbird");
}

#[tokio::test]
async fn species_search_api_no_results() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/search?q=nonexistent")
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

    assert_eq!(json["total"], 0);
}

#[tokio::test]
async fn species_search_by_scientific_name() {
    let app = app();

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v2/species/search?q=turdus")
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

    assert_eq!(json["total"], 1);
    let species = json["species"].as_array().unwrap();
    assert_eq!(species[0]["sci_name"], "Turdus merula");
}

// --- Species detail pages and HTMX partials ---

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
    assert!(html.contains("No additional info for"), "got: {html}");
    assert!(html.contains("--image-cache-dir"));
}
