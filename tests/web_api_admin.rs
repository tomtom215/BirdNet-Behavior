//! Integration tests for species HTMX partials and species list search.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rusqlite::{Connection, params};
use tower::ServiceExt;

use birdnet_web::server::build_router;
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
