//! Integration tests for the quarantine review page and HTMX partials.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rusqlite::Connection;
use tower::ServiceExt;

use birdnet_db::migration::migrate;
use birdnet_db::sqlite::{QuarantineReason, QuarantineRecord, insert_quarantine};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create a migrated in-memory DB with the full schema (incl. quarantine v10).
fn migrated_state() -> AppState {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    migrate(&conn).unwrap();
    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

/// Seed a quarantine row and return the [`AppState`].
fn state_with_quarantine(reason: QuarantineReason) -> AppState {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA journal_mode=WAL;").unwrap();
    migrate(&conn).unwrap();

    let record = QuarantineRecord {
        date: "2026-03-27",
        time: "07:15:30",
        sci_name: "Upupa epops",
        com_name: "Eurasian Hoopoe",
        confidence: 0.42,
        sf_probability: Some(0.61),
        reason,
        file_name: Some("BirdSongs/2026-03-27_07-15-30.wav"),
        lat: Some(51.5),
        lon: Some(-0.12),
        week: Some(13),
    };
    insert_quarantine(&conn, &record).unwrap();

    AppState::from_connection(conn, std::path::PathBuf::from(":memory:"))
}

fn app(state: AppState) -> axum::Router {
    build_router(state)
}

// ---------------------------------------------------------------------------
// Page rendering
// ---------------------------------------------------------------------------

#[tokio::test]
async fn quarantine_page_returns_html() {
    let router = app(migrated_state());

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/quarantine")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("<!DOCTYPE html>"));
    assert!(html.contains("Quarantine"));
    assert!(html.contains("quarantine-stats"));
    assert!(html.contains("quarantine-list"));
}

#[tokio::test]
async fn quarantine_page_nav_link_present() {
    let router = app(migrated_state());

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/quarantine")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body = axum::body::to_bytes(resp.into_body(), 65536).await.unwrap();
    let html = String::from_utf8_lossy(&body);

    // The layout nav should include the quarantine link.
    assert!(html.contains("href=\"/quarantine\""));
}

// ---------------------------------------------------------------------------
// Stats partial
// ---------------------------------------------------------------------------

#[tokio::test]
async fn quarantine_stats_empty_db() {
    let router = app(migrated_state());

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/pages/quarantine-stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("Pending Review"));
    assert!(html.contains('0'));
}

#[tokio::test]
async fn quarantine_stats_with_pending() {
    let router = app(state_with_quarantine(QuarantineReason::LowConfidence));

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/pages/quarantine-stats")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains('1'));
}

// ---------------------------------------------------------------------------
// List partial
// ---------------------------------------------------------------------------

#[tokio::test]
async fn quarantine_list_empty() {
    let router = app(migrated_state());

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/pages/quarantine-list")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 4096).await.unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("No entries found"));
}

#[tokio::test]
async fn quarantine_list_shows_pending_entries() {
    let router = app(state_with_quarantine(QuarantineReason::LowConfidence));

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/pages/quarantine-list?filter=pending")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
    let html = String::from_utf8_lossy(&body);

    assert!(html.contains("Eurasian Hoopoe"));
    assert!(html.contains("42%"));
    assert!(html.contains("Below species threshold"));
    assert!(html.contains("Approve"));
    assert!(html.contains("Reject"));
}

#[tokio::test]
async fn quarantine_list_filter_all_shows_entry() {
    let router = app(state_with_quarantine(QuarantineReason::BelowSfThresh));

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/pages/quarantine-list?filter=all")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 16384).await.unwrap();
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("Eurasian Hoopoe"));
}

// ---------------------------------------------------------------------------
// Pending count badge
// ---------------------------------------------------------------------------

#[tokio::test]
async fn quarantine_pending_count_zero() {
    let router = app(migrated_state());

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/pages/quarantine-pending-count")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    // Zero pending → empty body (no badge rendered)
    let html = String::from_utf8_lossy(&body);
    assert!(html.is_empty() || html == "0" || !html.contains("span"));
}

