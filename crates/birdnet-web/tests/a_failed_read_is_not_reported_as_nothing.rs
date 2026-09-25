//! A read that failed is not rendered as a read that found nothing.
//!
//! Each surface here defaulted a failed query to an empty list and rendered it
//! as fact: the alert-rule export downloaded a well-formed backup containing
//! no rules, the notification log said "No notifications yet", a prune that
//! failed said "Pruned 0", the RSS/iCal feeds served a valid empty feed with a
//! five-minute cache, and a detection's comments said "No comments yet". Each
//! is driven here against a station whose table has gone, with the working
//! station as the counterpart.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use birdnet_web::server::build_router;
use birdnet_web::state::AppState;
use tower::ServiceExt as _;

fn station(drop: &[&str]) -> axum::Router {
    let conn = rusqlite::Connection::open_in_memory().expect("open");
    birdnet_db::migration::migrate(&conn).expect("migrate");
    for table in drop {
        conn.execute_batch(&format!("DROP TABLE {table};"))
            .expect("drop");
    }
    build_router(AppState::from_connection(
        conn,
        std::path::PathBuf::from(":memory:"),
    ))
}

async fn call(app: &axum::Router, method: Method, uri: &str) -> (StatusCode, String) {
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("host", "localhost")
                .header("origin", "http://localhost")
                .header("hx-request", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = res.status();
    let body = axum::body::to_bytes(res.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn a_rule_export_that_could_not_read_the_rules_is_not_a_backup() {
    let (status, body) = call(
        &station(&["alert_rules"]),
        Method::GET,
        "/admin/rules/export",
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "a failed read downloaded as a backup: {body}"
    );
    let (status, body) = call(&station(&[]), Method::GET, "/admin/rules/export").await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.contains("\"rules\""), "{body}");
}

#[tokio::test]
async fn the_notification_log_does_not_say_empty_when_it_could_not_look() {
    let uri = "/admin/notifications/partial";
    let (_, body) = call(&station(&["notification_log"]), Method::GET, uri).await;
    assert!(!body.contains("No notifications yet"), "{body}");
    assert!(body.contains("couldn't load"), "{body}");
    let (_, body) = call(&station(&[]), Method::GET, uri).await;
    assert!(body.contains("No notifications yet"), "{body}");

    let uri = "/admin/notifications/prune";
    let (_, body) = call(&station(&["notification_log"]), Method::DELETE, uri).await;
    assert!(!body.contains("Pruned 0"), "{body}");
    assert!(body.contains("could not prune"), "{body}");
    let (_, body) = call(&station(&[]), Method::DELETE, uri).await;
    assert!(body.contains("Pruned 0"), "{body}");
}

#[tokio::test]
async fn a_feed_that_could_not_read_is_unavailable_not_empty() {
    for uri in ["/feeds/today.rss", "/feeds/rare.rss", "/feeds/rare.ics"] {
        let (status, body) = call(&station(&["detections"]), Method::GET, uri).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{uri}: {body}");
        let (status, _) = call(&station(&[]), Method::GET, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
    }
}

#[tokio::test]
async fn a_detections_comments_that_could_not_be_read_are_not_none() {
    let uri = "/pages/detection-comments?date=2026-05-01&time=06:10:00&sci_name=Turdus+merula";
    let (_, body) = call(&station(&["detection_comments"]), Method::GET, uri).await;
    assert!(!body.contains("No comments yet"), "{body}");
    assert!(body.contains("couldn't load"), "{body}");
    let (_, body) = call(&station(&[]), Method::GET, uri).await;
    assert!(body.contains("No comments yet"), "{body}");
}

/// A detection page whose read failed said "Detection not found", with a 200 —
/// to someone following a shared link, that reads as "it was deleted".
#[tokio::test]
async fn a_detection_that_could_not_be_read_is_not_reported_missing() {
    let uri = "/detections/detail?date=2026-05-01&time=06:10:00";
    let (status, body) = call(&station(&["detections"]), Method::GET, uri).await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(!body.contains("Detection not found"), "{body}");
    assert!(body.contains("couldn't load"), "{body}");
    // Counterpart: a detection that is genuinely not there is a 404 that says so.
    let (status, body) = call(&station(&[]), Method::GET, uri).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(body.contains("Detection not found"), "{body}");
}