#[tokio::test]
async fn quarantine_pending_count_nonzero() {
    let router = app(state_with_quarantine(QuarantineReason::Manual));

    let resp = router
        .oneshot(
            Request::builder()
                .uri("/pages/quarantine-pending-count")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let body = axum::body::to_bytes(resp.into_body(), 1024).await.unwrap();
    let html = String::from_utf8_lossy(&body);
    // Should render a badge with count "1"
    assert!(html.contains('1'));
    assert!(html.contains("span"));
}

// ---------------------------------------------------------------------------
// Approve action
// ---------------------------------------------------------------------------

#[tokio::test]
async fn approve_action_returns_ok_and_triggers_reload() {
    let state = state_with_quarantine(QuarantineReason::LowConfidence);
    let router = app(state.clone());

    // Get the quarantine ID from the DB.
    let id: i64 = state
        .with_db(|conn| {
            conn.query_row("SELECT id FROM quarantine LIMIT 1", [], |r| r.get(0))
                .map_err(birdnet_db::sqlite::DbError::Sqlite)
        })
        .unwrap();

    let body_str = format!("id={id}&filter=pending");
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/pages/quarantine-approve")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from(body_str))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    // The detection should now be in the detections table.
    let det_count: i64 = state
        .with_db(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM detections WHERE Sci_Name = 'Upupa epops'",
                [],
                |r| r.get(0),
            )
            .map_err(birdnet_db::sqlite::DbError::Sqlite)
        })
        .unwrap();
    assert_eq!(det_count, 1, "approved detection should be in detections");

    // The quarantine row should be marked approved.
    let approved: i64 = state
        .with_db(|conn| {
            conn.query_row(
                "SELECT approved FROM quarantine WHERE id = ?1",
                rusqlite::params![id],
                |r| r.get(0),
            )
            .map_err(birdnet_db::sqlite::DbError::Sqlite)
        })
        .unwrap();
    assert_eq!(approved, 1);
}

// ---------------------------------------------------------------------------
// Reject action
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reject_action_marks_reviewed() {
    let state = state_with_quarantine(QuarantineReason::LowConfidence);
    let router = app(state.clone());

    let id: i64 = state
        .with_db(|conn| {
            conn.query_row("SELECT id FROM quarantine LIMIT 1", [], |r| r.get(0))
                .map_err(birdnet_db::sqlite::DbError::Sqlite)
        })
        .unwrap();

    let body_str = format!("id={id}&filter=pending");
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/pages/quarantine-reject")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from(body_str))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let (reviewed, approved): (i64, i64) = state
        .with_db(|conn| {
            conn.query_row(
                "SELECT reviewed, approved FROM quarantine WHERE id = ?1",
                rusqlite::params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(birdnet_db::sqlite::DbError::Sqlite)
        })
        .unwrap();

    assert_eq!(reviewed, 1, "should be marked reviewed");
    assert_eq!(approved, 0, "should NOT be marked approved");
}

// ---------------------------------------------------------------------------
// Delete action
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_action_removes_row() {
    let state = state_with_quarantine(QuarantineReason::Manual);
    let router = app(state.clone());

    let id: i64 = state
        .with_db(|conn| {
            conn.query_row("SELECT id FROM quarantine LIMIT 1", [], |r| r.get(0))
                .map_err(birdnet_db::sqlite::DbError::Sqlite)
        })
        .unwrap();

    let body_str = format!("id={id}&filter=all");
    let resp = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/pages/quarantine-delete")
                .header("Content-Type", "application/x-www-form-urlencoded")
                .body(Body::from(body_str))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(resp.status(), StatusCode::OK);

    let count: i64 = state
        .with_db(|conn| {
            conn.query_row("SELECT COUNT(*) FROM quarantine", [], |r| r.get(0))
                .map_err(birdnet_db::sqlite::DbError::Sqlite)
        })
        .unwrap();
    assert_eq!(count, 0, "quarantine row should be deleted");
}
